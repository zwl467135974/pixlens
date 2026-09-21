//! PSD/PSB 测试样本生成器（doc/04-性能要求 §3.1 LIB-PSB）
//!
//! 用法：cargo run -p pixlens --bin genpsd -- --out tests/out/LIB-PSB [--big]
//! - 小样本矩阵：PSD/PSB × Raw/RLE × RGB8/16/32、灰度8，部分带 1036 内嵌缩略图
//! - 损坏样本：坏签名 / 截断 / 超大尺寸 / 纯垃圾字节
//! - --big：两个 ~2GB PSB（30000×17000×4通道×8bit RLE 噪声），含/不含内嵌缩略图各一（P8）

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder, RgbImage};
use pixlens_lib::codecs::psd::{self, PsdChannels};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args
        .iter()
        .position(|a| a == "--out")
        .map(|i| PathBuf::from(&args[i + 1]))
        .unwrap_or_else(|| PathBuf::from("tests/out/LIB-PSB"));
    let big = args.iter().any(|a| a == "--big");
    let real = args.iter().any(|a| a == "--real");
    if let Some(i) = args.iter().position(|a| a == "--probe") {
        let data = std::fs::read(&args[i + 1]).expect("读取文件失败");
        let info = psd::parse(&data).expect("解析失败");
        println!(
            "v{} mode={} ch={} depth={} {}x{} res={}..{} img@{}",
            info.version, info.mode, info.channels, info.depth, info.width, info.height,
            info.res_start, info.res_end, info.img_start
        );
        match psd::embedded_thumbnail(&data, &info) {
            Some(j) => println!(
                "embedded: {} bytes, head: {:02X?}",
                j.len(),
                &j[..j.len().min(8)]
            ),
            None => println!("embedded: NONE"),
        }
        println!("composite(256): {:?}", psd::decode_composite(&data, &info, 256, 1.0).map(|c| (c.width, c.height)));
        return;
    }
    std::fs::create_dir_all(&out).expect("创建输出目录失败");
    let thumb = make_thumb_jpeg();

    if real {
        write_real(&out, &thumb);
        return;
    }

    // 小样本矩阵
    let mut count = 0;
    for &(version, vname) in &[(1u16, "psd"), (2u16, "psb")] {
        for &(comp, cname) in &[(0u16, "raw"), (1u16, "rle")] {
            for &(nch, dname, depth) in &[
                (3usize, "rgb8", 8u16),
                (4usize, "rgb16a", 16u16),
                (3usize, "rgb32", 32u16),
                (2usize, "gray8a", 8u16),
            ] {
                let name = format!("small_{dname}_{cname}.{vname}");
                let ch = sample_channels(97, 61, nch, depth);
                let with_thumb = comp == 1 && version == 2 && nch == 3; // 部分带内嵌缩略图
                let data = psd::encode_psd(&ch, version, comp, if with_thumb { Some(&thumb) } else { None });
                std::fs::write(out.join(&name), data).expect("写入样本失败");
                count += 1;
            }
        }
    }
    println!("小样本 {count} 个 → {}", out.display());

    write_corrupt(&out);

    if big {
        write_big(&out.join("huge_with_thumb.psb"), true, &thumb);
        write_big(&out.join("huge_plain.psb"), false, &thumb);
    }
}

fn sample_channels(w: u32, h: u32, nch: usize, depth: u16) -> PsdChannels {
    let mut s = 0x243F_6A88_85A3_08D3u64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let n = (w as usize) * (h as usize);
    let bps = depth as usize / 8;
    let mode = if nch >= 3 { 3 } else { 1 };
    let mut planes = Vec::new();
    for _ in 0..nch {
        let mut p = Vec::with_capacity(n * bps);
        for _ in 0..n {
            match depth {
                8 => p.push((next() & 0xff) as u8),
                16 => p.extend_from_slice(&((next() & 0xffff) as u16).to_be_bytes()),
                _ => {
                    let v = (next() & 0xff) as f32 / 255.0;
                    p.extend_from_slice(&v.to_be_bytes());
                }
            }
        }
        planes.push(p);
    }
    PsdChannels { planes, width: w, height: h, depth, mode }
}

fn make_thumb_jpeg() -> Vec<u8> {
    let w = 1024;
    let h = 581;
    let mut img = RgbImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let t = (x + y) as f32 / (w + h) as f32;
        *p = image::Rgb([
            (79.0 + (147.0 - 79.0) * t) as u8,
            (70.0 + (51.0 - 70.0) * t) as u8,
            (229.0 + (234.0 - 229.0) * t) as u8,
        ]);
    }
    let mut buf = Vec::new();
    let enc = JpegEncoder::new_with_quality(&mut buf, 85);
    enc.write_image(img.as_raw(), w, h, ExtendedColorType::Rgb8).expect("编码缩略图失败");
    buf
}

