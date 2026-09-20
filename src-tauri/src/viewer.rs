//! 大图查看器服务（doc/03-架构设计 §3.3、§4）
//!
//! image://pixlens/?src=<URL编码路径>&maxDim=<0=全尺寸 | 目标最长边> &v=<mtime>
//!
//! LOD 策略：
//! - 预览（maxDim>0）：Rust 解码 → 降采样（贴合屏幕）→ JPEG q88 → 磁盘缓存（v_ 前缀）
//! - 全尺寸（maxDim=0）：
//!   - 浏览器可原生解码的格式（JPG/PNG/WebP/BMP/ICO/AVIF/SVG/GIF）→ 原始字节透传，
//!     由 WebView 一次性全量解码（Canvas 只做变换，不损失质量、不重复编解码）
//!   - TIFF → Rust 全量解码 → JPEG q92（浏览器不支持 TIFF）
//!   - PSD/PSB → 占位（M3 实现自研解析器）
//! 预览响应携带 X-PixLens-Natural 头（原始宽高），供前端计算 100% 缩放与 LOD 升级阈值。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::codecs::jpeg::JpegEncoder;
use percent_encoding::percent_decode_str;

use crate::thumb::{self, ThumbState};

const PREVIEW_JPEG_QUALITY: u8 = 88;
const FULL_JPEG_QUALITY: u8 = 92;

pub struct FrameOutput {
    pub bytes: Vec<u8>,
    pub mime: String,
    /// 原始尺寸（Rust 解码路径必填，透传路径为 None——由浏览器给出 naturalWidth）
    pub natural: Option<(u32, u32)>,
    /// 多页文件总页数（仅 TIFF）
    pub pages: Option<u32>,
}

pub fn handle(state: &ThumbState, uri: &tauri::http::Uri) -> FrameOutput {
    let Some(req) = parse_query(uri) else {
        return placeholder_frame(None, None);
    };
    let (src, max_dim, page, exposure) = req;
    let t0 = Instant::now();
    let out = match get_frame(state, &src, max_dim, page, exposure) {
        Ok(out) => out,
        Err(e) => {
            println!("[viewer] 失败 {} : {e}", short_name(&src));
            placeholder_frame(None, None)
        }
    };
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    if ms > 100.0 || cfg!(debug_assertions) {
        println!(
            "[perf] image {} maxDim={} p{} e{:.2} → {} B in {:.1} ms",
            short_name(&src),
            max_dim,
            page,
            exposure,
            out.bytes.len(),
            ms
        );
    }
    out
}

fn short_name(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.chars().take(40).collect())
}

/// (src, maxDim, page, exposure)
fn parse_query(uri: &tauri::http::Uri) -> Option<(String, u32, u32, f32)> {
    let q = uri.query()?;
    let mut src: Option<String> = None;
    let mut max_dim: u32 = 0;
    let mut page: u32 = 0;
    let mut exposure: f32 = 1.0;
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=')?;
        match k {
            "src" => src = Some(percent_decode_str(v).decode_utf8().ok()?.into_owned()),
            "maxDim" => max_dim = v.parse().unwrap_or(0),
            "page" => page = v.parse().unwrap_or(0),
            "e" => exposure = v.parse().unwrap_or(1.0),
            _ => {}
        }
    }
    let exposure = if exposure.is_finite() { exposure.clamp(0.05, 16.0) } else { 1.0 };
    Some((src?, if max_dim == 0 { 0 } else { max_dim.clamp(256, 16384) }, page, exposure))
}

fn mime_of(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "tif" | "tiff" | "psd" | "psb" => "image/jpeg", // 经 Rust 转 JPEG
        _ => "application/octet-stream",
    }
}

fn passthrough(path: &Path, ext: &str, natural: Option<(u32, u32)>) -> std::io::Result<FrameOutput> {
    Ok(FrameOutput {
        bytes: fs::read(path)?,
        mime: mime_of(ext).into(),
        natural,
        pages: None,
    })
}

