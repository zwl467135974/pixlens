//! 文件夹扫描 + 变更监听（doc/03-架构设计 §3.1）

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant, UNIX_EPOCH};

use notify::event::{EventKind, ModifyKind, RenameMode};
use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{Emitter, Manager};
use walkdir::WalkDir;

/// 图片扩展名白名单（小写）
pub const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "ico", "avif", "svg", "tif", "tiff", "psd", "psb",
    "hdr",
];

/// 视频扩展名白名单（小写）：首帧由 Media Foundation 提取，双击交给系统播放器
pub const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv"];

pub fn is_image_ext(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    IMAGE_EXTS.contains(&e.as_str())
}

pub fn is_video_ext(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    VIDEO_EXTS.contains(&e.as_str())
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub ext: String,
    pub size: u64,
    /// 毫秒级 Unix 时间戳（与缩略图缓存键联动）
    pub mtime: u64,
    /// "image" | "video"：视频走 MF 首帧缩略图 + 系统播放器打开
    pub kind: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub entries: Vec<Entry>,
    pub elapsed_ms: f64,
}

/// 只收集元数据，不打开文件内容
pub fn entry_from_path(p: &Path) -> Option<Entry> {
    let md = fs::metadata(p).ok()?;
    if !md.is_file() {
        return None;
    }
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let kind = if is_image_ext(&ext) {
        "image"
    } else if is_video_ext(&ext) {
        "video"
    } else {
        return None;
    };
    let mtime = md
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some(Entry {
        name: p.file_name()?.to_string_lossy().into_owned(),
        path: p.to_string_lossy().into_owned(),
        ext: ext.to_ascii_lowercase(),
        size: md.len(),
        mtime,
        kind,
    })
}

#[tauri::command]
pub fn scan_folder(app: tauri::AppHandle, path: String) -> Result<ScanResult, String> {
    let t0 = Instant::now();
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("不是有效文件夹: {}", path));
    }

    let mut entries: Vec<Entry> = Vec::new();
    for item in WalkDir::new(&dir).min_depth(1) {
        let Ok(item) = item else { continue };
        if !item.file_type().is_file() {
            continue;
        }
        if let Some(e) = entry_from_path(item.path()) {
            entries.push(e);
        }
    }
    // 基础顺序按路径排序，保证稳定；排序/过滤由前端负责
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!("[perf] scan_folder: {} files in {:.1} ms ({})", entries.len(), ms, path);

    start_watch(app, dir);

    Ok(ScanResult { entries, elapsed_ms: ms })
}

/// 文件夹变更事件（防抖后推送给前端做增量增删改）
#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct FsChanged {
    pub created: Vec<Entry>,
    pub removed: Vec<String>,
    pub updated: Vec<Entry>,
}

/// 同时只监听一个文件夹：换文件夹时丢弃旧 watcher
fn start_watch(app: tauri::AppHandle, dir: PathBuf) {
    let state = app.state::<crate::AppState>();
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    match notify::recommended_watcher(tx) {
        Ok(mut w) => {
            if let Err(e) = w.watch(&dir, RecursiveMode::Recursive) {
                println!("[watch] watch 失败: {e}");
            }
            *state.watcher.lock() = Some(w);
            std::thread::spawn(move || debounce_loop(app, rx));
        }
        Err(e) => println!("[watch] 初始化失败: {e}"),
    }
}

fn debounce_loop(app: tauri::AppHandle, rx: std::sync::mpsc::Receiver<notify::Result<notify::Event>>) {
    const DEBOUNCE: Duration = Duration::from_millis(300);
    loop {
        // 等第一个事件（阻塞）
        let mut events = match rx.recv() {
            Ok(Ok(e)) => vec![e],
            Ok(Err(_)) => continue,
            Err(_) => break, // watcher 已被替换/释放
        };
        // 收集防抖窗口内的事件
        loop {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(Ok(e)) => events.push(e),
                Ok(Err(_)) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    emit_changes(&app, &events);
                    return;
                }
            }
        }
        emit_changes(&app, &events);
    }
}

fn emit_changes(app: &tauri::AppHandle, events: &[notify::Event]) {
    let mut created: HashSet<PathBuf> = HashSet::new();
    let mut removed: HashSet<PathBuf> = HashSet::new();
    let mut updated: HashSet<PathBuf> = HashSet::new();

    for ev in events {
        match &ev.kind {
            EventKind::Create(_) => {
                for p in &ev.paths {
                    created.insert(p.clone());
                    removed.remove(p);
                }
            }
            EventKind::Remove(_) => {
                for p in &ev.paths {
                    removed.insert(p.clone());
                    created.remove(p);
                }
            }
            EventKind::Modify(ModifyKind::Name(mode)) => {
                // 重命名：paths = [from, to]
                if *mode == RenameMode::Both && ev.paths.len() == 2 {
                    removed.insert(ev.paths[0].clone());
                    created.insert(ev.paths[1].clone());
                } else {
                    for p in &ev.paths {
                        updated.insert(p.clone());
                    }
                }
            }
            EventKind::Modify(_) => {
                for p in &ev.paths {
                    updated.insert(p.clone());
                }
            }
            _ => {}
        }
    }

    let payload = build_payload(created, removed, updated);
    if payload.created.is_empty() && payload.removed.is_empty() && payload.updated.is_empty() {
        return;
    }
    println!(
        "[watch] fs-changed: +{} -{} ~{}",
        payload.created.len(),
        payload.removed.len(),
        payload.updated.len()
    );
    if let Err(e) = app.emit("fs-changed", payload) {
        println!("[watch] 事件推送失败: {e}");
    }
}

fn build_payload(
    created: HashSet<PathBuf>,
    removed: HashSet<PathBuf>,
    updated: HashSet<PathBuf>,
) -> FsChanged {
    let mut out = FsChanged::default();
    for p in created {
        if let Some(e) = entry_from_path(&p) {
            out.created.push(e);
        }
    }
    for p in removed {
        // 只关心支持的媒体扩展名（图片 + 视频）
        let ext = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        if is_image_ext(&ext) || is_video_ext(&ext) {
            out.removed.push(p.to_string_lossy().into_owned());
        }
    }
    for p in updated {
        match entry_from_path(&p) {
            Some(e) => out.updated.push(e),
            // 文件已不存在：转为删除
            None => out.removed.push(p.to_string_lossy().into_owned()),
        }
    }
    out
}
