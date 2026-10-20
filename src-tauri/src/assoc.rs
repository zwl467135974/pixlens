//! 系统默认看图软件（文件关联候选注册）
//!
//! Windows 10/11 不允许普通程序直写 UserChoice 默认关联（系统 hash 校验会拒绝，
//! 强行绕写也属杀软重点盯防行为）。采用 Chrome/Firefox 同款方案：
//!   1. HKCU 注册 ProgId + Capabilities → PixLens 进入各图片格式的"打开方式"列表
//!      与系统"默认应用"设置页；
//!   2. 无 UserChoice 的格式直接以 .ext 默认值生效（原值备份，取消注册时还原）；
//!   3. "设为默认"由前端跳转 ms-settings:defaultapps 让用户一键确认。
//! 全部 HKCU 写入，无需管理员；删除指向本程序的 UserChoice 是系统允许的操作。

use serde::Serialize;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, REG_OPTION_NON_VOLATILE,
    REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE, KEY_READ, KEY_SET_VALUE,
};

use crate::scan::IMAGE_EXTS;

const PROG_ID: &str = "PixLens.Image";
const APP_NAME: &str = "PixLens 图镜";
const APP_DESC: &str = "免费无广告的本地看图软件，支持 PSD / RAW / 视频缩略图";
/// .ext 原默认 ProgId 备份（"(none)" 哨兵 = 原本无默认值），与 ThumbBackup 同级
const BACKUP_KEY: &str = r"Software\PixLens\AssocBackup";
const CAPABILITIES_KEY: &str = r"Software\PixLens\Capabilities";
const NONE_SENTINEL: &str = "(none)";

// ── 注册表小工具（仅本模块使用） ─────────────────────

/// (持有物, 指针)：HSTRING 必须在 PCWSTR 使用期间存活
fn hold(s: &str) -> (HSTRING, PCWSTR) {
    let h = HSTRING::from(s);
    let p = PCWSTR(h.as_ptr());
    (h, p)
}

