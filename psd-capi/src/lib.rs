//! PSD/PSB 解码 C 接口（供 C++ Shell 缩略图扩展链接）
//!
//! 注意：刻意不依赖 image crate——实测其解码器在 COM 代理进程（DllHost）
//! 中首次执行即栈溢出；内嵌 JPEG（1036）的解码由 C++ 侧用系统 GDI+ 完成。
//! 本侧仅提供纯自研的解析/合成路径，无第三方解码器依赖。

/// 提取内嵌缩略图 JPEG（1036）原始字节。
/// 返回：0=未找到；>0=写出的字节数（截断到 out_cap）；<0=参数错
#[no_mangle]
pub extern "C" fn pixlens_psd_embedded_jpeg(
    data: *const u8,
    len: usize,
    out_jpeg: *mut u8,
    out_cap: usize,
) -> i32 {
    if data.is_null() || len == 0 {
        return -1;
    }
    let buf = unsafe { std::slice::from_raw_parts(data, len) };
    let Some(info) = psd_codec::parse(buf).ok() else {
        return 0;
    };
    match psd_codec::embedded_thumbnail(buf, &info) {
        Some(jpeg) => {
            if out_jpeg.is_null() || out_cap == 0 {
                return jpeg.len() as i32; // 查询所需大小
            }
            let n = jpeg.len().min(out_cap);
            unsafe { std::ptr::copy_nonoverlapping(jpeg.as_ptr(), out_jpeg, n) };
            n as i32
        }
        None => 0,
    }
}

/// 合成图跨步解码为 RGBA8（长边 ≤ cx），纯自研路径。
/// 返回 0 成功；负值失败。
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
}
