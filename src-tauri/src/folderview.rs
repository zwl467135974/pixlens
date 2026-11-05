//! Explorer 文件夹视图记忆读取（shellbag，只读不改）
//!
//! Explorer 把每个文件夹记忆的视图设置存在注册表（shellbag）：
//!   `...\Shell\BagMRU` 树记录 文件夹→bag 编号；`...\Shell\Bags\<n>\Shell\{文件夹类型GUID}`
//!   是 REG_BINARY，布局 = [分组键 PROPERTYKEY 20B] + N×[排序键 PROPERTYKEY 20B + 方向 i32 4B]
//! PROPERTYKEY = fmtid(16B) + pid(u32LE)；方向 1=升序 -1=降序（ShObjIdl_core.h SORTDIRECTION）
//! PKEY（propkey.h 核对，fmtid 同 {B725F130-47EF-101A-A5F1-02608C9EEBAC}）：
//!   名称 ItemNameDisplay=10 · 修改时间 DateModified=14 · 大小 Size=12 · 类型 ItemTypeName=4
//! 类型描述串（"PNG 文件"等，"类型"排序用）经 SHGetFileInfoW 按扩展名取，与 Explorer 列一致。
//! 全程只读——不修改 Explorer 的记忆。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_TYPENAME, SHGFI_USEFILEATTRIBUTES};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, HKEY,
    HKEY_CURRENT_USER, REG_BINARY, REG_VALUE_TYPE, KEY_READ,
};

/// Explorer 排序键 → PixLens SortKey（"name" | "mtime" | "size" | "type"）
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FolderSortInfo {
    pub key: String,
    /// 1=升序 -1=降序
    pub dir: i32,
}

// ── 注册表小工具 ─────────────────────────────

fn hold(s: &str) -> (HSTRING, PCWSTR) {
    let h = HSTRING::from(s);
    let p = PCWSTR(h.as_ptr());
    (h, p)
}

fn open_key(path: &str) -> Option<HKEY> {
    let (_h, p) = hold(path);
    let mut key = HKEY::default();
    let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, p, None, KEY_READ, &mut key) };
    if r != ERROR_SUCCESS { None } else { Some(key) }
}

