//! 窗口几何记忆 + 上次文件夹恢复（Rust 侧维护，前端 set_settings 不触碰）
//!
//! 启动时在 setup 阶段恢复（早于首帧，无闪烁）；Moved/Resized 事件更新
//! 内存中的普通几何（最大化期间不记录，保留上次普通几何便于还原）；
//! 关闭时随 settings.json 一并落盘。

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeom {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub maximized: bool,
}

/// 启动恢复：几何落在某块显示器上才应用位置（防止拔掉显示器后窗口飞出屏幕）
pub fn restore(app: &tauri::AppHandle) {
    use tauri::Manager;
    let Some(win) = app.get_webview_window("main") else { return };
    let Some(g) = crate::settings::load(app).window else { return };
    if g.w < 400 || g.h < 300 {
        return;
    }
    let _ = win.set_size(tauri::PhysicalSize::new(g.w, g.h));
    let on_screen = win.available_monitors().map(|ms| {
        ms.iter().any(|m| {
            let (mp, msz) = (m.position(), m.size());
            // 与显示器工作区有 ≥100px 交叠才视为可见
            g.x < mp.x + msz.width as i32 - 100
                && g.x + g.w as i32 > mp.x + 100
                && g.y < mp.y + msz.height as i32 - 100
                && g.y + g.h as i32 > mp.y + 100
        })
    });
    if on_screen.unwrap_or(true) {
        let _ = win.set_position(tauri::PhysicalPosition::new(g.x, g.y));
    }
    if g.maximized {
        let _ = win.maximize();
    }
}

/// Moved/Resized 时更新内存几何（轻量，关闭时才写盘）
pub fn track(app: &tauri::AppHandle) {
    use tauri::Manager;
    let Some(win) = app.get_webview_window("main") else { return };
    // 最小化窗口被系统挪到 (-32000,-32000) 且尺寸塌缩为 160x28——不是真实几何
    if win.is_minimized().unwrap_or(false) || win.is_maximized().unwrap_or(false) {
        return; // 最大化/最小化期间不覆盖普通几何
    }
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.inner_size()) else { return };
    // 存客户区尺寸（set_size 同语义），外框尺寸会导致每次恢复胖一圈
    if size.width < 400 || size.height < 300 {
        return;
    }
    let state = app.state::<crate::AppState>();
    *state.geom.lock() = Some(WindowGeom {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
        maximized: false,
    });
}

/// 关闭时持久化：普通几何 + 当前是否最大化
pub fn persist(app: &tauri::AppHandle) {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    let mut g = *state.geom.lock();
    if let Some(g) = g.as_mut() {
        if let Some(win) = app.get_webview_window("main") {
            g.maximized = win.is_maximized().unwrap_or(false);
        }
    }
    let mut s = crate::settings::load(app);
    s.window = g;
    let _ = crate::settings::save(app, &s);
}
