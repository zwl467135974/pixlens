//! 应用设置（doc/05 M7：缓存上限 256MB~8GB、默认排序、主题）
//!
//! 持久化于 %APPDATA%\com.pixlens.app\settings.json；缓存上限经原子量即时生效。

use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 缩略图/预览磁盘缓存上限（MB），256..8192
    pub cache_limit_mb: u32,
    /// name | mtime | size | type
    pub default_sort: String,
    /// dark | light
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cache_limit_mb: 1024,
            default_sort: "name".into(),
            theme: "dark".into(),
        }
    }
}

fn settings_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}

pub fn load(app: &tauri::AppHandle) -> Settings {
    let Some(p) = settings_path(app) else { return Settings::default() };
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(app: &tauri::AppHandle, s: &Settings) -> Result<(), String> {
    let Some(p) = settings_path(app) else { return Err("配置目录不可用".into()) };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(p, json).map_err(|e| e.to_string())
}

fn validate(s: &Settings) -> Result<(), String> {
    if !(256..=8192).contains(&s.cache_limit_mb) {
        return Err("缓存上限需在 256~8192 MB 之间".into());
    }
    if !["name", "mtime", "size", "type"].contains(&s.default_sort.as_str()) {
        return Err("排序键非法".into());
    }
    if !["dark", "light"].contains(&s.theme.as_str()) {
        return Err("主题非法".into());
    }
    Ok(())
}

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Settings {
    load(&app)
}

/// 应用设置并即时生效（缓存上限原子更新 + 触发 LRU 淘汰），返回当前缓存占用 MB
#[tauri::command]
pub fn set_settings(app: tauri::AppHandle, state: tauri::State<crate::AppState>, settings: Settings) -> Result<f64, String> {
    validate(&settings)?;
    let limit = settings.cache_limit_mb as u64 * 1024 * 1024;
    state.thumb.max_cache_bytes.store(limit, Ordering::Relaxed);
    save(&app, &settings)?;
    let dir = state.thumb.cache_dir.clone();
    crate::thumb::evict_if_needed(&dir, limit);
    Ok(dir_size_mb(&dir))
}

fn dir_size_mb(dir: &std::path::Path) -> f64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.metadata().ok().map(|m| m.len()))
                .sum::<u64>() as f64 / 1_048_576.0
        })
        .unwrap_or(0.0)
}