fn write_corrupt(dir: &PathBuf) {
    let ch = sample_channels(32, 32, 3, 8);
    let base = psd::encode_psd(&ch, 2, 1, None);

    // 坏签名
    let mut bad = base.clone();
    bad[0] = b'X';
    std::fs::write(dir.join("corrupt_sig.psb"), bad).unwrap();
    // 截断
    std::fs::write(dir.join("corrupt_trunc.psb"), &base[..base.len() / 3]).unwrap();
    // 超大尺寸（头部 400000）
    let mut bad = base.clone();
    bad[18..22].copy_from_slice(&400_000u32.to_be_bytes());
    std::fs::write(dir.join("corrupt_dims.psb"), bad).unwrap();
    // 行表越界
    let mut bad = base.clone();
    let info = psd::parse(&bad).unwrap();
    bad[info.img_start + 2..info.img_start + 4].copy_from_slice(&0xffffu16.to_be_bytes());
    std::fs::write(dir.join("corrupt_rowtable.psb"), bad).unwrap();
    // 纯垃圾
    let mut s = 0xDEAD_BEEF_CAFE_F00Du64;
    let garbage: Vec<u8> = (0..4096)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 33) as u8
        })
        .collect();
    std::fs::write(dir.join("corrupt_garbage.psd"), garbage).unwrap();
    println!("损坏样本 5 个");
}

/// ~2GB PSB：30000×17000×4 通道×8bit RLE，噪声内容（PackBits 几乎不可压缩）
fn write_big(path: &PathBuf, with_thumb: bool, thumb: &[u8]) {
    let (w, h, nch) = (30000usize, 17000usize, 4usize);
    let rows = h * nch;
    let t0 = std::time::Instant::now();

    // Pass 1：计算每行编码长度（内容按行号确定，两遍结果一致）
    let mut lens = Vec::with_capacity(rows);
    for r in 0..rows {
        let row = gen_row(r, w);
        let mut enc = Vec::with_capacity(w + w / 64);
        psd::packbits_encode(&row, &mut enc);
        lens.push(enc.len() as u32);
    }
    let data_bytes: u64 = lens.iter().map(|&l| l as u64).sum();

    let file = File::create(path).expect("创建文件失败");
    let mut out = BufWriter::with_capacity(1 << 20, file);

    // 头部
    out.write_all(b"8BPS").unwrap();
    out.write_all(&2u16.to_be_bytes()).unwrap(); // PSB
    out.write_all(&[0u8; 6]).unwrap();
    out.write_all(&(nch as u16).to_be_bytes()).unwrap();
    out.write_all(&(h as u32).to_be_bytes()).unwrap();
    out.write_all(&(w as u32).to_be_bytes()).unwrap();
    out.write_all(&8u16.to_be_bytes()).unwrap(); // depth
    out.write_all(&3u16.to_be_bytes()).unwrap(); // RGB
    out.write_all(&0u64.to_be_bytes()).unwrap(); // 颜色模式数据
    // 图像资源（可选 1036）
    if with_thumb {
        let mut res = Vec::new();
        res.extend_from_slice(b"8BIM");
        res.extend_from_slice(&1036u16.to_be_bytes());
        res.extend_from_slice(&[0u8, 0u8]);
        res.extend_from_slice(&(thumb.len() as u64).to_be_bytes());
        res.extend_from_slice(thumb);
        if thumb.len() % 2 == 1 {
            res.push(0);
        }
        out.write_all(&(res.len() as u64).to_be_bytes()).unwrap();
        out.write_all(&res).unwrap();
    } else {
        out.write_all(&0u64.to_be_bytes()).unwrap();
    }
    out.write_all(&0u64.to_be_bytes()).unwrap(); // 图层与蒙版
    out.write_all(&1u16.to_be_bytes()).unwrap(); // RLE
    // 行字节数表（u32 × 行数×通道数）
    for l in &lens {
        out.write_all(&l.to_be_bytes()).unwrap();
    }
    // Pass 2：行数据
    for r in 0..rows {
        let row = gen_row(r, w);
        let mut enc = Vec::with_capacity(w + w / 64);
        psd::packbits_encode(&row, &mut enc);
        out.write_all(&enc).unwrap();
        if r % 17000 == 0 {
            print!("\r  通道 {}/{}", r / 17000 + 1, nch);
            std::io::stdout().flush().ok();
        }
    }
    out.flush().unwrap();
    drop(out);
    println!(
        "\r{} 完成：{} 行，数据 {:.2} GB，总耗时 {:.1}s",
        path.file_name().unwrap().to_string_lossy(),
        rows,
        data_bytes as f64 / 1e9,
        t0.elapsed().as_secs_f64()
    );
}

