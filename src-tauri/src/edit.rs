//! 基础编辑（doc/01 F5、doc/03 §3.5）
//!
//! 管线：解码 → 裁剪（原图坐标 ROI）→ 旋转 → 翻转 → 色彩调节 → 滤镜 → 编码落盘。
//! 色彩调节公式与前端 CSS filter 完全一致（brightness→contrast→saturate 链序），
//! 保证"实时预览 = 保存成品"。保存策略：默认另存副本（{原名}_edited.(n).ext），可选覆盖。

use std::path::{Path, PathBuf};

use image::GenericImageView;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct EditOps {
    pub crop: Option<CropRect>,
    /// 顺时针 0/90/180/270
    pub rotate: u32,
    pub flip_h: bool,
    pub flip_v: bool,
    /// -100..100（与 CSS filter 系数一致：factor = 1 + v/100）
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    /// none | gray | sepia | invert
    pub filter: String,
}

impl Default for EditOps {
    fn default() -> Self {
        Self {
            crop: None,
            rotate: 0,
            flip_h: false,
            flip_v: false,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            filter: "none".into(),
        }
    }
}

fn clamp255(v: f32) -> u8 {
    v.clamp(0.0, 255.0).round() as u8
}

/// 色彩调节 + 滤镜 —— 与前端 CSS filter 逐公式一致
pub fn apply_color(img: &mut image::RgbaImage, ops: &EditOps) {
    let fb = 1.0 + ops.brightness / 100.0;
    let fc = 1.0 + ops.contrast / 100.0;
    let fs = 1.0 + ops.saturation / 100.0;
    let filter = ops.filter.as_str();
    for p in img.pixels_mut() {
        // brightness（乘法）
        let (mut r, mut g, mut b) = (p.0[0] as f32 * fb, p.0[1] as f32 * fb, p.0[2] as f32 * fb);
        // contrast：(v - 127.5)·fc + 127.5（CSS 在 [0,1] 域等价）
        r = (r - 127.5) * fc + 127.5;
        g = (g - 127.5) * fc + 127.5;
        b = (b - 127.5) * fc + 127.5;
        // saturate：Rec.709 亮度系数
        let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        r = l + (r - l) * fs;
        g = l + (g - l) * fs;
        b = l + (b - l) * fs;
        match filter {
            "gray" => {
                let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                r = l;
                g = l;
                b = l;
            }
            "sepia" => {
                let (ri, gi, bi) = (r, g, b);
                r = 0.393 * ri + 0.769 * gi + 0.189 * bi;
                g = 0.349 * ri + 0.686 * gi + 0.168 * bi;
                b = 0.272 * ri + 0.534 * gi + 0.131 * bi;
            }
            "invert" => {
                r = 255.0 - r;
                g = 255.0 - g;
                b = 255.0 - b;
            }
            _ => {}
        }
        p.0 = [clamp255(r), clamp255(g), clamp255(b), p.0[3]];
    }
}

/// 几何变换：裁剪 → 旋转 → 翻转（顺序与前端视图变换一致）
pub fn apply_geometry(mut img: image::DynamicImage, ops: &EditOps) -> Result<image::DynamicImage, String> {
    if let Some(c) = ops.crop {
        let (w, h) = img.dimensions();
        if c.x >= w || c.y >= h || c.w == 0 || c.h == 0 || c.x.saturating_add(c.w) > w || c.y.saturating_add(c.h) > h {
            return Err(format!("裁剪区越界（{}×{} 内 {}+{},{}+{}）", w, h, c.x, c.w, c.y, c.h));
        }
        img = img.crop_imm(c.x, c.y, c.w, c.h);
    }
    img = match ops.rotate % 360 {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => img,
    };
    if ops.flip_h {
        img = img.fliph();
    }
    if ops.flip_v {
        img = img.flipv();
    }
    Ok(img)
}

fn decode_for_edit(path: &Path) -> Result<image::DynamicImage, String> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "psd" | "psb" => {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| e.to_string())?;
            let info = crate::codecs::psd::parse(&mmap).map_err(|e| e.to_string())?;
            let c = crate::codecs::psd::decode_composite(&mmap, &info, 0, 1.0)
                .or_else(|_| crate::codecs::psd::decode_composite(&mmap, &info, 8192, 1.0))
                .map_err(|e| e.to_string())?;
            Ok(image::DynamicImage::ImageRgba8(c.image))
        }
        "hdr" => {
            let rgba = crate::codecs::tiff_pages::decode_hdr(path, 1.0).map_err(|e| e.to_string())?;
            Ok(image::DynamicImage::ImageRgba8(rgba))
        }
        "avif" | "svg" => Err("该格式暂不支持编辑".into()),
        _ => crate::thumb::decode_with_limits(path).map_err(|e| format!("解码失败: {e}")),
    }
}

fn encode_edit(img: image::DynamicImage, out_ext: &str, quality: u8) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut buf = Vec::new();
    match out_ext {
        "png" => {
            let rgba = img.to_rgba8();
            let enc = image::codecs::png::PngEncoder::new_with_quality(
                &mut buf,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::Adaptive,
            );
            enc.write_image(rgba.as_raw(), rgba.width(), rgba.height(), image::ExtendedColorType::Rgba8)
                .map_err(|e| e.to_string())?;
        }
        "webp" => {
            let rgba = img.to_rgba8();
            let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut buf);
            enc.write_image(rgba.as_raw(), rgba.width(), rgba.height(), image::ExtendedColorType::Rgba8)
                .map_err(|e| e.to_string())?;
        }
        _ => {
            let rgb = img.to_rgb8();
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100));
            enc.write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(buf)
}