fn get_frame(
    state: &ThumbState,
    src: &str,
    max_dim: u32,
    page: u32,
    exposure: f32,
) -> std::io::Result<FrameOutput> {
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

    // GIF（动图播放走 <img> 叠加层）与 AVIF/SVG：预览与全尺寸均透传
    if matches!(ext.as_str(), "gif" | "avif" | "svg") {
        return passthrough(&path, &ext, None);
    }

    // Radiance HDR：解码 + 色调映射（曝光参数）
    if ext == "hdr" {
        return hdr_frame(state, &path, max_dim, exposure);
    }

    // 多页 TIFF：指定页解码（页数经响应头返回前端）
    if ext == "tif" || ext == "tiff" {
        return tiff_frame(state, &path, max_dim, page, exposure);
    }

    // 全尺寸：浏览器原生格式透传；PSD/PSB 走自研解析器
    if max_dim == 0 {
        return match ext.as_str() {
            "jpg" | "jpeg" | "png" | "webp" | "bmp" | "ico" => passthrough(&path, &ext, None),
            "psd" | "psb" => psd_frame(state, &path, 0, exposure),
            _ => Ok(placeholder_frame(None, None)),
        };
    }

    // 预览：磁盘缓存命中 → 只读文件头取原始尺寸后直接返回
    let cache = state.cache_dir.join(format!(
        "v{}_{}.jpg",
        blake3_key(&path, md.len(), mtime),
        max_dim
    ));
    if let Ok(bytes) = fs::read(&cache) {
        if let Ok((w, h)) = image::ImageReader::open(&path)
            .map_err(|e| image::ImageError::IoError(e))
            .and_then(|r| r.into_dimensions())
        {
            return Ok(FrameOutput {
                bytes,
                mime: "image/jpeg".into(),
                natural: Some((w, h)),
                pages: None,
            });
        }
    }

    if matches!(ext.as_str(), "psd" | "psb") {
        return psd_frame(state, &path, max_dim, exposure);
    }
    let _permit = state.view_sem.acquire();
    let img = thumb::decode_with_limits(&path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let (w, h) = (img.width(), img.height());
    let preview = downscale_to(img, max_dim);
    let out = encode_frame(preview, PREVIEW_JPEG_QUALITY, Some((w, h)))?;
    let _ = fs::write(&cache, &out.bytes);
    state.note_write();
    Ok(out)
}

/// 多页 TIFF 指定页：预览缓存键含页码
fn tiff_frame(
    state: &ThumbState,
    path: &Path,
    max_dim: u32,
    page: u32,
    exposure: f32,
) -> std::io::Result<FrameOutput> {
    let md = fs::metadata(path)?;
    let mtime = md
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if max_dim > 0 {
        let cache = state.cache_dir.join(format!(
            "vt{}_{}_p{}.jpg",
            blake3_key(path, md.len(), mtime),
            max_dim,
            page
        ));
        if let Ok(bytes) = fs::read(&cache) {
            // 页数轻量获取：重解码一次太贵——缓存的预览命中时页数未知，
            // 由前端在页面切换越界时兜底（decode_page 会返回明确错误）
            if let Ok((w, h)) = tiff_first_dimensions(path) {
                return Ok(FrameOutput {
                    bytes,
                    mime: "image/jpeg".into(),
                    natural: Some((w, h)),
                    pages: None,
                });
            }
        }
    }
    let _permit = state.view_sem.acquire();
    let t0 = std::time::Instant::now();
    let tp = crate::codecs::tiff_pages::decode_page(path, page, exposure)?;
    let pages = tp.pages;
    let natural = (tp.width, tp.height);
    let img = image::DynamicImage::ImageRgba8(tp.image);
    let quality = if max_dim == 0 { FULL_JPEG_QUALITY } else { PREVIEW_JPEG_QUALITY };
    let preview = if max_dim > 0 { downscale_to(img, max_dim) } else { img };
    let out = encode_frame(preview, quality, Some(natural))?;
    let out = FrameOutput { bytes: out.bytes, mime: out.mime, natural: out.natural, pages: Some(pages) };
    if max_dim > 0 {
        let cache = state.cache_dir.join(format!(
            "vt{}_{}_p{}.jpg",
            blake3_key(path, md.len(), mtime),
            max_dim,
            page
        ));
        let _ = fs::write(&cache, &out.bytes);
        state.note_write();
    }
    let _ = t0;
    Ok(out)
}

/// TIFF 首页尺寸（缓存命中路径补全 natural 用）
fn tiff_first_dimensions(path: &Path) -> std::io::Result<(u32, u32)> {
    use tiff::decoder::Decoder;
    let file = fs::File::open(path)?;
    let mut dec = Decoder::new(std::io::BufReader::new(file))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    dec.dimensions()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// Radiance HDR：解码 + 曝光色调映射（预览缓存键含曝光）
fn hdr_frame(state: &ThumbState, path: &Path, max_dim: u32, exposure: f32) -> std::io::Result<FrameOutput> {
    let md = fs::metadata(path)?;
    let mtime = md
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if max_dim > 0 {
        let cache = state.cache_dir.join(format!(
            "vh{}_{}_e{}.jpg",
            blake3_key(path, md.len(), mtime),
            max_dim,
            (exposure * 100.0).round() as i32
        ));
        if let Ok(bytes) = fs::read(&cache) {
            if let Ok((w, h)) = image::ImageReader::open(path)
                .map_err(|e| image::ImageError::IoError(e))
                .and_then(|r| r.into_dimensions())
            {
                return Ok(FrameOutput {
                    bytes,
                    mime: "image/jpeg".into(),
                    natural: Some((w, h)),
                    pages: None,
                });
            }
        }
    }
    let _permit = state.view_sem.acquire();
    let rgba = crate::codecs::tiff_pages::decode_hdr(path, exposure)?;
    let (w, h) = rgba.dimensions();
    let img = image::DynamicImage::ImageRgba8(rgba);
    let quality = if max_dim == 0 { FULL_JPEG_QUALITY } else { PREVIEW_JPEG_QUALITY };
    let preview = if max_dim > 0 { downscale_to(img, max_dim) } else { img };
    let out = encode_frame(preview, quality, Some((w, h)))?;
    if max_dim > 0 {
        let cache = state.cache_dir.join(format!(
            "vh{}_{}_e{}.jpg",
            blake3_key(path, md.len(), mtime),
            max_dim,
            (exposure * 100.0).round() as i32
        ));
        let _ = fs::write(&cache, &out.bytes);
        state.note_write();
    }
    Ok(out)
}

/// PSD/PSB 整帧：跨步预览（max_dim>0，走磁盘缓存）或全量合成（max_dim=0，超护栏降级 8192）
fn psd_frame(state: &ThumbState, path: &Path, max_dim: u32, exposure: f32) -> std::io::Result<FrameOutput> {
    let invalid = |e: crate::codecs::psd::PsdError| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    };
    let file = fs::File::open(path)?;
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    let info = crate::codecs::psd::parse(&mmap).map_err(invalid)?;
    let mtime = file
        .metadata()?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // 曝光缓存键后缀（仅 32 位有视觉差异；为简单起见 16/8 位也纳入键——1.0 时无后缀，与旧缓存兼容）
    let ekey = if (exposure - 1.0).abs() < f32::EPSILON {
        String::new()
    } else {
        format!("_e{}", (exposure * 100.0).round() as i32)
    };
    if max_dim > 0 {
        // 预览：磁盘缓存（键含 mtime）
        let cache = state.cache_dir.join(format!(
            "v{}_{}{}.jpg",
            blake3_key(path, file.metadata()?.len(), mtime),
            max_dim,
            ekey
        ));
        if let Ok(bytes) = fs::read(&cache) {
            return Ok(FrameOutput {
                bytes,
                mime: "image/jpeg".into(),
                natural: Some((info.width, info.height)),
                pages: None,
            });
        }
    }

    let _permit = state.view_sem.acquire();
    let composite = match crate::codecs::psd::decode_composite(&mmap, &info, max_dim, exposure) {
        Ok(c) => c,
        // 巨型文件全量被护栏拒绝 → 降级为 8192 预览（金字塔分块属 v1.1）
        Err(crate::codecs::psd::PsdError::TooLarge(_)) if max_dim == 0 => {
            crate::codecs::psd::decode_composite(&mmap, &info, 8192, exposure).map_err(invalid)?
        }
        Err(e) => return Err(invalid(e)),
    };
    let natural = (composite.width, composite.height);
    let img = image::DynamicImage::ImageRgba8(composite.image);
    let quality = if max_dim == 0 { FULL_JPEG_QUALITY } else { PREVIEW_JPEG_QUALITY };
    let out = encode_frame(img, quality, Some(natural))?;
    if max_dim > 0 {
        let cache = state.cache_dir.join(format!(
            "v{}_{}{}.jpg",
            blake3_key(path, file.metadata()?.len(), mtime),
            max_dim,
            ekey
        ));
        let _ = fs::write(&cache, &out.bytes);
        state.note_write();
    }
    Ok(out)
}

fn blake3_key(path: &Path, size: u64, mtime: u64) -> String {
    let mut h = blake3::Hasher::new();
    h.update(path.to_string_lossy().as_bytes());
    h.update(b"|");
    h.update(&size.to_le_bytes());
    h.update(b"|");
    h.update(&mtime.to_le_bytes());
    h.finalize().to_hex().to_string()[..24].to_string()
}

/// 长边缩到 max_dim（不放大）
fn downscale_to(img: image::DynamicImage, max_dim: u32) -> image::DynamicImage {
    let long = img.width().max(img.height());
    if long <= max_dim {
        return img;
    }
    if img.width() >= img.height() {
        thumb::downscale(img, max_dim)
    } else {
        let h = max_dim;
        let w = ((h as u64 * img.width() as u64) / img.height().max(1) as u64).max(1) as u32;
        img.thumbnail(w, h)
    }
}

fn encode_frame(
    img: image::DynamicImage,
    quality: u8,
    natural: Option<(u32, u32)>,
) -> std::io::Result<FrameOutput> {
    let rgb = thumb::flatten_alpha(img);
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    let enc = JpegEncoder::new_with_quality(&mut buf, quality);
    rgb.write_with_encoder(enc)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(FrameOutput { bytes: buf, mime: "image/jpeg".into(), natural, pages: None })
}

fn placeholder_frame(natural: Option<(u32, u32)>, pages: Option<u32>) -> FrameOutput {
    let mut img = image::RgbImage::from_pixel(512, 512, image::Rgb([58, 58, 66]));
    for p in img.pixels_mut() {
        p.0 = [p.0[0] / 2 + 29, p.0[1] / 2 + 29, p.0[2] / 2 + 33];
    }
    let mut buf: Vec<u8> = Vec::new();
    let enc = JpegEncoder::new_with_quality(&mut buf, 70);
    let _ = img.write_with_encoder(enc);
    FrameOutput { bytes: buf, mime: "image/jpeg".into(), natural, pages: None }
}
