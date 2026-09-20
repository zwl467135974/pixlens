//! 批量处理管线（doc/01 F4、doc/03 §3.4）
//!
//! - 重命名：{原名} {序号} {序号:n} {日期} 模板（日期取文件 mtime），
//!   预览与执行共用同一展开函数（保证一致）；冲突检测（批内重名 / 与批外磁盘现存同名）；
//!   两阶段执行（先全部改临时名再落最终名，规避 a→b、b→a 交叉）
//! - 转换/压缩/缩放：rayon 并行，进度经 batch-progress 事件推送，
//!   AtomicBool 检查点取消；默认输出 pixlens_output/ 子目录，可选覆盖
//! - 解码在内存完成后才写盘；新文件冲突自动追加 " (n)"

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageEncoder};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

/// 可参与转换的输入扩展名（doc/01 F3：AVIF/SVG/PSD 不参与转换输出）
const CONVERTIBLE: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "ico", "tif", "tiff"];

fn jobs() -> &'static parking_lot::Mutex<HashMap<u64, Arc<AtomicBool>>> {
    static JOBS: OnceLock<parking_lot::Mutex<HashMap<u64, Arc<AtomicBool>>>> = OnceLock::new();
    JOBS.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

// ── 进度 ─────────────────────────────
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BatchProgress {
    pub job_id: u64,
    pub done: usize,
    pub failed: usize,
    pub total: usize,
    pub current: String,
    pub canceled: bool,
    pub finished: bool,
}

fn emit_progress(app: &tauri::AppHandle, p: BatchProgress) {
    let _ = app.emit("batch-progress", p);
}

// ── 重命名 ─────────────────────────────
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RenamePair {
    pub path: String,
    pub name: String,
    pub new_name: String,
    pub conflict: bool,
    pub reason: String,
}

/// 模板展开：{原名} {序号} {序号:n} {日期}(=mtime yyyy-MM-dd)；未知 token 原样保留
pub fn expand_template(template: &str, base: &str, index: u32, date: &str) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        let Some(rel) = after.find('}') else {
            out.push_str(after);
            return out;
        };
        let token = &after[1..rel];
        rest = &after[rel + 1..];
        if token == "原名" {
            out.push_str(base);
        } else if token == "序号" {
            out.push_str(&index.to_string());
        } else if let Some(n) = token.strip_prefix("序号:") {
            let width = n.parse::<usize>().unwrap_or(0);
            out.push_str(&format!("{index:0width$}"));
        } else if token == "日期" {
            out.push_str(date);
        } else {
            out.push('{');
            out.push_str(token);
            out.push('}');
        }
    }
    out.push_str(rest);
    out
}

/// mtime → "yyyy-MM-dd"（无 chrono 依赖，Howard Hinnant civil 算法）
fn unix_ms_to_date(ms: u64) -> String {
    let days = (ms / 86_400_000) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn pair_key(dir: Option<&Path>, name: &str) -> String {
    format!("{}\u{0}{}", dir.and_then(|d| d.to_str()).unwrap_or(""), name)
}

/// 预览：纯计算不改盘。冲突 = 批内新名重复，或目标名与"批外磁盘现存文件"同名
/// （批内让位由两阶段执行解决，不算冲突）。
#[tauri::command]
pub fn batch_rename_preview(paths: Vec<String>, template: String, start: u32) -> Result<Vec<RenamePair>, String> {
    if template.trim().is_empty() {
        return Err("模板为空".into());
    }
    let mut pairs: Vec<RenamePair> = Vec::with_capacity(paths.len());
    let batch_sources: HashSet<String> = paths.iter().cloned().collect();

    for (i, p) in paths.iter().enumerate() {
        let path = PathBuf::from(p);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let date = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| unix_ms_to_date(d.as_millis() as u64))
            .unwrap_or_else(|| "1970-01-01".into());
        let new_base = expand_template(&template, &stem, start + i as u32, &date);
        pairs.push(RenamePair {
            path: p.clone(),
            name,
            new_name: format!("{new_base}{ext}"),
            conflict: false,
            reason: String::new(),
        });
    }

    // 批内重复
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut dups: Vec<(usize, usize)> = Vec::new(); // (后到者, 先到者)
    for i in 0..pairs.len() {
        let dir = Path::new(&pairs[i].path).parent().map(|d| d.to_path_buf());
        let key = pair_key(dir.as_deref(), &pairs[i].new_name);
        match seen.get(&key) {
            Some(&first) => dups.push((i, first)),
            None => {
                seen.insert(key, i);
            }
        }
    }
    for (i, first) in dups {
        pairs[i].conflict = true;
        pairs[i].reason = "批内新名重复".into();
        if !pairs[first].conflict {
            pairs[first].conflict = true;
            pairs[first].reason = "批内新名重复".into();
        }
    }

    // 与批外磁盘现存同名（同目录）
    for p in pairs.iter_mut() {
        if p.conflict {
            continue;
        }
        let path = Path::new(&p.path);
        let Some(dir) = path.parent() else { continue };
        let target = dir.join(&p.new_name);
        let target_str = target.to_string_lossy().into_owned();
        if target.exists() && !batch_sources.contains(&target_str) {
            p.conflict = true;
            p.reason = "目标名已存在".into();
        }
    }
    Ok(pairs)
}