/// 编辑落盘：返回输出路径
#[tauri::command]
pub fn edit_apply(src: String, ops: EditOps, overwrite: bool, quality: u8) -> Result<String, String> {
    let path = PathBuf::from(&src);
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let out_ext = match ext.as_str() {
        "jpg" | "jpeg" => "jpg",
        "png" => "png",
        "webp" => "webp",
        _ => "jpg", // 其余格式（bmp/tif/gif/psd/hdr…）编辑结果统一存 JPG
    };

    let img = decode_for_edit(&path)?;
    let img = apply_geometry(img, &ops)?;
    let mut rgba = img.to_rgba8();
    apply_color(&mut rgba, &ops);
    let final_img = image::DynamicImage::ImageRgba8(rgba);
    let bytes = encode_edit(final_img, out_ext, quality)?;

    let out = if overwrite {
        path.with_extension(out_ext)
    } else {
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        crate::batch::unique_path(dir.join(format!("{stem}_edited.{out_ext}")))
    };
    std::fs::write(&out, bytes).map_err(|e| format!("写入失败: {e}"))?;
    Ok(out.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_math_matches_css() {
        // brightness +10：128×1.1 = 140.8 → 141
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([128, 128, 128, 255]));
        apply_color(
            &mut img,
            &EditOps { brightness: 10.0, ..Default::default() },
        );
        assert_eq!(img.get_pixel(0, 0).0, [141, 141, 141, 255]);

        // contrast +100（×2）：(128-127.5)×2+127.5 = 128.5 → 129... 先乘后比：
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([200, 100, 50, 255]));
        apply_color(&mut img, &EditOps { contrast: 100.0, ..Default::default() });
        // (200-127.5)*2+127.5 = 272.5 → 255；(100-127.5)*2+127.5 = 72.5 → 73；(50-127.5)*2+127.5=-27.5 → 0
        assert_eq!(img.get_pixel(0, 0).0, [255, 73, 0, 255]);

        // saturate -100（全灰）：l(200,100,50) = 117.65 → 118
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([200, 100, 50, 255]));
        apply_color(&mut img, &EditOps { saturation: -100.0, ..Default::default() });
        assert_eq!(img.get_pixel(0, 0).0, [118, 118, 118, 255]);

        // sepia(100,100,100)：(135,120,94)
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([100, 100, 100, 255]));
        apply_color(&mut img, &EditOps { filter: "sepia".into(), ..Default::default() });
        assert_eq!(img.get_pixel(0, 0).0, [135, 120, 94, 255]);

        // invert
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]));
        apply_color(&mut img, &EditOps { filter: "invert".into(), ..Default::default() });
        assert_eq!(img.get_pixel(0, 0).0, [245, 235, 225, 255]);

        // alpha 保留
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 77]));
        apply_color(&mut img, &EditOps { filter: "invert".into(), ..Default::default() });
        assert_eq!(img.get_pixel(0, 0).0[3], 77);
    }

    #[test]
    fn geometry_crop_rotate_flip() {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(4, 3, image::Rgba([1, 2, 3, 255])));
        let ops = EditOps { crop: Some(CropRect { x: 1, y: 0, w: 2, h: 3 }), rotate: 90, ..Default::default() };
        let out = apply_geometry(img, &ops).unwrap();
        assert_eq!(out.dimensions(), (3, 2), "2×3 顺时针 90° → 3×2");

        // 越界裁剪被拒绝
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::new(4, 3));
        let ops = EditOps { crop: Some(CropRect { x: 3, y: 0, w: 5, h: 3 }), ..Default::default() };
        assert!(apply_geometry(img, &ops).is_err());
    }

    #[test]
    fn edit_apply_preserves_original() {
        let dir = std::env::temp_dir().join("pixlens_edit_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("e.png");
        let img = image::RgbImage::from_pixel(32, 24, image::Rgb([120, 130, 140]));
        img.save_with_format(&src, image::ImageFormat::Png).unwrap();
        let before = std::fs::read(&src).unwrap();

        let ops = EditOps { brightness: 20.0, rotate: 180, filter: "gray".into(), ..Default::default() };
        let out = edit_apply(src.to_string_lossy().into_owned(), ops, false, 90).unwrap();
        assert!(out.contains("e_edited"), "输出应为副本: {out}");
        assert!(Path::new(&out).exists());
        assert_eq!(std::fs::read(&src).unwrap(), before, "原图字节不应变化");

        // 输出可解码且尺寸正确
        let decoded = image::ImageReader::open(&out).unwrap().decode().unwrap();
        assert_eq!(decoded.dimensions(), (32, 24));

        // 覆盖模式写原路径（PNG 源保持 PNG）
        let ops = EditOps::default();
        let out2 = edit_apply(src.to_string_lossy().into_owned(), ops, true, 90).unwrap();
        assert!(out2.replace('/', "\\").ends_with("e.png"), "覆盖输出应为 e.png: {out2}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
