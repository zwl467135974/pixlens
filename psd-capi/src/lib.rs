//! PSD/PSB 解码 C 接口（供 C++ Shell 缩略图扩展动态加载）
//!
//! 注意：刻意不依赖 image crate——实测其解码器在 COM 代理进程（DllHost）
//! 中首次执行即栈溢出；内嵌 JPEG（1036）的解码由 C++ 侧用系统 GDI+ 完成。
//! 本侧仅提供纯自研的解析/合成路径，无第三方解码器依赖。
//!
//! 大文件走 from_file（mmap 零拷贝，按需分页，GB 级 PSB 只触碰采样页，
//! 不吃代理进程内存）；stream 路径用 mem 接口（调用方已截断上限）。

use std::os::raw::c_char;
use std::panic::{self, AssertUnwindSafe};

/// C ABI 边界不得展开：panic 穿透 extern "C" 会 abort 整个 DllHost。
/// 统一兜底为负值错误码（-100），由调用方按普通失败处理。
fn guarded<F: FnOnce() -> i32>(f: F) -> i32 {
    panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or(-100)
}

/// C 侧字节源回调：seek 绝对定位（0 成功），read 读满缓冲（0 成功）
pub type CbSeek = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, pos: u64) -> i32;
pub type CbRead = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, buf: *mut u8, len: usize) -> i32;

struct StreamSource {
    ctx: *mut std::ffi::c_void,
    seek: CbSeek,
    read: CbRead,
}

// 回调只在本线程内串行使用（IStream 非线程安全，psd-codec 流式路径为串行）
unsafe impl Send for StreamSource {}

impl psd_codec::ByteSource for StreamSource {
    fn pread(&mut self, off: u64, out: &mut [u8]) -> Result<(), psd_codec::PsdError> {
        if out.is_empty() {
            return Ok(());
        }
        unsafe {
            if (self.seek)(self.ctx, off) != 0 {
                return Err(psd_codec::PsdError::Truncated);
            }
            if (self.read)(self.ctx, out.as_mut_ptr(), out.len()) != 0 {
                return Err(psd_codec::PsdError::Truncated);
            }
        }
        Ok(())
    }
}

/// 提取内嵌缩略图 JPEG（1036）原始字节（mmap 版）。
/// 返回：0=未找到；>0=写出的字节数（截断到 out_cap）；<0=失败
#[no_mangle]
pub extern "C" fn pixlens_psd_embedded_jpeg_from_file(
    path: *const c_char,
    out_jpeg: *mut u8,
    out_cap: usize,
) -> i32 {
    guarded(|| {
        let Some(p) = (unsafe { cstr(path) }) else { return -1 };
        let file = match std::fs::File::open(p) {
            Ok(f) => f,
            Err(_) => return -1,
        };
        let mm = match (unsafe { memmap2::Mmap::map(&file) }) {
            Ok(m) => m,
            Err(_) => return -2,
        };
        embedded_impl(&mm, out_jpeg, out_cap)
    })
}

/// 合成图跨步解码为 RGBA8（长边 ≤ cx，mmap 版）。0 成功；负值失败。
#[no_mangle]
pub extern "C" fn pixlens_psd_composite_rgba_from_file(
    path: *const c_char,
    cx: u32,
    out_rgba: *mut u8,
    out_cap: usize,
    out_w: *mut u32,
    out_h: *mut u32,
) -> i32 {
    guarded(|| {
        let Some(p) = (unsafe { cstr(path) }) else { return -1 };
        let file = match std::fs::File::open(p) {
            Ok(f) => f,
            Err(_) => return -1,
        };
        let mm = match (unsafe { memmap2::Mmap::map(&file) }) {
            Ok(m) => m,
            Err(_) => return -2,
        };
        let info = match psd_codec::parse(&mm) {
            Ok(i) => i,
            Err(_) => return -3,
        };
        let comp = match psd_codec::decode_composite(&mm, &info, cx, 1.0) {
            Ok(c) => c,
            Err(_) => return -4,
        };
        let (w, h) = comp.image.dimensions();
        let need = (w as usize) * (h as usize) * 4;
        if out_rgba.is_null() || out_w.is_null() || out_h.is_null() || out_cap < need {
            return -5;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(comp.image.as_raw().as_ptr(), out_rgba, need);
            *out_w = w;
            *out_h = h;
        }
        0
    })
}

