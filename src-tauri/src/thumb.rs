//! 缩略图管线（doc/03-架构设计 §3.2、§5）
//!
//! thumb://pixlens/?src=<URL编码路径>&w=<目标宽>  → 缩略图字节
//! 流程：缓存键(blake3 路径+大小+mtime) → 磁盘缓存命中直接返回；
//! 未命中 → 信号量准入 → 按扩展名解码 → 缩放 → JPEG(q82) → 写缓存 → 返回。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use walkdir::WalkDir;

use image::codecs::jpeg::JpegEncoder;
use image::DynamicImage;
use percent_encoding::percent_decode_str;
use tauri::Manager;

/// 简单计数限流门：并发达到上限时阻塞等待（替代不存在的 parking_lot::Semaphore）
pub struct Gate {
    max: usize,
    active: std::sync::Mutex<usize>,
    cv: std::sync::Condvar,
}

impl Gate {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            active: std::sync::Mutex::new(0),
            cv: std::sync::Condvar::new(),
        }
    }

    pub fn acquire(&self) -> GateGuard<'_> {
        let mut a = self.active.lock().unwrap();
        while *a >= self.max {
            a = self.cv.wait(a).unwrap();
        }
        *a += 1;
        GateGuard { gate: self }
    }
}

pub struct GateGuard<'a> {
    gate: &'a Gate,
}

impl Drop for GateGuard<'_> {
    fn drop(&mut self) {
        let mut a = self.gate.active.lock().unwrap();
        *a -= 1;
        self.gate.cv.notify_one();
    }
}

/// 缩略图解码并发 = min(物理核数-1, 8)
const MAX_CONCURRENT_DECODES: usize = 8;
/// 缩略图 JPEG 编码质量
const JPEG_QUALITY: u8 = 82;
/// 磁盘缓存默认上限（LRU 淘汰）
const DEFAULT_CACHE_LIMIT: u64 = 1024 * 1024 * 1024;
/// 解码限额：最大边长 / 单次解码内存（稳定性护栏，doc/04 §4）
const MAX_DIMENSION: u32 = 300_000;
const MAX_DECODE_ALLOC: u64 = 1_500_000_000;

pub struct ThumbState {
    pub cache_dir: PathBuf,
    /// 缩略图解码限流门
    pub sem: Gate,
    /// 查看器整帧解码限流门（独立于缩略图，避免翻页被滚动解码积压拖慢）
    pub view_sem: Gate,
    /// 缓存上限（设置页可改，原子生效）
    pub max_cache_bytes: std::sync::atomic::AtomicU64,
    writes: AtomicUsize,
}

impl ThumbState {
    pub fn new(app: &tauri::AppHandle) -> Self {
        let cache_dir = app
            .path()
            .app_cache_dir()
            .unwrap_or_else(|_| PathBuf::from(".cache"))
            .join("thumbs");
        let _ = fs::create_dir_all(&cache_dir);
        let permits = num_cpus::get_physical().saturating_sub(1).clamp(1, MAX_CONCURRENT_DECODES);
        Self {
            cache_dir,
            sem: Gate::new(permits),
            view_sem: Gate::new(permits),
            max_cache_bytes: std::sync::atomic::AtomicU64::new(DEFAULT_CACHE_LIMIT),
            writes: AtomicUsize::new(0),
        }
    }
}

pub struct ThumbOutput {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

pub fn handle(state: &ThumbState, uri: &tauri::http::Uri) -> ThumbOutput {
    let Some((src, w)) = parse_query(uri) else {
        return placeholder();
    };
    let t0 = Instant::now();
    match get_thumb(state, &src, w) {
        Ok(out) => {
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            if ms > 50.0 || cfg!(debug_assertions) {
                println!("[perf] thumb {} w={} → {} B in {:.1} ms", short_path(&src), w, out.bytes.len(), ms);
            }
            out
        }
        Err(e) => {
            println!("[thumb] 失败 {} : {e}", short_path(&src));
            placeholder()
        }
    }
}

fn short_path(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.chars().take(40).collect())
}

fn parse_query(uri: &tauri::http::Uri) -> Option<(String, u32)> {
    let q = uri.query()?;
    let mut src: Option<String> = None;
    let mut w: u32 = 256;
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=')?;
        match k {
            "src" => {
                src = Some(percent_decode_str(v).decode_utf8().ok()?.into_owned());
            }
            "w" => {
                w = v.parse().ok()?;
            }
            _ => {}
        }
    }
    let src = src?;
    if src.is_empty() {
        return None;
    }
    Some((src, w.clamp(32, 1024)))
}

