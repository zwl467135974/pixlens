//! 多页 TIFF 解码（doc/01 F3：多页 TIFF 翻页；doc/02 §4 预案：直接用 tiff crate）
//!
//! image crate 只解码首帧；此处逐 IFD 迭代实现页计数与指定页解码，
//! 支持 8/16 位整数（>>8 归一化）与 32 位浮点（色调映射 + 曝光）。

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use tiff::decoder::{DecodingResult, Decoder};
use tiff::ColorType;

pub struct TiffPage {
    pub image: image::RgbaImage,
    /// 该页像素尺寸
    pub width: u32,
    pub height: u32,
    /// 文件总页数（IFD 数）
    pub pages: u32,
}

/// 解码指定页（0 基）。page 超界时返回错误。
pub fn decode_page(path: &Path, page: u32, exposure: f32) -> std::io::Result<TiffPage> {
    let file = File::open(path)?;
    let mut dec = Decoder::new(BufReader::new(file))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    let mut current: u32 = 0;
    loop {
        if current == page {
            let (image, width, height) = decode_current(&mut dec, exposure)?;
            let pages = current + 1 + count_rest(&mut dec)?;
            return Ok(TiffPage { image, width, height, pages });
        }
        if !dec.more_images() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("页码超界：{page}（共 {} 页）", current + 1),
            ));
        }
        dec.next_image()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        current += 1;
    }
}

fn count_rest(dec: &mut Decoder<BufReader<File>>) -> std::io::Result<u32> {
    let mut n: u32 = 0;
    while dec.more_images() {
        dec.next_image()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        n += 1;
    }
    Ok(n)
}

type CurrentImage = (image::RgbaImage, u32, u32);

fn decode_current(
    dec: &mut Decoder<BufReader<File>>,
    exposure: f32,
) -> std::io::Result<CurrentImage> {
    let (w, h) = dec
        .dimensions()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    if w == 0 || h == 0 || w > 300_000 || h > 300_000 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "尺寸非法"));
    }
    let ct = dec
        .colortype()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let result = dec
        .read_image()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    let channels = match ct {
        ColorType::Gray(_) => 1usize,
        ColorType::GrayA(_) => 2usize,
        ColorType::RGB(_) => 3usize,
        ColorType::RGBA(_) => 4usize,
        _ => 3usize,
    };
    let n = w as usize * h as usize;
    let mut out = image::RgbaImage::new(w, h);
    let raw: &mut [u8] = &mut out;

    match result {
        DecodingResult::U8(v) => {
            if v.len() < n * channels {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "数据不足"));
            }
            for i in 0..n {
                let o = i * 4;
                let s = i * channels;
                write_px(raw, o, &v[s..], channels, |x| x);
            }
        }
        DecodingResult::U16(v) => {
            if v.len() < n * channels {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "数据不足"));
            }
            for i in 0..n {
                let o = i * 4;
                let s = i * channels;
                let conv: Vec<u8> = (0..channels).map(|c| (v[s + c] >> 8) as u8).collect();
                write_px(raw, o, &conv, channels, |x| x);
            }
        }
        DecodingResult::F32(v) => {
            if v.len() < n * channels {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "数据不足"));
            }
            // 32 位浮点：线性 → sRGB 色调映射（含曝光）
            for i in 0..n {
                let o = i * 4;
                let s = i * channels;
                let conv: Vec<u8> = (0..channels)
                    .map(|c| crate::codecs::tone_map(v[s + c], exposure))
                    .collect();
                write_px(raw, o, &conv, channels, |x| x);
            }
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "该 TIFF 采样类型暂不支持",
            ));
        }
    }
    Ok((out, w, h))
}

#[allow(clippy::too_many_arguments)]
fn write_px(raw: &mut [u8], o: usize, samples: &[u8], channels: usize, _f: fn(u8) -> u8) {
    let (r, g, b, a) = match channels {
        1 => (samples[0], samples[0], samples[0], 255u8),
        2 => (samples[0], samples[0], samples[0], samples[1]),
        3 => (samples[0], samples[1], samples[2], 255u8),
        _ => (samples[0], samples[1], samples[2], samples[3]),
    };
    raw[o] = r;
    raw[o + 1] = g;
    raw[o + 2] = b;
    raw[o + 3] = a;
}

/// Radiance HDR（.hdr）解码：f32 线性 → 色调映射
pub fn decode_hdr(path: &Path, exposure: f32) -> std::io::Result<image::RgbaImage> {
    let file = File::open(path)?;
    let dec = image::codecs::hdr::HdrDecoder::new(BufReader::new(file))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let dynimg = image::DynamicImage::from_decoder(dec)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let rgb32 = match dynimg {
        image::DynamicImage::ImageRgb32F(img) => img,
        other => {
            // HDR 解码理论上总是 Rgb32F；兜底走普通路径
            return Ok(other.to_rgba8());
        }
    };
    let (w, h) = rgb32.dimensions();
    let mut out = image::RgbaImage::new(w, h);
    for (o, p) in out.pixels_mut().zip(rgb32.pixels()) {
        *o = image::Rgba([
            crate::codecs::tone_map(p.0[0], exposure),
            crate::codecs::tone_map(p.0[1], exposure),
            crate::codecs::tone_map(p.0[2], exposure),
            255,
        ]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 多页往返：tiff 编码器写两页 → 解码器报告 2 页且第二页内容正确
    #[test]
    fn multipage_roundtrip() {
        let path = std::env::temp_dir().join("pixlens_test_2p.tif");
        let file = File::create(&path).unwrap();
        let mut enc = tiff::encoder::TiffEncoder::new(std::io::BufWriter::new(file)).unwrap();
        enc.write_image::<tiff::encoder::colortype::RGB8>(4, 4, &vec![200u8; 4 * 4 * 3]).unwrap();
        enc.write_image::<tiff::encoder::colortype::RGB8>(4, 4, &vec![50u8; 4 * 4 * 3]).unwrap();
        drop(enc);

        let p0 = decode_page(&path, 0, 1.0).unwrap();
        assert_eq!(p0.pages, 2, "应识别为 2 页");
        assert_eq!(p0.image.get_pixel(0, 0).0[0], 200);
        let p1 = decode_page(&path, 1, 1.0).unwrap();
        assert_eq!(p1.pages, 2);
        assert_eq!(p1.image.get_pixel(0, 0).0[0], 50);
        // 越界页返回错误
        assert!(decode_page(&path, 2, 1.0).is_err());
    }
}