/// 枚举某键全部值：返回 (值名, 数据字节) 列表（只关心二进制值）
fn enum_values(key: HKEY) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut name_buf = vec![0u16; 256];
    let mut data_buf = vec![0u8; 16384];
    for i in 0..1024u32 {
        let mut name_len = name_buf.len() as u32;
        let mut ty = REG_VALUE_TYPE::default();
        let mut data_len = data_buf.len() as u32;
        let r = unsafe {
            RegEnumValueW(
                key,
                i,
                Some(PWSTR(name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut ty.0),
                Some(data_buf.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        if r != ERROR_SUCCESS {
            break;
        }
        let name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
        if ty == REG_BINARY && data_len > 0 {
            out.push((name, data_buf[..data_len as usize].to_vec()));
        }
    }
    out
}

fn get_u32(key: HKEY, value: &str) -> Option<u32> {
    let (_vh, name) = hold(value);
    let mut out = 0u32;
    let r = unsafe {
        RegQueryValueExW(
            key,
            name,
            None,
            None,
            Some(&mut out as *mut u32 as *mut u8),
            Some(&mut (std::mem::size_of::<u32>() as u32)),
        )
    };
    if r != ERROR_SUCCESS { None } else { Some(out) }
}

fn get_binary(key: HKEY, value: &str) -> Option<Vec<u8>> {
    let (_vh, name) = hold(value);
    let mut len = 0u32;
    let r = unsafe { RegQueryValueExW(key, name, None, Some(&mut REG_VALUE_TYPE::default()), None, Some(&mut len)) };
    if r != ERROR_SUCCESS || len == 0 || len > 1 << 20 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    let r = unsafe {
        RegQueryValueExW(key, name, None, Some(&mut REG_VALUE_TYPE::default()), Some(buf.as_mut_ptr()), Some(&mut len))
    };
    if r != ERROR_SUCCESS { None } else { Some(buf) }
}

/// 枚举子键名（bag 的 Shell 下是 {文件夹类型GUID} 子键）
fn enum_subkeys(key: HKEY) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = vec![0u16; 256];
    for i in 0..256u32 {
        let mut len = buf.len() as u32;
        let r = unsafe {
            RegEnumKeyExW(key, i, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None)
        };
        if r != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    out
}

/// 读 bag（编号）记忆的排序：Shell\{文件夹类型GUID}\Sort 逐个子键试
fn bag_sort(slot: u32) -> Option<FolderSortInfo> {
    let bag_shell = format!(r"{BAGS_ROOT}\{slot}\Shell");
    let key = open_key(&bag_shell)?;
    let mut result = None;
    for sub in enum_subkeys(key) {
        if !sub.starts_with('{') {
            continue;
        }
        let (_h, sub_path) = hold(&sub);
        let mut sub_key = HKEY::default();
        let r = unsafe { RegOpenKeyExW(key, sub_path, None, KEY_READ, &mut sub_key) };
        if r != ERROR_SUCCESS {
            continue;
        }
        if let Some(blob) = get_binary(sub_key, "Sort") {
            if let Some(info) = parse_sort_blob(&blob) {
                result = Some(info);
            }
        }
        unsafe { let _ = RegCloseKey(sub_key); };
        if result.is_some() {
            break;
        }
    }
    unsafe { let _ = RegCloseKey(key); };
    result
}

// ── Sort blob 解析 ─────────────────────────────

/// shell 属性集 fmtid（propkey.h 核对）：名称/时间/大小/类型都在这里
const SHELL_FMTID: [u8; 16] = [
    0x30, 0xF1, 0x25, 0xB7, 0xEF, 0x47, 0x1A, 0x10, 0xA5, 0xF1, 0x02, 0x60, 0x8C, 0x9E, 0xEB, 0xAC,
];
/// System.ItemType 的 fmtid（"类型"列的另一种键）
const ITEM_TYPE_FMTID: [u8; 16] = [
    0xA6, 0x6A, 0x63, 0x28, 0x3D, 0x95, 0xD2, 0x11, 0xB5, 0xD6, 0x00, 0xC0, 0x4F, 0xD9, 0x18, 0xD0,
];

/// PROPERTYKEY 的 pid → PixLens 排序键
fn pid_to_key(fmtid: &[u8; 16], pid: u32) -> Option<&'static str> {
    if *fmtid == SHELL_FMTID {
        return match pid {
            10 => Some("name"),   // PKEY_ItemNameDisplay
            14 => Some("mtime"),  // PKEY_DateModified
            12 => Some("size"),   // PKEY_Size
            4 => Some("type"),    // PKEY_ItemTypeText（Explorer"类型"列文本）
            _ => None,
        };
    }
    if *fmtid == ITEM_TYPE_FMTID && pid == 11 {
        return Some("type"); // PKEY_ItemType（扩展名形式）
    }
    None
}

/// 解析 Sort blob：跳过 20B 分组键，取第一个可识别的排序键（主排序）
fn parse_sort_blob(blob: &[u8]) -> Option<FolderSortInfo> {
    if blob.len() < 20 {
        return None;
    }
    let mut off = 20;
    while off + 24 <= blob.len() {
        let mut fmtid = [0u8; 16];
        fmtid.copy_from_slice(&blob[off..off + 16]);
        let pid = u32::from_le_bytes(blob[off + 16..off + 20].try_into().ok()?);
        let dir = i32::from_le_bytes(blob[off + 20..off + 24].try_into().ok()?);
        if dir == 1 || dir == -1 {
            if let Some(key) = pid_to_key(&fmtid, pid) {
                return Some(FolderSortInfo { key: key.to_string(), dir });
            }
        }
        off += 24;
    }
    None
}

// ── BagMRU 树查找 ─────────────────────────────

/// 从 SHITEMID blob 提取候选名（UTF-16 可打印 run；过滤控制区/私用区）
fn item_names(blob: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let units: Vec<u16> = blob
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut cur = String::new();
    for u in units {
        let printable = (0x20..0x7F).contains(&u)
            || (0x4E00..=0x9FFF).contains(&u)
            || (0x3000..=0x303F).contains(&u)
            || (0xFF00..=0xFFEF).contains(&u)
            || (0x00C0..=0x024F).contains(&u);
        if printable && char::from_u32(u as u32).is_some() {
            cur.push(char::from_u32(u as u32).unwrap());
            continue;
        }
        if cur.chars().count() >= 2 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.clear();
        }
    }
    if cur.chars().count() >= 2 {
        out.push(cur);
    }
    out
}

fn norm_component(s: &str) -> String {
    s.trim_matches(['/', '\\']).trim().to_lowercase()
}

/// ANSI 可打印 run（盘符项是 "D:\" 这类 ASCII 串）
fn item_names_ansi(blob: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for &b in blob {
        if (0x20..0x7F).contains(&b) {
            cur.push(b as char);
            continue;
        }
        if cur.chars().count() >= 2 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.clear();
        }
    }
    if cur.chars().count() >= 2 {
        out.push(cur);
    }
    out
}