fn get_thumb(state: &ThumbState, src: &str, w: u32) -> std::io::Result<ThumbOutput> {
    let path = PathBuf::from(&src);
    let md = fs::metadata(&path)?;
    if !md.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "不是文件"));
    }
    let mtime = md
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    // 缓存键 = blake3(路径|大小|mtime)[..24] + 宽度；文件变化自动失效
    let key = cache_key(&path, md.len(), mtime, w);

    // AVIF / SVG：透传原始字节给 WebView 原生解码
    if ext == "avif" || ext == "svg" {
        let mime = if ext == "svg" { "image/svg+xml" } else { "image/avif" };
        let cache = state.cache_dir.join(format!("{key}.{ext}"));
        if let Ok(bytes) = fs::read(&cache) {
            return Ok(ThumbOutput { bytes, mime });
        }
        let _permit = state.sem.acquire();
        let bytes = fs::read(&path)?;
        let _ = fs::write(&cache, &bytes);
        state.note_write();
        return Ok(ThumbOutput { bytes, mime });
    }

    let cache = state.cache_dir.join(format!("{key}.jpg"));
    if let Ok(bytes) = fs::read(&cache) {
        return Ok(ThumbOutput { bytes, mime: "image/jpeg" });
    }

    // PSD / PSB：1036 内嵌缩略图快路径 → 无内嵌则合成图跨步解码（doc/03 §6.2）
    if ext == "psd" || ext == "psb" {
        let out = psd_thumb(state, &path, w)?;
        let _ = fs::write(&cache, &out.bytes);
        state.note_write();
        return Ok(out);
    }

    // 视频：Media Foundation 提取代表帧（doc/03 §6.3）
    if crate::scan::is_video_ext(&ext) {
        let out = video_thumb(state, &path, w)?;
        let _ = fs::write(&cache, &out.bytes);
        state.note_write();
        return Ok(out);
    }

    let _permit = state.sem.acquire();
    let img = decode_with_limits(&path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let bytes = encode_jpeg(img, w)?;
    let _ = fs::write(&cache, &bytes);
    state.note_write();
    Ok(ThumbOutput { bytes, mime: "image/jpeg" })
}

/// 视频缩略图：MF 提取代表帧 → 缩放 → JPEG（缓存键与图片共用一套）
#[cfg(windows)]
fn video_thumb(state: &ThumbState, path: &Path, w: u32) -> std::io::Result<ThumbOutput> {
    let _permit = state.sem.acquire();
    let frame = crate::codecs::video::extract_frame(path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let bytes = encode_jpeg(image::DynamicImage::ImageRgba8(frame), w)?;
    Ok(ThumbOutput { bytes, mime: "image/jpeg" })
}

#[cfg(not(windows))]
fn video_thumb(_state: &ThumbState, _path: &Path, _w: u32) -> std::io::Result<ThumbOutput> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "仅支持 Windows"))
}

/// 缩放 + 压暗透明底 + JPEG(q82)；缓存写入由调用方负责
fn encode_jpeg(img: DynamicImage, w: u32) -> std::io::Result<Vec<u8>> {
    let thumb = downscale(img, w);
    let rgb = flatten_alpha(thumb);
    let mut buf: Vec<u8> = Vec::with_capacity(16 * 1024);
    let enc = JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY);
    rgb.write_with_encoder(enc)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(buf)
}

/// PSD/PSB 缩略图：mmap → 1036 内嵌 JPEG 快路径；无内嵌则合成图跨步解码缩略
fn psd_thumb(state: &ThumbState, path: &Path, w: u32) -> std::io::Result<ThumbOutput> {
    let invalid = |e: crate::codecs::psd::PsdError| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    };
    let file = fs::File::open(path)?;
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    let info = crate::codecs::psd::parse(&mmap).map_err(invalid)?;

    // 快路径：内嵌缩略图（JFIF JPEG），毫秒级
    if let Some(jpeg) = crate::codecs::psd::embedded_thumbnail(&mmap, &info) {
        return Ok(ThumbOutput { bytes: jpeg, mime: "image/jpeg" });
    }

    // 合成图：跨步解码直接到缩略尺寸
    let _permit = state.sem.acquire();
    let composite = crate::codecs::psd::decode_composite(&mmap, &info, w, 1.0).map_err(invalid)?;
    let img = DynamicImage::ImageRgba8(composite.image);
    let bytes = encode_jpeg(img, w)?;
    Ok(ThumbOutput { bytes, mime: "image/jpeg" })
}

fn cache_key(path: &Path, size: u64, mtime: u64, w: u32) -> String {    let mut h = blake3::Hasher::new();
    h.update(path.to_string_lossy().as_bytes());
    h.update(b"|");
    h.update(&size.to_le_bytes());
    h.update(b"|");
    h.update(&mtime.to_le_bytes());
    let hex = h.finalize().to_hex().to_string();
    format!("{}_{}", &hex[..24], w)
}

pub(crate) fn decode_with_limits(path: &Path) -> image::ImageResult<DynamicImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    reader.limits(limits);
    reader.decode()
}

/// 目标宽内缩放（不放大）
pub(crate) fn downscale(img: DynamicImage, w: u32) -> DynamicImage {
    if img.width() <= w {
        return img;
    }
    let h = ((w as u64 * img.height() as u64) / img.width().max(1) as u64).max(1) as u32;
    img.thumbnail(w, h)
}

