//! 视频首帧提取（Media Foundation，doc/03-架构设计 §6.3）
//!
//! mp4/mov/webm/mkv/avi/wmv → 代表帧（优先 1s 处）→ RGBA。
//! 只依赖系统解码器：H.264/VP9 等原生支持，HEVC 取决于系统是否装了扩展
//! （未装则本模块报错，缩略图回落占位块，不影响其他格式）。

use std::path::Path;

use image::RgbaImage;
use windows::core::{GUID, HSTRING};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VT_I8;

/// 解码帧最大像素数（8K 护栏；超限拒绝，防御异常流）
const MAX_PIXELS: u64 = 40_000_000;

/// 提取一帧 RGBA。优先取 1s 处的代表帧（首帧常是黑屏），不支持 seek 则退回首帧。
pub fn extract_frame(path: &Path) -> Result<RgbaImage, String> {
    unsafe {
        // 协议回调每次新线程：COM 按 MTA 初始化；已被初始化（含 STA）时失败可忽略
        let coinited = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        mf_startup_once();

        let result = extract_inner(path);

        if coinited {
            CoUninitialize();
        }
        // MFStartup 只进不出：进程退出统一回收，避免线程竞态卸载
        result
    }
}

unsafe fn extract_inner(path: &Path) -> Result<RgbaImage, String> {
    let url = HSTRING::from(path.as_os_str());
    // 启用内置 video processor：源阅读器才能输出 RGB32（解码器原生多为 YUV）
    let mut attrs_opt: Option<IMFAttributes> = None;
    MFCreateAttributes(&mut attrs_opt, 1).map_err(|e| format!("创建属性表失败: {e}"))?;
    let attrs = attrs_opt.ok_or("属性表为空")?;
    attrs
        .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
        .map_err(|e| format!("启用视频处理失败: {e}"))?;
    let reader = MFCreateSourceReaderFromURL(&url, Some(&attrs))
        .map_err(|e| format!("打开视频失败: {e}"))?;
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    reader.SetStreamSelection(stream, true).map_err(|e| format!("选择视频流失败: {e}"))?;
    // 关音频：省一路解码，无音频流时忽略
    let _ = reader.SetStreamSelection(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, false);

    let native = reader.GetCurrentMediaType(stream).map_err(|e| format!("读流格式失败: {e}"))?;
    let frame_size = native
        .GetUINT64(&MF_MT_FRAME_SIZE)
        .map_err(|e| format!("读分辨率失败: {e}"))?;
    let w = (frame_size >> 32) as u32;
    let h = (frame_size & 0xFFFF_FFFF) as u32;
    if w == 0 || h == 0 || (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(format!("异常分辨率 {w}x{h}"));
    }

    // 目标格式 RGB32（老式 video processing 模式下旋转/镜像会被自动应用）
    let mt = MFCreateMediaType().map_err(|e| format!("创建媒体类型失败: {e}"))?;
    mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
        .map_err(|e| format!("设置主类型失败: {e}"))?;
    mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
        .map_err(|e| format!("设置像素格式失败: {e}"))?;
    reader
        .SetCurrentMediaType(stream, None, &mt)
        .map_err(|e| format!("切换 RGB32 失败: {e}"))?;
    // 转换后类型：已应用旋转则宽高对调、剩余旋转量为 0——以它为准
    let current = reader.GetCurrentMediaType(stream).map_err(|e| format!("读输出格式失败: {e}"))?;
    let out_size = current.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(frame_size);
    let ow = (out_size >> 32) as u32;
    let oh = (out_size & 0xFFFF_FFFF) as u32;
    let rotation = current.GetUINT32(&MF_MT_VIDEO_ROTATION).unwrap_or(0);

    // seek 到 1s（100ns 单位）；不可 seek 的容器忽略，从首帧读
    let mut pos = PROPVARIANT::default();
    (*pos.Anonymous.Anonymous).vt = VT_I8;
    (*pos.Anonymous.Anonymous).Anonymous.hVal = 10_000_000_i64;
    let _ = reader.SetCurrentPosition(&GUID::zeroed(), &pos);

    for _ in 0..240 {
        let mut actual: u32 = 0;
        let mut flags: u32 = 0;
        let mut ts: i64 = 0;
        let mut sample: Option<IMFSample> = None;
        reader
            .ReadSample(stream, 0, Some(&mut actual), Some(&mut flags), Some(&mut ts), Some(&mut sample))
            .map_err(|e| format!("读帧失败: {e}"))?;
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Err("视频流为空".to_string());
        }
        // seek 缝隙处可能给 STREAMTICK（sample=None），跳过等下一包
        let Some(sample) = sample else { continue };
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|e| format!("取缓冲失败: {e}"))?;
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut max_len = 0u32;
        let mut cur_len = 0u32;
        buffer
            .Lock(&mut ptr, Some(&mut max_len), Some(&mut cur_len))
            .map_err(|e| format!("锁缓冲失败: {e}"))?;
        let copied = (|| {
            let stride = stride_of(&reader, stream, ow);
            let bytes = std::slice::from_raw_parts(ptr, cur_len as usize);
            let img = bgra_to_rgba(bytes, ow, oh, stride);
            rotate(img, rotation)
        })();
        buffer.Unlock().map_err(|e| format!("解锁失败: {e}"))?;
        return Ok(copied);
    }
    Err("读帧次数超限".to_string())
}