/// 两阶段执行：全部先改临时名（同目录 `.__pixlens_tmp_{i}_` 前缀）再落最终名。
/// 注意：Windows 的 fs::rename 会静默覆盖已存在目标，落名前必须显式检查
/// （此时批内源文件已全部挪入临时名，残留目标必为批外文件 → 拒绝并回滚）。
#[tauri::command]
pub fn batch_rename_apply(pairs: Vec<(String, String)>) -> Result<(usize, Vec<String>), String> {
    let mut errors: Vec<String> = Vec::new();
    let mut staged: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new(); // (temp, src, dst)

    for (i, (src, dst)) in pairs.iter().enumerate() {
        let srcp = PathBuf::from(src);
        let dstp = PathBuf::from(dst);
        let name = srcp
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let temp = srcp
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!(".__pixlens_tmp_{i}_{name}"));
        match std::fs::rename(&srcp, &temp) {
            Ok(()) => staged.push((temp, srcp, dstp)),
            Err(e) => errors.push(format!("{src}: {e}")),
        }
    }

    let mut renamed = 0usize;
    for (temp, src, dst) in staged {
        if dst.exists() {
            let _ = std::fs::rename(&temp, &src); // 回滚
            errors.push(format!("{}: 目标名已存在", dst.display()));
        } else if std::fs::rename(&temp, &dst).is_ok() {
            renamed += 1;
        } else {
            let _ = std::fs::rename(&temp, &src); // 回滚
            errors.push(format!("{}: 目标名不可用", src.display()));
        }
    }
    Ok((renamed, errors))
}

// ── 转换 / 压缩 / 缩放 ───────────────────
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOptions {
    pub format: String,          // jpg | png | webp
    pub quality: u8,             // 1-100（PNG / WebP 无损时忽略）
    pub scale_mode: String,      // none | longest | percent
    pub scale_value: u32,        // px（longest）或百分比（percent）
    pub out_policy: String,      // subdir | overwrite
    pub out_dir: Option<String>, // 显式输出目录（bench 用）
}

fn apply_scale(img: image::DynamicImage, opts: &ConvertOptions) -> image::DynamicImage {
    match opts.scale_mode.as_str() {
        "longest" if opts.scale_value > 0 => {
            let v = opts.scale_value;
            if img.width().max(img.height()) > v {
                crate::viewer::downscale_to(img, v)
            } else {
                img
            }
        }
        "percent" if opts.scale_value > 0 && opts.scale_value < 100 => {
            let long = img.width().max(img.height());
            let target = ((long as u64 * opts.scale_value as u64) / 100).max(1) as u32;
            crate::viewer::downscale_to(img, target)
        }
        _ => img,
    }
}