fn blob_matches(blob: &[u8], comp: &str, loose: bool) -> bool {
    let comp = comp.to_lowercase();
    for name in item_names(blob) {
        let n = norm_component(&name);
        // 垃圾黏头（"安ÂBaiduNetdiskDownload"）→ ends_with；
        // 系统项友好名嵌在长 run（"\"Users.@shell32.dll,-21813"）→ contains（短 comp 不用，防误配）
        if n == comp || n.ends_with(&comp) || (comp.chars().count() >= 4 && n.contains(&comp)) {
            return true;
        }
        // 盘符匹配放宽：bag 里盘名可能是 "本地磁盘 (D:)" 形式
        if loose && n.contains(&comp) {
            return true;
        }
    }
    false
}

/// 盘符项匹配：ANSI run（"D:\"、"/D:\"）归一后 == "d:"
fn blob_matches_drive(blob: &[u8], letter: &str) -> bool {
    let want = format!("{letter}:");
    item_names_ansi(blob).iter().any(|n| norm_component(n) == want)
}

/// 从节点起深度受限收集全部盘符子节点（盘符在树里可能出现多处：Desktop 直挂/此电脑下）
fn find_drive_nodes(start: &str, letter: &str, depth: u32, out: &mut Vec<String>) {
    if depth == 0 {
        return;
    }
    let Some(key) = open_key(start) else { return };
    let mut children: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, data) in enum_values(key) {
        if name.chars().all(|c| c.is_ascii_digit()) {
            children.push((name, data));
        }
    }
    unsafe { let _ = RegCloseKey(key); };
    for (name, data) in &children {
        if blob_matches_drive(data, letter) {
            out.push(format!(r"{start}\{name}"));
        }
    }
    for (name, _) in &children {
        find_drive_nodes(&format!(r"{start}\{name}"), letter, depth - 1, out);
    }
}

/// BagMRU 根（Win7+ 主位置）；老位置作为兜底
const BAGMRU_ROOT: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU";
const BAGS_ROOT: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags";

fn node_slot(path: &str) -> Option<u32> {
    let key = open_key(path)?;
    let v = get_u32(key, "NodeSlot");
    unsafe { let _ = RegCloseKey(key); };
    v
}

/// 拆路径组件：D:\a\b → ["d", "a", "b"]；非盘符路径返回 None
fn path_components(path: &str) -> Option<Vec<String>> {
    let p = path.trim_end_matches(['\\', '/']);
    let mut comps: Vec<String> = Vec::new();
    let idx = p.find(':')?;
    comps.push(p[..idx].to_lowercase());
    for seg in p[idx + 1..].split(['\\', '/']) {
        if !seg.is_empty() {
            comps.push(seg.to_string());
        }
    }
    if comps.is_empty() {
        None
    } else {
        Some(comps)
    }
}

