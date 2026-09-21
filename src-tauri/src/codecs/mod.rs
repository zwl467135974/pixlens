//! 图像编解码器：格式嗅探与分发（doc/03-架构设计 §7）
//!
//! PSD/PSB 解析器已拆分为独立 crate `psd-codec`（应用与 Shell 缩略图扩展共用）。

pub use psd_codec as psd;
pub use psd_codec::tone_map;

pub mod tiff_pages;