/// 带透明通道的图压到深色底上，避免黑底/白底突兀
pub(crate) fn flatten_alpha(img: DynamicImage) -> image::RgbImage {
    match img {
        DynamicImage::ImageRgba8(rgba) => {
            let (w, h) = rgba.dimensions();
            let bg: [u8; 3] = [42, 42, 48];
            let mut out = image::RgbImage::new(w, h);
            for (o, p) in out.pixels_mut().zip(rgba.pixels()) {
                let a = p.0[3] as u32;
                let inv = 255 - a;
                *o = image::Rgb([
                    ((p.0[0] as u32 * a + bg[0] as u32 * inv) / 255) as u8,
                    ((p.0[1] as u32 * a + bg[1] as u32 * inv) / 255) as u8,
                    ((p.0[2] as u32 * a + bg[2] as u32 * inv) / 255) as u8,
                ]);
            }
            out
        }
        other => other.to_rgb8(),
    }
}

/// 深灰占位（解码失败 / 未支持格式）
fn placeholder() -> ThumbOutput {
    let mut img = image::RgbImage::from_pixel(96, 96, image::Rgb([58, 58, 66]));
    for p in img.pixels_mut() {
        p.0 = [p.0[0] / 2 + 29, p.0[1] / 2 + 29, p.0[2] / 2 + 33];
    }
    let mut buf: Vec<u8> = Vec::new();
    let enc = JpegEncoder::new_with_quality(&mut buf, 70);
    let _ = img.write_with_encoder(enc);
    ThumbOutput { bytes: buf, mime: "image/jpeg" }
}

impl ThumbState {
    /// 测试用：临时缓存目录 + 固定并发
    #[cfg(test)]
    pub fn for_test() -> Self {
        let dir = std::env::temp_dir().join(format!("pixlens-thumb-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        Self {
            cache_dir: dir,
            sem: Gate::new(2),
            view_sem: Gate::new(2),
            max_cache_bytes: std::sync::atomic::AtomicU64::new(1024 * 1024 * 1024),
            writes: AtomicUsize::new(0),
        }
    }

    /// 记录一次缓存写入，周期性触发 LRU 淘汰
    pub fn note_write(&self) {
        let n = self.writes.fetch_add(1, Ordering::Relaxed) + 1;
        if n % 128 == 0 {
            let dir = self.cache_dir.clone();
            let max = self.max_cache_bytes.load(std::sync::atomic::Ordering::Relaxed);
            std::thread::spawn(move || evict_if_needed(&dir, max));
        }
    }
}

/// 视频缩略图全管线（MF 提帧 → JPEG → 磁盘缓存命中）。本地手动跑。
#[cfg(windows)]
#[test]
#[ignore]
fn video_thumb_pipeline() {
    let dir = PathBuf::from(r"C:\Users\GA\Desktop");
    let target = WalkDir::new(&dir)
        .min_depth(1)
        .max_depth(2)
        .into_iter()
        .flatten()
        .find(|e| e.path().extension().map(|x| x == "mp4").unwrap_or(false))
        .map(|e| e.into_path())
        .expect("桌面下无 mp4 样本");
    let state = ThumbState::for_test();
    let src = target.to_string_lossy().into_owned();

    let t0 = Instant::now();
    let out1 = get_thumb(&state, &src, 256).expect("首次提取失败");
    let cold = t0.elapsed().as_millis();
    assert_eq!(out1.mime, "image/jpeg");
    assert!(out1.bytes.len() > 1000, "JPEG 过小: {}", out1.bytes.len());
    assert!(out1.bytes.starts_with(&[0xFF, 0xD8]), "非 JPEG 头");

    let t1 = Instant::now();
    let out2 = get_thumb(&state, &src, 256).expect("缓存读取失败");
    let warm = t1.elapsed().as_millis();
    assert_eq!(out1.bytes, out2.bytes, "缓存命中内容不一致");
    println!("[video-thumb] 冷 {cold}ms / 热 {warm}ms / {} B", out1.bytes.len());
    let _ = fs::remove_dir_all(&state.cache_dir);
}

/// LRU 淘汰：超限后按文件 mtime 从旧到新删除，直到降到 90% 以下
pub fn evict_if_needed(cache_dir: &Path, max_bytes: u64) {
    let mut items: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    let mut total: u64 = 0;
    let Ok(rd) = fs::read_dir(cache_dir) else { return };
    for entry in rd.flatten() {
        let Ok(md) = entry.metadata() else { continue };
        if !md.is_file() {
            continue;
        }
        let len = md.len();
        let mtime = md.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        total += len;
        items.push((entry.path(), len, mtime));
    }
    if total <= max_bytes {
        return;
    }
    items.sort_by_key(|(_, _, t)| *t);
    let target = max_bytes / 10 * 9;
    let mut freed: u64 = 0;
    let mut removed = 0;
    for (path, len, _) in items {
        if total - freed <= target {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            freed += len;
            removed += 1;
        }
    }
    println!("[cache] LRU 淘汰：删除 {removed} 个文件，释放 {:.1} MB", freed as f64 / 1_048_576.0);
}

pub fn wipe_cache(cache_dir: &Path) -> Result<(), String> {
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(cache_dir).map_err(|e| e.to_string())?;
    println!("[cache] 已清空缓存目录 {}", cache_dir.display());
    Ok(())
}