/// 转换输出：JPEG 用白底合成（透明内容不落深底），PNG/WebP 保留 RGBA
fn encode_output(img: image::DynamicImage, opts: &ConvertOptions) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    match opts.format.as_str() {
        "jpg" | "jpeg" => {
            let rgb = match img {
                image::DynamicImage::ImageRgba8(rgba) => {
                    let (w, h) = rgba.dimensions();
                    let mut out = image::RgbImage::new(w, h);
                    for (o, p) in out.pixels_mut().zip(rgba.pixels()) {
                        let a = p.0[3] as u32;
                        let inv = 255 - a;
                        *o = image::Rgb([
                            ((p.0[0] as u32 * a + 255 * inv) / 255) as u8,
                            ((p.0[1] as u32 * a + 255 * inv) / 255) as u8,
                            ((p.0[2] as u32 * a + 255 * inv) / 255) as u8,
                        ]);
                    }
                    out
                }
                other => other.to_rgb8(),
            };
            let enc = JpegEncoder::new_with_quality(&mut buf, opts.quality.clamp(1, 100));
            enc.write_image(rgb.as_raw(), rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
                .map_err(|e| e.to_string())?;
        }
        "png" => {
            let rgba = img.to_rgba8();
            let enc = PngEncoder::new_with_quality(&mut buf, CompressionType::Fast, FilterType::Adaptive);
            enc.write_image(rgba.as_raw(), rgba.width(), rgba.height(), ExtendedColorType::Rgba8)
                .map_err(|e| e.to_string())?;
        }
        "webp" => {
            // image crate 当前仅提供无损 WebP 编码（质量参数忽略）
            let rgba = img.to_rgba8();
            let enc = WebPEncoder::new_lossless(&mut buf);
            enc.write_image(rgba.as_raw(), rgba.width(), rgba.height(), ExtendedColorType::Rgba8)
                .map_err(|e| e.to_string())?;
        }
        other => return Err(format!("不支持的输出格式 {other}")),
    }
    Ok(buf)
}

/// 新文件冲突避让：name.ext → name (1).ext …
pub(crate) fn unique_path(mut p: PathBuf) -> PathBuf {
    if !p.exists() {
        return p;
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let parent = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    for n in 1..1000u32 {
        p = parent.join(format!("{stem} ({n}){ext}"));
        if !p.exists() {
            return p;
        }
    }
    p
}

fn convert_one(src: &str, opts: &ConvertOptions) -> Result<(), String> {
    let path = PathBuf::from(src);
    let img = crate::thumb::decode_with_limits(&path).map_err(|e| format!("解码失败: {e}"))?;
    let img = apply_scale(img, opts);
    let bytes = encode_output(img, opts)?;
    let format = opts.format.as_str();
    let out = match opts.out_policy.as_str() {
        "overwrite" => path.with_extension(format),
        _ => {
            let dir = match &opts.out_dir {
                Some(d) => PathBuf::from(d),
                None => path.parent().unwrap_or(Path::new(".")).join("pixlens_output"),
            };
            let _ = std::fs::create_dir_all(&dir);
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            dir.join(format!("{name}.{format}"))
        }
    };
    let out = if opts.out_policy == "overwrite" { out } else { unique_path(out) };
    std::fs::write(&out, bytes).map_err(|e| format!("写入失败: {e}"))
}

#[tauri::command]
pub fn batch_convert(
    app: tauri::AppHandle,
    paths: Vec<String>,
    opts: ConvertOptions,
) -> Result<u64, String> {
    let ext_ok = |p: &str| {
        Path::new(p)
            .extension()
            .map(|e| CONVERTIBLE.contains(&e.to_string_lossy().to_ascii_lowercase().as_str()))
            .unwrap_or(false)
    };
    let targets: Vec<String> = paths.into_iter().filter(|p| ext_ok(p)).collect();
    let total = targets.len();
    if total == 0 {
        return Err("没有可转换的文件".into());
    }
    let job_id = NEXT_JOB.fetch_add(1, Ordering::Relaxed);
    let cancel = Arc::new(AtomicBool::new(false));
    jobs().lock().insert(job_id, cancel.clone());

    let opts = Arc::new(opts);
    std::thread::spawn(move || {
        let done = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let stride = (total / 200).max(1);
        let canceled_flag = AtomicBool::new(false);

        targets.par_iter().for_each(|p| {
            if cancel.load(Ordering::Relaxed) {
                canceled_flag.store(true, Ordering::Relaxed);
                return;
            }
            match convert_one(p, &opts) {
                Ok(()) => {
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if d % stride == 0 {
                        emit_progress(
                            &app,
                            BatchProgress {
                                job_id,
                                done: d,
                                failed: failed.load(Ordering::Relaxed),
                                total,
                                current: p.clone(),
                                canceled: false,
                                finished: false,
                            },
                        );
                    }
                }
                Err(e) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    println!("[batch] 转换失败 {p}: {e}");
                }
            }
        });

        let canceled = canceled_flag.load(Ordering::Relaxed);
        emit_progress(
            &app,
            BatchProgress {
                job_id,
                done: done.load(Ordering::Relaxed),
                failed: failed.load(Ordering::Relaxed),
                total,
                current: String::new(),
                canceled,
                finished: true,
            },
        );
        jobs().lock().remove(&job_id);
    });
    Ok(job_id)
}