fn reg_sz_bytes(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity((s.len() + 1) * 2);
    for u in s.encode_utf16().chain(std::iter::once(0)) {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v
}

fn create_key(parent: HKEY, sub: &str) -> Result<HKEY, String> {
    let (_h, path) = hold(sub);
    let mut key = HKEY::default();
    let r = unsafe {
        RegCreateKeyExW(
            parent,
            path,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if r != ERROR_SUCCESS {
        return Err(format!("RegCreateKeyExW({sub}) 失败: {r:?}"));
    }
    Ok(key)
}

fn open_key(parent: HKEY, sub: &str, sam: REG_SAM_FLAGS) -> Option<HKEY> {
    let (_h, path) = hold(sub);
    let mut key = HKEY::default();
    let r = unsafe { RegOpenKeyExW(parent, path, None, sam, &mut key) };
    if r != ERROR_SUCCESS {
        None
    } else {
        Some(key)
    }
}

fn set_sz(parent: HKEY, sub: &str, value: Option<&str>, data: &str) -> Result<(), String> {
    let key = create_key(parent, sub)?;
    let (_vh, name) = match value {
        Some(v) => hold(v),
        None => (HSTRING::from(""), PCWSTR::null()),
    };
    let bytes = reg_sz_bytes(data);
    let r =
        unsafe { RegSetValueExW(key, name, None, REG_SZ, Some(bytes.as_slice())) };
    unsafe { let _ = RegCloseKey(key); };
    if r != ERROR_SUCCESS {
        return Err(format!("RegSetValueExW({sub}\\{value:?}) 失败: {r:?}"));
    }
    Ok(())
}

/// 读 REG_SZ / REG_EXPAND_SZ；键或值不存在、类型不符均返回 None
fn get_sz(parent: HKEY, sub: &str, value: Option<&str>) -> Option<String> {
    let key = open_key(parent, sub, KEY_READ)?;
    let result = (|| {
        let (_vh, name) = match value {
            Some(v) => hold(v),
            None => (HSTRING::from(""), PCWSTR::null()),
        };
        let mut ty = REG_VALUE_TYPE::default();
        let mut cb = 0u32;
        let r = unsafe { RegQueryValueExW(key, name, None, Some(&mut ty), None, Some(&mut cb)) };
        if r != ERROR_SUCCESS || (ty != REG_SZ && ty != REG_VALUE_TYPE(2)) {
            return None;
        }
        let mut buf = vec![0u8; cb as usize];
        let r = unsafe {
            RegQueryValueExW(key, name, None, Some(&mut ty), Some(buf.as_mut_ptr()), Some(&mut cb))
        };
        if r != ERROR_SUCCESS {
            return None;
        }
        let s = String::from_utf16_lossy(
            &buf[..cb as usize]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        );
        let s = s.trim_end_matches('\0');
        Some(s.to_string())
    })();
    unsafe { let _ = RegCloseKey(key); };
    result
}

fn del_value(parent: HKEY, sub: &str, value: Option<&str>) {
    let Some(key) = open_key(parent, sub, KEY_SET_VALUE) else { return };
    unsafe {
        let (_vh, name) = match value {
            Some(v) => hold(v),
            None => (HSTRING::from(""), PCWSTR::null()),
        };
        let _ = RegDeleteValueW(key, name);
        let _ = RegCloseKey(key);
    }
}

fn del_tree(root: HKEY, sub: &str) {
    let (_h, path) = hold(sub);
    unsafe { let _ = RegDeleteTreeW(root, path); }
}

fn del_key(parent: HKEY, sub: &str) {
    let (_h, path) = hold(sub);
    unsafe { let _ = RegDeleteKeyW(parent, path); }
}

fn key_exists(parent: HKEY, sub: &str) -> bool {
    open_key(parent, sub, KEY_READ).is_some()
}

// ── 对外操作 ─────────────────────────────

/// 注册为系统看图候选（ProgId + OpenWithProgids + Capabilities + 默认应用列表）。
/// 返回注册的扩展数。重复调用幂等（.ext 原默认值备份不覆盖）。
pub fn register() -> Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| format!("取可执行文件路径失败: {e}"))?;
    let exe = exe.to_string_lossy().replace('/', r"\");

    let classes = r"Software\Classes";
    // 1) ProgId：类型名 / 图标 / 打开命令
    let prog = format!(r"{classes}\{PROG_ID}");
    set_sz(HKEY_CURRENT_USER, &prog, None, APP_NAME)?;
    set_sz(HKEY_CURRENT_USER, &format!(r"{prog}\DefaultIcon"), None, &format!("\"{exe}\",0"))?;
    set_sz(HKEY_CURRENT_USER, &format!(r"{prog}\shell\open\command"), None,
           &format!("\"{exe}\" \"%1\""))?;

    // 2) 逐扩展：备份原默认值 → OpenWithProgids 候选 → .ext 默认值（无 UserChoice 时即生效）
    for ext in IMAGE_EXTS {
        let dot = format!(".{ext}");
        let ext_key = format!(r"{classes}\{dot}");
        let cur = get_sz(HKEY_CURRENT_USER, &ext_key, None).filter(|s| !s.is_empty());
        // 首次注册记录备份（升级重装不覆盖）
        if get_sz(HKEY_CURRENT_USER, BACKUP_KEY, Some(&dot)).is_none() {
            set_sz(HKEY_CURRENT_USER, BACKUP_KEY, Some(&dot),
                   cur.as_deref().unwrap_or(NONE_SENTINEL))?;
        }
        set_sz(HKEY_CURRENT_USER, &format!(r"{ext_key}\OpenWithProgids"), Some(PROG_ID), "")?;
        if cur.as_deref() != Some(PROG_ID) {
            set_sz(HKEY_CURRENT_USER, &ext_key, None, PROG_ID)?;
        }
    }

    // 3) 默认应用列表：Capabilities + RegisteredApplications（HKCU 即可，无需管理员）
    set_sz(HKEY_CURRENT_USER, CAPABILITIES_KEY, Some("ApplicationName"), APP_NAME)?;
    set_sz(HKEY_CURRENT_USER, CAPABILITIES_KEY, Some("ApplicationDescription"), APP_DESC)?;
    for ext in IMAGE_EXTS {
        set_sz(HKEY_CURRENT_USER, &format!(r"{CAPABILITIES_KEY}\FileAssociations"),
               Some(ext), PROG_ID)?;
    }
    set_sz(HKEY_CURRENT_USER, r"Software\RegisteredApplications", Some("PixLens"),
           CAPABILITIES_KEY)?;

    Ok(IMAGE_EXTS.len() as u32)
}

/// 取消注册并还原：删候选值 / 还原 .ext 默认值 / 删指向本程序的 UserChoice / 清 Capabilities。
/// 返回处理的扩展数。未注册时调用同样安全（幂等）。
pub fn unregister() -> Result<u32, String> {
    let classes = r"Software\Classes";
    let file_exts = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts";

    for ext in IMAGE_EXTS {
        let dot = format!(".{ext}");
        let ext_key = format!(r"{classes}\{dot}");
        // 1) 还原 .ext 默认值（有备份才动）
        if let Some(orig) = get_sz(HKEY_CURRENT_USER, BACKUP_KEY, Some(&dot)) {
            if orig == NONE_SENTINEL {
                del_value(HKEY_CURRENT_USER, &ext_key, None);
            } else {
                set_sz(HKEY_CURRENT_USER, &ext_key, None, &orig)?;
            }
            del_value(HKEY_CURRENT_USER, BACKUP_KEY, Some(&dot));
        }
        // 2) 打开方式候选
        del_value(HKEY_CURRENT_USER, &format!(r"{ext_key}\OpenWithProgids"), Some(PROG_ID));
        // 3) UserChoice 指向本程序 → 删除（系统回落为"选择打开方式"；删键是允许操作）
        let uc = format!(r"{file_exts}\{dot}\UserChoice");
        if get_sz(HKEY_CURRENT_USER, &uc, Some("ProgId")).as_deref() == Some(PROG_ID) {
            del_key(HKEY_CURRENT_USER, &uc);
        }
    }

    del_tree(HKEY_CURRENT_USER, CAPABILITIES_KEY);
    del_value(HKEY_CURRENT_USER, r"Software\RegisteredApplications", Some("PixLens"));
    del_tree(HKEY_CURRENT_USER, &format!(r"{classes}\{PROG_ID}"));
    del_key(HKEY_CURRENT_USER, BACKUP_KEY);
    Ok(IMAGE_EXTS.len() as u32)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssocStatus {
    /// 是否已注册为候选
    pub registered: bool,
    /// 当前已是系统默认（UserChoice 指向本程序）的扩展
    pub default_for: Vec<String>,
}

pub fn status() -> AssocStatus {
    let file_exts = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts";
    let default_for = IMAGE_EXTS
        .iter()
        .filter(|ext| {
            let uc = format!(r"{file_exts}\.{ext}\UserChoice");
            get_sz(HKEY_CURRENT_USER, &uc, Some("ProgId")).as_deref() == Some(PROG_ID)
        })
        .map(|e| e.to_string())
        .collect();
    AssocStatus {
        registered: key_exists(HKEY_CURRENT_USER, CAPABILITIES_KEY),
        default_for,
    }
}

/// 打开系统"默认应用"设置页（Win10/11 由用户在此一键设为默认）
pub fn open_settings_page() {
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", "ms-settings:defaultapps"])
        .spawn();
}

// ── Tauri 命令 ─────────────────────────────

#[tauri::command]
pub fn assoc_register() -> Result<u32, String> {
    register()
}

#[tauri::command]
pub fn assoc_unregister() -> Result<u32, String> {
    unregister()
}

#[tauri::command]
pub fn assoc_status() -> AssocStatus {
    status()
}

#[tauri::command]
pub fn open_default_apps() {
    open_settings_page();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_live_under_pixlens_namespace() {
        assert!(BACKUP_KEY.starts_with(r"Software\PixLens\"));
        assert!(CAPABILITIES_KEY.starts_with(r"Software\PixLens\"));
    }

    /// 注册 → 状态 → 取消 的注册表往返（真实 HKCU，测试机专用；
    /// CI 无注册表环境会失败，故默认忽略：cargo test -- --ignored）
    #[test]
    #[ignore]
    fn register_and_unregister_roundtrip() {
        let _ = unregister(); // 从干净状态开始
        let s0 = status();
        assert!(!s0.registered);

        let n = register().expect("注册失败");
        assert_eq!(n as usize, IMAGE_EXTS.len());
        assert!(status().registered);
        assert!(get_sz(HKEY_CURRENT_USER, r"Software\Classes\.jpg\OpenWithProgids", Some(PROG_ID)).is_some());
        assert!(key_exists(HKEY_CURRENT_USER, &format!(r"Software\Classes\{PROG_ID}")));
        assert!(get_sz(HKEY_CURRENT_USER, r"Software\RegisteredApplications", Some("PixLens")).is_some());

        register().expect("重复注册失败"); // 幂等

        let n2 = unregister().expect("取消注册失败");
        assert_eq!(n2 as usize, IMAGE_EXTS.len());
        assert!(!status().registered);
        assert!(get_sz(HKEY_CURRENT_USER, r"Software\Classes\.jpg\OpenWithProgids", Some(PROG_ID)).is_none());
        assert!(!key_exists(HKEY_CURRENT_USER, &format!(r"Software\Classes\{PROG_ID}")));
        assert!(!key_exists(HKEY_CURRENT_USER, BACKUP_KEY));
    }
}