/// 进程内首次调用时初始化 MF（幂等）
fn mf_startup_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let _ = MFStartup(MF_VERSION, MFSTARTUP_LITE);
    });
}

unsafe fn stride_of(reader: &IMFSourceReader, stream: u32, w: u32) -> i32 {
    reader
        .GetCurrentMediaType(stream)
        .ok()
        .and_then(|mt| mt.GetUINT32(&MF_MT_DEFAULT_STRIDE).ok())
        .map(|v| v as i32)
        // 绝大多数 RGB32 转换输出为正值（top-down）
        .unwrap_or((w as i32) * 4)
}

/// MF 的 RGB32 实为 BGRA 字节序；负 stride 表示 bottom-up 行序，需翻转
fn bgra_to_rgba(bytes: &[u8], w: u32, h: u32, stride: i32) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    let row_len = (w as usize) * 4;
    let raw = img.as_mut();
    for y in 0..h as usize {
        let src_off = if stride >= 0 {
            y * stride as usize
        } else {
            (h as usize - 1 - y) * (-(stride) as usize)
        };
        let row = &bytes[src_off..src_off + row_len];
        let dst = &mut raw[y * row_len..(y + 1) * row_len];
        for (d, chunk) in dst.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
            d[0] = chunk[2];
            d[1] = chunk[1];
            d[2] = chunk[0];
            d[3] = 0xFF;
        }
    }
    img
}

/// 竖拍视频的旋转元数据（MFVideoRotationFormat：0/90/180/270 度）落到像素上
fn rotate(img: RgbaImage, rotation: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    match rotation {
        90 => {
            let mut out = RgbaImage::new(h, w);
            for (x, y, p) in img.enumerate_pixels() {
                out.put_pixel(h - 1 - y, x, *p);
            }
            out
        }
        180 => {
            let mut out = RgbaImage::new(w, h);
            for (x, y, p) in img.enumerate_pixels() {
                out.put_pixel(w - 1 - x, h - 1 - y, *p);
            }
            out
        }
        270 => {
            let mut out = RgbaImage::new(h, w);
            for (x, y, p) in img.enumerate_pixels() {
                out.put_pixel(y, w - 1 - x, *p);
            }
            out
        }
        _ => img,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_stride_neg_flips_rows() {
        // 2x2，负 stride：最后一行数据在前
        let bytes: Vec<u8> = vec![
            0, 0, 0xFF, 0, 0, 0, 0xFF, 0, // 红（bottom 行）
            0, 0xFF, 0, 0, 0, 0xFF, 0, 0, // 绿
        ];
        let img = bgra_to_rgba(&bytes, 2, 2, -8);
        assert_eq!(img.get_pixel(0, 0)[1], 0xFF); // 顶行应为绿
        assert_eq!(img.get_pixel(0, 1)[0], 0xFF); // 底行应为红
    }

    #[test]
    fn bgra_swaps_r_b() {
        let bytes: Vec<u8> = vec![0, 0, 0xFF, 0];
        let img = bgra_to_rgba(&bytes, 1, 1, 4);
        assert_eq!(img.get_pixel(0, 0)[0], 0xFF); // BGRA 蓝 → RGBA 红
        assert_eq!(img.get_pixel(0, 0)[2], 0);
        assert_eq!(img.get_pixel(0, 0)[3], 0xFF);
    }

    #[test]
    fn rotate_90_moves_origin() {
        let mut img = RgbaImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgba([10, 0, 0, 255]));
        let out = rotate(img, 90);
        assert_eq!(out.dimensions(), (1, 2));
        // 顺时针 90°：1 行变 1 列，首像素仍在原点
        assert_eq!(out.get_pixel(0, 0)[0], 10);
    }

    /// 真实 mp4 代表帧提取（需本机 MF 运行时与桌面下任意 mp4 样本，本地手动跑）
    #[test]
    #[ignore]
    fn extracts_real_mp4() {
        let dir = std::path::Path::new(r"C:\Users\GA\Desktop");
        let target = walkdir::WalkDir::new(dir)
            .min_depth(1)
            .max_depth(2)
            .into_iter()
            .flatten()
            .find(|e| e.path().extension().map(|x| x == "mp4").unwrap_or(false))
            .map(|e| e.into_path())
            .expect("桌面下无 mp4 样本");
        let img = extract_frame(&target).expect("提取失败");
        println!("{}x{} 代表帧", img.width(), img.height());
        assert!(img.width() > 0 && img.height() > 0);
        img.save(r"C:\Users\GA\AppData\Local\Temp\pixlens_video_frame.png").unwrap();
    }
}