#[tauri::command]
pub fn batch_cancel(job_id: u64) -> Result<bool, String> {
    let mut map = jobs().lock();
    if let Some(flag) = map.get(&job_id) {
        flag.store(true, Ordering::Relaxed);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_expansion() {
        assert_eq!(expand_template("{原名}_{序号:3}", "IMG_001", 7, "2026-09-20"), "IMG_001_007");
        assert_eq!(expand_template("{日期}_{序号}", "a", 42, "2026-09-20"), "2026-09-20_42");
        assert_eq!(expand_template("x{未知}y", "a", 1, "d"), "x{未知}y");
        assert_eq!(expand_template("无token", "a", 1, "d"), "无token");
        assert_eq!(expand_template("{原名", "a", 1, "d"), "{原名");
        assert_eq!(expand_template("{序号:5}", "a", 42, "d"), "00042");
    }

    #[test]
    fn date_from_unix() {
        assert_eq!(unix_ms_to_date(0), "1970-01-01");
        assert_eq!(unix_ms_to_date(86_400_000), "1970-01-02");
        assert_eq!(unix_ms_to_date(946_684_800_000), "2000-01-01");
    }

    #[test]
    fn rename_conflict_detection_and_apply() {
        let dir = std::env::temp_dir().join("pixlens_rename_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        let c = dir.join("c.txt"); // 批外现存
        std::fs::write(&a, b"1").unwrap();
        std::fs::write(&b, b"2").unwrap();
        std::fs::write(&c, b"3").unwrap();

        let ap = a.to_string_lossy().into_owned();
        let bp = b.to_string_lossy().into_owned();

        // 预览：批内让位（b 将被改名走）不算冲突
        let pairs = batch_rename_preview(vec![ap.clone(), bp.clone()], "{序号:2}".into(), 1).unwrap();
        assert_eq!(pairs.len(), 2);
        assert!(pairs.iter().all(|p| !p.conflict), "批内让位不应冲突: {pairs:?}");
        assert_eq!(pairs[0].new_name, "01.txt");

        // 预览：目标名与批外文件冲突
        let cp = c.to_string_lossy().into_owned();
        let pairs2 = batch_rename_preview(vec![ap.clone()], "{序号}".into(), 3).unwrap(); // a → 3.txt? 不冲突
        assert!(!pairs2[0].conflict);

        // 交叉改名（a↔b）两阶段执行
        let (renamed, errs) =
            batch_rename_apply(vec![(ap.clone(), bp.clone()), (bp.clone(), ap.clone())]).unwrap();
        assert_eq!(renamed, 2, "交叉改名应成功: {errs:?}");
        assert_eq!(std::fs::read(&a).unwrap(), b"2"); // 内容互换
        assert_eq!(std::fs::read(&b).unwrap(), b"1");

        // 目标为批外现存文件 → 失败并回滚
        let (renamed2, errs2) =
            batch_rename_apply(vec![(ap, cp)]).unwrap();
        assert_eq!(renamed2, 0);
        assert!(!errs2.is_empty());
        assert!(a.exists(), "失败后应回滚保留原名");
        assert_eq!(std::fs::read(&c).unwrap(), b"3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_path_increments() {
        let dir = std::env::temp_dir().join("pixlens_unique_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.jpg");
        std::fs::write(&p, b"1").unwrap();
        let u = unique_path(p.clone());
        assert_eq!(u.file_name().unwrap().to_string_lossy(), "x (1).jpg");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
