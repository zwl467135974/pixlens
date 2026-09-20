//! 图像编解码器：格式嗅探与分发（doc/03-架构设计 §7）

pub mod psd;
pub mod tiff_pages;

/// 线性 → sRGB 基础色调映射（带曝光系数），供 PSD32 / float TIFF / Radiance HDR 共用
pub fn tone_map(v: f32, exposure: f32) -> u8 {
    let v = (v * exposure).clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}