/// 在 BagMRU 树中定位该文件夹的全部候选节点（盘符/名字都可能多处出现）
fn bag_nodes_for(path: &str) -> Vec<String> {
    let Some(comps) = path_components(path) else {
        return Vec::new();
    };
    let mut drives = Vec::new();
    find_drive_nodes(BAGMRU_ROOT, &comps[0], 3, &mut drives);
    let mut nodes = drives;
    for comp in &comps[1..] {
        let mut next = Vec::new();
        for node in &nodes {
            next.extend(child_nodes_matching(node, comp));
        }
        nodes = next;
        if nodes.is_empty() {
            break;
        }
    }
    nodes
}

/// 节点下名字匹配 comp 的全部子节点
fn child_nodes_matching(parent: &str, comp: &str) -> Vec<String> {
    let Some(key) = open_key(parent) else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for (name, data) in enum_values(key) {
        if name.chars().all(|c| c.is_ascii_digit()) && blob_matches(&data, comp, false) {
            hits.push(format!(r"{parent}\{name}"));
        }
    }
    unsafe { let _ = RegCloseKey(key); };
    hits
}

/// BagMRU 子树先序遍历（数字子键），带深度与访问上限护栏
fn walk_tree(node: &str, depth: u32, visit: &mut dyn FnMut(&str, u32)) {
    const MAX_DEPTH: u32 = 8;
    visit(node, depth);
    if depth >= MAX_DEPTH {
        return;
    }
    let Some(key) = open_key(node) else { return };
    let mut kids: Vec<String> = Vec::new();
    for (name, _data) in enum_values(key) {
        if name.chars().all(|c| c.is_ascii_digit()) {
            kids.push(name);
        }
    }
    unsafe { let _ = RegCloseKey(key); };
    for kid in kids {
        walk_tree(&format!(r"{node}\{kid}"), depth + 1, visit);
    }
}

