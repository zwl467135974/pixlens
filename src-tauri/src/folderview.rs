//! 文件夹视图相关：类型描述串 + PixLens 自身的文件夹排序记忆
//!
//! 设计取舍（v1.4.1 实测结论）：Windows 不提供实时读取 Explorer 当前排序的公开途径——
//! shellbag（注册表）落盘时机由 Explorer 内部决定（延迟数分钟甚至需重启进程），
//! 废弃的 SHGetViewStatePropertyBag 在 Win10 上读不到数据，IFolderView2 只能查
//! "正开着"的窗口。"跟随 Explorer 排序"因此不可靠，已移除；只保留：
//!   1. 类型描述串（SHGetFileInfoW，"类型"列排序与 Explorer 对齐）
//!   2. PixLens 自身的文件夹排序记忆（settings 持久化，用户在应用内改的永久生效）

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_TYPENAME, SHGFI_USEFILEATTRIBUTES};

/// PixLens 记住的文件夹排序（用户在应用内改的；settings 持久化）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FolderSortMem {
    pub key: String,
    /// 1=升序 -1=降序
    pub dir: i32,
    /// 保存时刻（Unix ms）
    pub at: u64,
}

fn hold(s: &str) -> (HSTRING, PCWSTR) {
    let h = HSTRING::from(s);
    let p = PCWSTR(h.as_ptr());
    (h, p)
}

// ── 类型描述串（Explorer"类型"列同款）─────────────────

fn type_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 扩展名 → Explorer"类型"列文本（SHGFI_USEFILEATTRIBUTES 不碰文件，按扩展名查壳表）
pub fn type_name_for_ext(ext: &str) -> String {
    if ext.is_empty() {
        return String::new();
    }
    let key = ext.to_ascii_lowercase();
    if let Some(hit) = type_cache().lock().unwrap().get(&key) {
        return hit.clone();
    }
    let fake = format!("a.{}", key);
    let (_h, path) = hold(&fake);
    let mut info = SHFILEINFOW::default();
    let r = unsafe {
        SHGetFileInfoW(
            path,
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES,
        )
    };
    let name = if r != 0 {
        let end = info.szTypeName.iter().position(|&c| c == 0).unwrap_or(0);
        String::from_utf16_lossy(&info.szTypeName[..end])
    } else {
        String::new()
    };
    type_cache().lock().unwrap().insert(key, name.clone());
    name
}

// ── Tauri 命令 ─────────────────────────────

/// 生效排序：PixLens 自身记忆（用户在应用内改的）；无记忆返回 None 用全局默认
#[tauri::command]
pub fn get_folder_sort(app: tauri::AppHandle, path: String) -> Option<FolderSortMem> {
    crate::settings::load(&app).folder_sorts.get(&path).cloned()
}

#[tauri::command]
pub fn get_type_name(ext: String) -> String {
    type_name_for_ext(&ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names_are_shell_agreed() {
        // 与 Explorer"类型"列一致（本机 zh-CN；返回非空且非扩展名本身）
        let jpg = type_name_for_ext("jpg");
        assert!(!jpg.is_empty());
        assert_ne!(jpg, "jpg");
    }
}