fn gen_row(r: usize, w: usize) -> Vec<u8> {
    let mut s = 0x9E37_79B9_7F4A_7C15u64 ^ (r as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let mut row = Vec::with_capacity(w);
    for _ in 0..w {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        row.push((s >> 56) as u8);
    }
    row
}

/// —— 真实 Photoshop 形态样本（--real）——
/// 与合成样本的差异：1036 带 28 字节头、1036 前有其他资源块、CMYK 色彩模式。
/// 用于复现真实文件在缩略图链路上的行为（v1.1.1 修复验证）。
fn write_real(dir: &std::path::Path, jpeg: &[u8]) {
    // 头式 1036 资源数据：28 字节头 + JFIF
    let mut t = vec![0u8; 28];
    t[0..4].copy_from_slice(&1u32.to_be_bytes()); // version = JPEG
    t[4..8].copy_from_slice(&160u32.to_be_bytes());
    t[8..12].copy_from_slice(&120u32.to_be_bytes());
    t[12..16].copy_from_slice(&((28 + jpeg.len()) as u32).to_be_bytes());
    t[16..20].copy_from_slice(&(jpeg.len() as u32).to_be_bytes());
    t.extend_from_slice(jpeg);

    let mut res = Vec::new();
    // 前置资源块 1005（奇数长 Pascal 名，检验补齐）
    push_resource(&mut res, 1, 1005, b"desc\x00", b"pixlens probe");
    push_resource(&mut res, 1, 1036, b"", &t);

    // 1) RGB8 RLE + 头式 1036（真实 Photoshop 最常见形态）
    let ch = sample_channels(200, 120, 3, 8);
    write_real_file(&dir.join("real_thumb_hdr.psd"), &ch, 1, &res);
    // 2) CMYK8 RLE 无缩略图（逼出 CMYK 合成路径）
    let mut cmyk = sample_channels(160, 90, 4, 8);
    cmyk.mode = 4;
    write_real_file(&dir.join("real_cmyk_rle.psd"), &cmyk, 1, &[]);
    // 3) CMYK8 RLE + 头式 1036（PSB）
    let mut res2 = Vec::new();
    push_resource(&mut res2, 2, 1036, b"", &t);
    write_real_file(&dir.join("real_cmyk_thumb_hdr.psb"), &cmyk, 2, &res2);
    // 4) RGB16 RLE + 头式 1036
    let ch16 = sample_channels(200, 120, 4, 16);
    write_real_file(&dir.join("real_rgb16_thumb_hdr.psd"), &ch16, 1, &res);
    println!("真实形态样本 4 个 → {}", dir.display());
}

fn push_resource(out: &mut Vec<u8>, version: u16, id: u16, name: &[u8], data: &[u8]) {
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(&id.to_be_bytes());
    // Pascal 名：长度字节 + 字符，整体补齐到偶数（空名 = 2 字节）
    out.push(name.len() as u8);
    out.extend_from_slice(name);
    if (1 + name.len()) % 2 == 1 {
        out.push(0);
    }
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

/// 手工拼装（encode_psd 不支持自定义资源段）：头 + 空颜色模式 + 资源 + 空图层段 + RLE 图像数据
fn write_real_file(path: &std::path::Path, ch: &PsdChannels, version: u16, res: &[u8]) {
    let ls = if version == 2 { 8 } else { 4 };
    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&[0u8; 6]);
    out.extend_from_slice(&(ch.planes.len() as u16).to_be_bytes());
    out.extend_from_slice(&ch.height.to_be_bytes());
    out.extend_from_slice(&ch.width.to_be_bytes());
    out.extend_from_slice(&ch.depth.to_be_bytes());
    out.extend_from_slice(&ch.mode.to_be_bytes());
    out.extend_from_slice(&vec![0u8; ls]); // 颜色模式数据：空
    out.extend_from_slice(&(res.len() as u32).to_be_bytes());
    out.extend_from_slice(res);
    out.extend_from_slice(&vec![0u8; ls]); // 图层与蒙版：空
    out.extend_from_slice(&1u16.to_be_bytes()); // RLE
    let w = ch.width as usize;
    let h = ch.height as usize;
    let row_bytes = w * (ch.depth as usize / 8);
    let mut table: Vec<u32> = Vec::new();
    let mut rows: Vec<Vec<u8>> = Vec::new();
    for plane in &ch.planes {
        for y in 0..h {
            let mut pr = Vec::new();
            psd::packbits_encode(&plane[y * row_bytes..(y + 1) * row_bytes], &mut pr);
            table.push(pr.len() as u32);
            rows.push(pr);
        }
    }
    for t in &table {
        if version == 2 {
            out.extend_from_slice(&t.to_be_bytes());
        } else {
            out.extend_from_slice(&(*t as u16).to_be_bytes());
        }
    }
    for pr in rows {
        out.extend_from_slice(&pr);
    }
    std::fs::write(path, out).expect("写入样本失败");
}