/// 读 Explorer 对该文件夹记忆的排序；无记忆/不认识键 → None
pub fn folder_sort(path: &str) -> Option<FolderSortInfo> {
    for node in bag_nodes_for(path) {
        let Some(slot) = node_slot(&node) else {
            continue;
        };
        if let Some(info) = bag_sort(slot) {
            return Some(info);
        }
    }
    None
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

/// 调试探针：打印 BagMRU walk 过程与 bag 内容（诊断用）
pub fn debug_folder_sort(path: &str) -> String {
    let mut log = String::new();
    let Some(comps) = path_components(path) else {
        return "no drive".into();
    };
    log.push_str(&format!("comps={comps:?}\n"));
    let nodes = bag_nodes_for(path);
    if nodes.is_empty() {
        log.push_str("node NOT FOUND\n");
        return log;
    }
    for node in nodes {
        log.push_str(&format!("node={node}\n"));
        let slot = node_slot(&node);
        log.push_str(&format!("slot={slot:?}\n"));
        if let Some(slot) = slot {
            log.push_str(&format!("  sort={:?}\n", bag_sort(slot)));
        }
    }
    log
}

// ── Tauri 命令 ─────────────────────────────

#[tauri::command]
pub fn get_folder_sort(path: String) -> Option<FolderSortInfo> {
    folder_sort(&path)
}

#[tauri::command]
pub fn get_type_name(ext: String) -> String {
    type_name_for_ext(&ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机 shellbag 实拍样本（bag-dump3.ps1 抓取）：
    /// [分组键 GUID0/pid1] + [排序键 shell/pid10(ItemNameDisplay) dir=1]
    const SAMPLE_NAME_ASC: &[u8] = &[
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00,
        0x30, 0xF1, 0x25, 0xB7, 0xEF, 0x47, 0x1A, 0x10, 0xA5, 0xF1, 0x02, 0x60, 0x8C, 0x9E, 0xEB, 0xAC,
        0x0A, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn parses_name_ascending() {
        let s = parse_sort_blob(SAMPLE_NAME_ASC).expect("解析失败");
        assert_eq!(s.key, "name");
        assert_eq!(s.dir, 1);
    }

    #[test]
    fn rejects_short_blob_and_unknown_key() {
        assert!(parse_sort_blob(&[0u8; 8]).is_none());
        // 全零 blob（分组键后无排序列）→ None
        assert!(parse_sort_blob(&[0u8; 20]).is_none());
        // 方向非法（0）→ 跳过 → None
        let mut bad = SAMPLE_NAME_ASC.to_vec();
        let n = bad.len();
        bad[n - 4..].copy_from_slice(&0u32.to_le_bytes());
        assert!(parse_sort_blob(&bad).is_none());
    }

    #[test]
    fn item_names_extracts_cjk() {
        // 二进制垃圾前缀 + "视频素材"
        let mut blob = vec![0x31, 0x00, 0x5E, 0x00, 0x00, 0x00];
        let name: Vec<u16> = "视频素材".encode_utf16().collect();
        for u in name {
            blob.extend_from_slice(&u.to_le_bytes());
        }
        blob.push(0);
        let names = item_names(&blob);
        assert!(names.iter().any(|n| n.contains("视频素材")), "got {names:?}");
    }

    #[test]
    fn type_names_are_shell_agreed() {
        // 与 Explorer"类型"列一致（本机 zh-CN）
        let jpg = type_name_for_ext("jpg");
        assert!(!jpg.is_empty());
        assert_ne!(jpg, "jpg");
    }

    /// 探针：真实路径端到端（bag walk + Sort 解析；cargo test -- --ignored --nocapture）
    #[test]
    #[ignore]
    fn probe_root_enum() {
        for p in [r"C:\Windows", r"C:\Users\GA\Desktop", r"D:\PSD缩略图测试"] {
            println!("{p} => {:?}", folder_sort(p));
        }
        // 找带 mtime 记忆的文件夹（sweep 曾见 slot=15 mtime desc 在 Public 链）
        for p in [
            r"C:\Users\Public\Documents",
            r"C:\Users\Public\Downloads",
            r"C:\Users\Public\Pictures",
            r"C:\Users\Public\Music",
            r"C:\Users\Public\Videos",
            r"C:\ProgramData\Microsoft\Windows\Start Menu",
        ] {
            println!("{p} => {:?}", folder_sort(p));
        }
    }

    /// 全树扫描：所有带显式排序记忆的文件夹（cargo test -- --ignored --nocapture）
    #[test]
    #[ignore]
    fn probe_tree_sweep() {
        let mut visited = 0usize;
        let mut hits = 0usize;
        walk_tree(BAGMRU_ROOT, 0, &mut |node, depth| {
            visited += 1;
            if depth > 5 {
                return;
            }
            let Some(slot) = node_slot(node) else { return };
            if let Some(info) = bag_sort(slot) {
                hits += 1;
                println!("{node}\n  slot={slot} sort={:?} dir={}", info.key, info.dir);
            }
        });
        println!("visited={visited} sortMemory={hits}");
    }

    /// 本机实测：BagMRU 树查找 + 类型描述串（真实注册表；cargo test -- --ignored --nocapture）
    #[test]
    #[ignore]
    fn live_bagmru_walk_and_typenames() {
        for ext in ["jpg", "png", "psd", "psb", "dng", "nef", "mp4", "svg"] {
            println!("{ext} => {}", type_name_for_ext(ext));
        }
        for p in [
            r"C:\Users\GA\Desktop\PSD缩略图测试",
            r"D:\PSD缩略图测试",
            r"C:\Windows",
            r"C:\Users\GA\Desktop",
        ] {
            println!("=== {p} ===\n{}", debug_folder_sort(p));
        }
    }
}
