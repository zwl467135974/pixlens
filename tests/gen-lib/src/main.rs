//! 合成测试图库生成器（doc/04-性能要求 §3.1）
//!
//! 用法：cargo run -p gen-lib --release -- --count 10000 --out tests/out/LIB-M
//! 构成：JPG 70% / PNG 15% / GIF 8% / WebP 5% / SVG 2%，尺寸 0.5~5MP 随机，
//!       内容为渐变 + 几何形状 + 噪声（保证解码与压缩都有真实工作量）。
//! 文件名非补零递增（img_2 < img_10），用于检验自然排序。

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use image::codecs::gif::GifEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{codecs::png::CompressionType, codecs::png::FilterType};
use image::{ExtendedColorType, Frame, ImageEncoder, RgbImage};
use rayon::prelude::*;

/// xorshift64*，按文件序号播种，无 rand 依赖
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.below((hi - lo) as u64) as u32)
    }
}

/// 生成一帧：双色对角渐变 + 随机矩形/圆 + 每像素噪声
fn gen_frame(w: u32, h: u32, rng: &mut Rng) -> RgbImage {
    let c0 = [rng.range(30, 220), rng.range(30, 220), rng.range(30, 220)];
    let c1 = [rng.range(30, 220), rng.range(30, 220), rng.range(30, 220)];
    let n_shapes = rng.range(3, 10);
    // (x, y, w, h, 亮度)
    let rects: Vec<(u32, u32, u32, u32, u32)> = (0..n_shapes)
        .map(|_| {
            let sw = rng.range(w / 12, w / 3).max(8);
            let sh = rng.range(h / 12, h / 3).max(8);
            (rng.below((w - sw) as u64) as u32, rng.below((h - sh) as u64) as u32, sw, sh, rng.range(40, 215))
        })
        .collect();

    let mut img = RgbImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let t = (x + y) as f32 / (w + h) as f32;
            let mut p = [
                (c0[0] as f32 + (c1[0] as f32 - c0[0] as f32) * t) as i32,
                (c0[1] as f32 + (c1[1] as f32 - c0[1] as f32) * t) as i32,
                (c0[2] as f32 + (c1[2] as f32 - c0[2] as f32) * t) as i32,
            ];
            for &(rx, ry, rw, rh, lum) in &rects {
                if x >= rx && x < rx + rw && y >= ry && y < ry + rh {
                    p[0] = (p[0] + lum as i32) / 2;
                    p[1] = (p[1] + lum as i32 * 3 / 4) / 2;
                    p[2] = (p[2] + lum as i32 / 2) / 2;
                }
            }
            let n = (rng.next() & 0xf) as i32 - 8;
            img.put_pixel(
                x,
                y,
                image::Rgb([
                    (p[0] + n).clamp(0, 255) as u8,
                    (p[1] + n).clamp(0, 255) as u8,
                    (p[2] + n).clamp(0, 255) as u8,
                ]),
            );
        }
    }
    img
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut count: usize = 1000;
    let mut out = PathBuf::from("tests/out/LIB-S");
    let mut seed: u64 = 0x853c_49e6_748f_ea9b;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--count" => {
                count = args[i + 1].parse().expect("--count 需要数字");
                i += 1;
            }
            "--out" => {
                out = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--seed" => {
                seed = args[i + 1].parse().expect("--seed 需要数字");
                i += 1;
            }
            other => {
                eprintln!("未知参数: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    std::fs::create_dir_all(&out).expect("创建输出目录失败");
    println!("生成 {count} 张图片 → {}", out.display());

    let t0 = std::time::Instant::now();
    let ok = (0..count)
        .into_par_iter()
        .map(|idx| -> bool {
            let mut rng = Rng::new(seed ^ (idx as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let name = format!("img_{}", idx);
            let path = out.join(&name);
            let kind = rng.below(100);
            let result = if kind < 70 {
                // JPG：0.5~5MP
                let w = rng.range(800, 2800);
                let h = rng.range(600, 2000);
                let img = gen_frame(w, h, &mut rng);
                let f = BufWriter::new(File::create(path.with_extension("jpg")).expect("创建文件失败"));
                let enc = JpegEncoder::new_with_quality(f, 85);
                enc.write_image(img.as_raw(), w, h, ExtendedColorType::Rgb8)
            } else if kind < 85 {
                // PNG：控制在 1.5MP 内（ deflate 慢）
                let w = rng.range(500, 1400);
                let h = rng.range(400, 1100);
                let img = gen_frame(w, h, &mut rng);
                let f = BufWriter::new(File::create(path.with_extension("png")).expect("创建文件失败"));
                let enc = PngEncoder::new_with_quality(f, CompressionType::Fast, FilterType::Adaptive);
                enc.write_image(img.as_raw(), w, h, ExtendedColorType::Rgb8)
            } else if kind < 93 {
                // GIF：小尺寸多帧动图
                let w = rng.range(240, 480);
                let h = rng.range(200, 420);
                let frames: Vec<Frame> = (0..rng.range(3, 7))
                    .map(|_| {
                        let img = gen_frame(w, h, &mut rng);
                        let mut rgba = image::RgbaImage::new(w, h);
                        for (o, p) in rgba.pixels_mut().zip(img.pixels()) {
                            *o = image::Rgba([p.0[0], p.0[1], p.0[2], 255]);
                        }
                        Frame::new(rgba)
                    })
                    .collect();
                let f = BufWriter::new(File::create(path.with_extension("gif")).expect("创建文件失败"));
                GifEncoder::new(f).encode_frames(frames)
            } else if kind < 98 {
                // WebP：控制在 1.2MP 内（当前 image crate 仅提供无损编码器）
                let w = rng.range(600, 1300);
                let h = rng.range(500, 950);
                let img = gen_frame(w, h, &mut rng);
                let f = BufWriter::new(File::create(path.with_extension("webp")).expect("创建文件失败"));
                let enc = WebPEncoder::new_lossless(f);
                enc.write_image(img.as_raw(), w, h, ExtendedColorType::Rgb8)
            } else {
                // SVG：透传路径用
                let hue = rng.below(360);
                let svg = format!(
                    "<svg xmlns='http://www.w3.org/2000/svg' width='{}' height='{}'>\
                     <defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'>\
                     <stop offset='0' stop-color='hsl({},70%,55%)'/>\
                     <stop offset='1' stop-color='hsl({},70%,35%)'/></linearGradient></defs>\
                     <rect width='100%' height='100%' fill='url(#g)'/>\
                     <circle cx='{}' cy='{}' r='{}' fill='hsl({},80%,70%)' opacity='0.8'/>\
                     </svg>",
                    w_svg(&mut rng), h_svg(&mut rng),
                    hue,
                    (hue + 60) % 360,
                    rng.range(100, 400),
                    rng.range(100, 300),
                    rng.range(50, 180),
                    (hue + 180) % 360,
                );
                std::fs::write(path.with_extension("svg"), svg)
                    .map_err(image::ImageError::IoError)
            };
            result.is_ok()
        })
        .filter(|&ok| ok)
        .count();

    let total_bytes: u64 = std::fs::read_dir(&out)
        .map(|rd| rd.flatten().filter_map(|e| e.metadata().ok().map(|m| m.len())).sum())
        .unwrap_or(0);
    println!(
        "完成：{ok}/{count} 张，共 {:.2} GB，耗时 {:.1}s",
        total_bytes as f64 / 1e9,
        t0.elapsed().as_secs_f64()
    );
}

fn w_svg(rng: &mut Rng) -> u32 {
    rng.range(400, 1200)
}
fn h_svg(rng: &mut Rng) -> u32 {
    rng.range(300, 900)
}