/// 流式合成解码：head 为文件前部（parse_prefix 用，≥2MB 覆盖绝大多数），
/// total 为完整文件长度，行数据经 C 回调按需读取（IStream seek/read）——
/// 任意大小文件有界内存，替代"整流读入"路径。0 成功；负值失败。
#[no_mangle]
pub extern "C" fn pixlens_psd_composite_rgba_stream(
    head: *const u8,
    head_len: usize,
    total: u64,
    ctx: *mut std::ffi::c_void,
    seek: CbSeek,
    read: CbRead,
    cx: u32,
    out_rgba: *mut u8,
    out_cap: usize,
    out_w: *mut u32,
    out_h: *mut u32,
) -> i32 {
    guarded(|| {
        if head.is_null() || out_rgba.is_null() || out_w.is_null() || out_h.is_null()
            || head_len == 0 || cx == 0 || ctx.is_null() {
            return -1;
        }
        let head = unsafe { std::slice::from_raw_parts(head, head_len) };
        let info = match psd_codec::parse_prefix(head, total) {
            Ok(i) => i,
            Err(_) => return -2,
        };
        let mut src = StreamSource { ctx, seek, read };
        let comp = match psd_codec::decode_composite_source(&mut src, &info, cx, 1.0) {
            Ok(c) => c,
            Err(_) => return -3,
        };
        let (w, h) = comp.image.dimensions();
        let need = (w as usize) * (h as usize) * 4;
        if out_cap < need {
            return -4;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(comp.image.as_raw().as_ptr(), out_rgba, need);
            *out_w = w;
            *out_h = h;
        }
        0
    })
}

unsafe fn cstr(p: *const c_char) -> Option<&'static std::path::Path> {
    if p.is_null() {
        return None;
    }
    let mut len = 0usize;
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(p as *const u8, len) };
    Some(std::path::Path::new(unsafe { std::str::from_utf8_unchecked(bytes) }))
}

fn embedded_impl(data: &[u8], out_jpeg: *mut u8, out_cap: usize) -> i32 {
    let Some(info) = psd_codec::parse(data).ok() else {
        return 0;
    };
    match psd_codec::embedded_thumbnail(data, &info) {
        Some(jpeg) => {
            if out_jpeg.is_null() || out_cap == 0 {
                return jpeg.len() as i32;
            }
            let n = jpeg.len().min(out_cap);
            unsafe { std::ptr::copy_nonoverlapping(jpeg.as_ptr(), out_jpeg, n) };
            n as i32
        }
        None => 0,
    }
}

/// 内存版：提取内嵌缩略图 JPEG（C++ 壳 stream 数据路径）
#[no_mangle]
pub extern "C" fn pixlens_psd_embedded_jpeg(
    data: *const u8,
    len: usize,
    out_jpeg: *mut u8,
    out_cap: usize,
) -> i32 {
    guarded(|| {
        if data.is_null() || len == 0 {
            return -1;
        }
        let buf = unsafe { std::slice::from_raw_parts(data, len) };
        embedded_impl(buf, out_jpeg, out_cap)
    })
}

/// 内存版：合成图跨步解码（数据来自 stream；截断数据解析失败返回负值）
#[no_mangle]
pub extern "C" fn pixlens_psd_composite_rgba(
    data: *const u8,
    len: usize,
    cx: u32,
    out_rgba: *mut u8,
    out_cap: usize,
    out_w: *mut u32,
    out_h: *mut u32,
) -> i32 {
    guarded(|| {
        if data.is_null() || out_rgba.is_null() || out_w.is_null() || out_h.is_null() || len == 0 || cx == 0 {
            return -1;
        }
        let buf = unsafe { std::slice::from_raw_parts(data, len) };
        let info = match psd_codec::parse(buf) {
            Ok(i) => i,
            Err(_) => return -2,
        };
        let comp = match psd_codec::decode_composite(buf, &info, cx, 1.0) {
            Ok(c) => c,
            Err(_) => return -3,
        };
        let (w, h) = comp.image.dimensions();
        let need = (w as usize) * (h as usize) * 4;
        if out_cap < need {
            return -4;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(comp.image.as_raw().as_ptr(), out_rgba, need);
            *out_w = w;
            *out_h = h;
        }
        0
    })
}
