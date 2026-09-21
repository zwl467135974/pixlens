//! PSD / PSB 自研解析器（doc/03-架构设计 §6，头号特色模块）
//!
//! - PSD 与 PSB 统一处理：差异仅版本号与三处长度字段（u32 → u64）
//! - 全部大端字节序；合成图数据为平面排列（ch0 全部行 → ch1 …）
//! - 快路径：图像资源 1036 = 内嵌 JPEG 缩略图，直接提取返回
//! - 预览：跨步行采样（rstep/cstep），只解码预览需要的行——GB 级 PSB 秒出预览的关键
//! - 深度：8 直读；16 → >>8；32（HDR）→ 线性→sRGB 基础曝光映射（M4 精化）
//! - 护栏：尺寸上限（PSD 30000 / PSB 300000）、全量像素 8 亿、全量缓冲 1.5GB、
//!   行表与文件长度交叉校验；所有读取边界检查，损坏文件返回 Err 而非 panic

#[cfg(feature = "rayon")]
use rayon::prelude::*;

// ── 错误 ─────────────────────────────
#[derive(Debug)]
pub enum PsdError {
    BadSignature,
    BadVersion(u16),
    UnsupportedDepth(u16),
    UnsupportedMode(u16),
    UnsupportedCompression(u16),
    Unsupported(String),
    Truncated,
    TooLarge(String),
}

impl std::fmt::Display for PsdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PsdError::BadSignature => write!(f, "签名不是 8BPS"),
            PsdError::BadVersion(v) => write!(f, "未知版本 {v}"),
            PsdError::UnsupportedDepth(d) => write!(f, "不支持位深 {d}"),
            PsdError::UnsupportedMode(m) => write!(f, "暂不支持此色彩模式（mode={m}）"),
            PsdError::UnsupportedCompression(c) => write!(f, "不支持压缩方式 {c}"),
            PsdError::Unsupported(s) => write!(f, "{s}"),
            PsdError::Truncated => write!(f, "文件截断"),
            PsdError::TooLarge(s) => write!(f, "超大文件拒绝全量合成：{s}"),
        }
    }
}

// ── 头部与段定位 ─────────────────────
#[derive(Debug, Clone)]
pub struct PsdInfo {
    pub version: u16,  // 1=PSD, 2=PSB
    pub channels: u16, // 含颜色 + alpha + 专色
    pub height: u32,
    pub width: u32,
    pub depth: u16, // 1/8/16/32
    pub mode: u16,  // 1=灰度, 3=RGB
    pub res_start: usize,
    pub res_end: usize,
    pub img_start: usize, // 压缩方式字段所在偏移
}

fn u16be(d: &[u8], o: usize) -> Result<u16, PsdError> {
    let b = d.get(o..o + 2).ok_or(PsdError::Truncated)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn u32be(d: &[u8], o: usize) -> Result<u32, PsdError> {
    let b = d.get(o..o + 4).ok_or(PsdError::Truncated)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64be(d: &[u8], o: usize) -> Result<u64, PsdError> {
    let b = d.get(o..o + 8).ok_or(PsdError::Truncated)?;
    Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// 段长度字段：PSD=u32，PSB=u64
fn read_len(d: &[u8], o: usize, version: u16) -> Result<u64, PsdError> {
    if version == 2 {
        u64be(d, o)
    } else {
        u32be(d, o).map(|v| v as u64)
    }
}

fn len_size(version: u16) -> usize {
    if version == 2 {
        8
    } else {
        4
    }
}

pub fn parse(data: &[u8]) -> Result<PsdInfo, PsdError> {
    if data.len() < 26 {
        return Err(PsdError::Truncated);
    }
    if &data[0..4] != b"8BPS" {
        return Err(PsdError::BadSignature);
    }
    let version = u16be(data, 4)?;
    if version != 1 && version != 2 {
        return Err(PsdError::BadVersion(version));
    }
    // 6..12 保留字节：真实文件可能非零，跳过不校验
    let channels = u16be(data, 12)?;
    let height = u32be(data, 14)?;
    let width = u32be(data, 18)?;
    let depth = u16be(data, 22)?;
    let mode = u16be(data, 24)?;
    if channels == 0 || channels > 56 {
        return Err(PsdError::Unsupported("通道数非法".into()));
    }
    let max_dim: u32 = if version == 1 { 30_000 } else { 300_000 };
    if width == 0 || height == 0 || width > max_dim || height > max_dim {
        return Err(PsdError::TooLarge(format!("{width}×{height}")));
    }
    if !matches!(depth, 1 | 8 | 16 | 32) {
        return Err(PsdError::UnsupportedDepth(depth));
    }
    if mode > 9 {
        return Err(PsdError::UnsupportedMode(mode));
    }

    let ls = len_size(version);
    let mut off: u64 = 26;
    let total = data.len() as u64;

    // 颜色模式数据段（跳过）
    let color_len = read_len(data, off as usize, version)?;
    off += ls as u64 + color_len;
    if off > total {
        return Err(PsdError::Truncated);
    }
    // 图像资源段
    let res_len = read_len(data, off as usize, version)?;
    let res_start = (off + ls as u64) as usize;
    let res_end = (off + ls as u64 + res_len) as usize;
    off += ls as u64 + res_len;
    if off > total {
        return Err(PsdError::Truncated);
    }
    // 图层与蒙版段（v1 整体跳过）
    let layer_len = read_len(data, off as usize, version)?;
    off += ls as u64 + layer_len;
    if off > total {
        return Err(PsdError::Truncated);
    }
    // 图像数据段
    let img_start = off as usize;
    if img_start + 2 > data.len() {
        return Err(PsdError::Truncated);
    }

    Ok(PsdInfo {
        version,
        channels,
        height,
        width,
        depth,
        mode,
        res_start,
        res_end,
        img_start,
    })
}

// ── 1036 内嵌缩略图快路径 ───────────────
/// 扫图像资源段提取 1036（JFIF JPEG）；资源段损坏时返回 None（快路径可失败，走合成图）
pub fn embedded_thumbnail(data: &[u8], info: &PsdInfo) -> Option<Vec<u8>> {
    let end = info.res_end.min(data.len());
    let mut off = info.res_start;
    while off + 12 <= end {
        let sig = data.get(off..off + 4)?;
        if sig != b"8BIM" && sig != b"8B64" {
            return None;
        }
        let id = u16be(data, off + 4).ok()?;
        let mut p = off + 6;
        // Pascal 名（长度字节 + 名字，整体补齐到偶数）
        let name_len = *data.get(p)? as usize;
        p += 1;
        let name_padded = if (1 + name_len) % 2 == 1 { name_len + 1 } else { name_len };
        p += name_padded;
        let dlen = read_len(data, p, info.version).ok()? as usize;
        let dstart = p.checked_add(len_size(info.version))?;
        let dend = dstart.checked_add(dlen)?;
        if id == 1036 {
            return data.get(dstart..dend.min(data.len())).map(|s| s.to_vec());
        }
        off = dend + (dlen % 2); // 资源数据同样补齐到偶数
    }
    None
}

// ── 合成图解码 ─────────────────────
pub struct Composite {
    /// 原始尺寸
    pub width: u32,
    pub height: u32,
    /// 解码结果（全量=原始尺寸；预览=采样尺寸）
    pub image: image::RgbaImage,
}

/// PackBits（RLE）解压，输出必须恰好填满 dst
pub fn packbits_decode(src: &[u8], dst: &mut [u8]) -> Result<(), PsdError> {
    let mut si = 0usize;
    let mut di = 0usize;
    while di < dst.len() {
        if si >= src.len() {
            return Err(PsdError::Truncated);
        }
        let n = src[si] as i8;
        si += 1;
        if n >= 0 {
            let count = (n as usize) + 1;
            let seg = src.get(si..si + count).ok_or(PsdError::Truncated)?;
            if di + count > dst.len() {
                return Err(PsdError::Truncated);
            }
            dst[di..di + count].copy_from_slice(seg);
            si += count;
            di += count;
        } else if n != -128 {
            let count = (-n as usize) + 1;
            if si >= src.len() || di + count > dst.len() {
                return Err(PsdError::Truncated);
            }
            let b = src[si];
            si += 1;
            dst[di..di + count].fill(b);
            di += count;
        }
        // n == -128：无操作
    }
    Ok(())
}

/// 解码合成图。
/// - `max_dim == 0`：全量解码（受 8 亿像素 / 1.5GB 缓冲护栏约束）
/// - `max_dim > 0`：跨步行采样预览（只解码采样行，输出 ≤ max_dim 长边）
/// - `exposure`：32 位 HDR 色调映射曝光系数（1.0 = 默认）
pub fn decode_composite(
    data: &[u8],
    info: &PsdInfo,
    max_dim: u32,
    exposure: f32,
) -> Result<Composite, PsdError> {
    // v1 仅支持灰度(1) / RGB(3)；CMYK/Lab/索引等显示占位
    let base = match info.mode {
        1 => 1usize,
        3 => 3usize,
        m => return Err(PsdError::UnsupportedMode(m)),
    };
    match info.depth {
        8 | 16 | 32 => {}
        1 => return Err(PsdError::Unsupported("位图模式暂不支持".into())),
        d => return Err(PsdError::UnsupportedDepth(d)),
    }
    let nch = info.channels as usize;
    if nch < base {
        return Err(PsdError::Unsupported("通道数不足".into()));
    }
    let has_alpha = nch > base; // alpha = 紧跟颜色通道后的第一个通道，其余专色忽略

    let w = info.width as usize;
    let h = info.height as usize;
    let bps = info.depth as usize / 8; // 1/2/4
    let row_bytes = w.checked_mul(bps).ok_or_else(|| PsdError::TooLarge("行字节数溢出".into()))?;

    // 采样步进
    let step = if max_dim > 0 && w.max(h) > max_dim as usize {
        ((w.max(h) as f64 / max_dim as f64).ceil() as usize).max(1)
    } else {
        1
    };
    let ow = (w + step - 1) / step;
    let oh = (h + step - 1) / step;

    // 全量护栏（仅 step==1 时可能是全量）
    if step == 1 {
        let px = w as u64 * h as u64;
        if px > 800_000_000 {
            return Err(PsdError::TooLarge(format!("{px} 像素")));
        }
        if px * 4 > 1_500_000_000 {
            return Err(PsdError::TooLarge(format!("{} GB 缓冲", px * 4 / 1_000_000_000)));
        }
    }

    // 压缩方式与每行数据定位（偏移 + 压缩长度）
    let compression = u16be(data, info.img_start)?;
    let rows_total = h.checked_mul(nch).ok_or_else(|| PsdError::TooLarge("行数溢出".into()))?;
    let row_loc: Option<Vec<(usize, usize)>> = match compression {
        0 => None, // Raw：直接按公式定位
        1 => {
            // RLE：行字节数表（行数×通道数 个 u16(PSD)/u32(PSB)）
            let ent = if info.version == 2 { 4 } else { 2 };
            let tbl = info.img_start + 2;
            if tbl + rows_total * ent > data.len() {
                return Err(PsdError::Truncated);
            }
            let mut offs = Vec::with_capacity(rows_total);
            let mut cur = tbl + rows_total * ent;
            for i in 0..rows_total {
                let rl = if info.version == 2 {
                    u32be(data, tbl + i * ent)? as usize
                } else {
                    u16be(data, tbl + i * ent)? as usize
                };
                // 交叉校验：累计偏移不得越界
                let end = cur.checked_add(rl).ok_or(PsdError::Truncated)?;
                if end > data.len() {
                    return Err(PsdError::Truncated);
                }
                offs.push((cur, rl));
                cur = end;
            }
            Some(offs)
        }
        c => return Err(PsdError::UnsupportedCompression(c)),
    };
    let raw_base = info.img_start + 2;

    let nplanes = base + usize::from(has_alpha);
    // 采样行并行解码（行级粒度，rayon；no-rayon feature 下退化为串行——
    // Shell 缩略图扩展在 COM 代理进程中使用，避免重量级依赖）
    let sampled: Vec<usize> = (0..h).step_by(step).collect();
    let mut buf = vec![0u8; ow.checked_mul(oh).and_then(|n| n.checked_mul(4)).ok_or_else(|| PsdError::TooLarge("输出缓冲溢出".into()))?];
    let row_stride = ow * 4;
    let mut run_row = |planes: &mut Vec<Vec<u8>>, row_out: &mut [u8], y: usize| -> Result<(), PsdError> {
        for (pi, plane) in planes.iter_mut().enumerate().take(nplanes) {
            let row_index = pi * h + y;
            match &row_loc {
                None => {
                    // Raw
                    let start = raw_base
                        .checked_add(row_index.checked_mul(row_bytes).ok_or(PsdError::Truncated)?)
                        .ok_or(PsdError::Truncated)?;
                    let end = start.checked_add(row_bytes).ok_or(PsdError::Truncated)?;
                    let seg = data.get(start..end).ok_or(PsdError::Truncated)?;
                    plane.copy_from_slice(seg);
                }
                Some(locs) => {
                    let (start, len) = *locs.get(row_index).ok_or(PsdError::Truncated)?;
                    let seg = data.get(start..start + len).ok_or(PsdError::Truncated)?;
                    packbits_decode(seg, plane)?;
                }
            }
        }
        convert_row(planes, info, base, has_alpha, w, ow, step, exposure, row_out);
        Ok(())
    };
    #[cfg(feature = "rayon")]
    let result: Result<(), PsdError> = buf
        .par_chunks_mut(row_stride)
        .zip(sampled.par_iter())
        .try_for_each_init(
            || vec![vec![0u8; row_bytes]; nplanes],
            |planes: &mut Vec<Vec<u8>>, (row_out, &y)| run_row(planes, row_out, y),
        );
    #[cfg(not(feature = "rayon"))]
    let result: Result<(), PsdError> = (|| {
        let mut planes = vec![vec![0u8; row_bytes]; nplanes];
        for (row_out, &y) in buf.chunks_mut(row_stride).zip(sampled.iter()) {
            run_row(&mut planes, row_out, y)?;
        }
        Ok(())
    })();
    result?;
    let image = image::RgbaImage::from_raw(ow as u32, oh as u32, buf)
        .ok_or_else(|| PsdError::TooLarge("输出缓冲尺寸非法".into()))?;

    Ok(Composite { width: info.width, height: info.height, image })
}

#[allow(clippy::too_many_arguments)]
fn convert_row(
    planes: &[Vec<u8>],
    info: &PsdInfo,
    base: usize,
    has_alpha: bool,
    w: usize,
    ow: usize,
    step: usize,
    exposure: f32,
    out_row: &mut [u8],
) {
    let bps = info.depth as usize / 8;
    let sample = |plane: &Vec<u8>, x: usize| -> u8 {
        let o = x * bps;
        match info.depth {
            8 => *plane.get(o).unwrap_or(&0),
            16 => {
                let hi = plane.get(o).copied().unwrap_or(0);
                // 大端高字节
                hi
            }
            32 => {
                let b = plane.get(o..o + 4).map(|s| [s[0], s[1], s[2], s[3]]).unwrap_or([0; 4]);
                tone_map(f32::from_be_bytes(b), exposure)
            }
            _ => 0,
        }
    };
    for ox in 0..ow {
        let x = (ox * step).min(w.saturating_sub(1));
        let idx = ox * 4;
        match base {
            1 => {
                let g = sample(&planes[0], x);
                out_row[idx] = g;
                out_row[idx + 1] = g;
                out_row[idx + 2] = g;
            }
            _ => {
                out_row[idx] = sample(&planes[0], x);
                out_row[idx + 1] = sample(&planes[1], x);
                out_row[idx + 2] = sample(&planes[2], x);
            }
        }
        out_row[idx + 3] = if has_alpha { sample(&planes[base], x) } else { 255 };
    }
}

// ── 编码器（单元测试与样本生成用） ──────────
/// PackBits 简单编码器（正确性优先：≥3 连续重复走 repeat，其余 literal）
pub fn packbits_encode(src: &[u8], dst: &mut Vec<u8>) {
    let n = src.len();
    let mut i = 0;
    while i < n {
        // 找重复段
        let mut run = 1;
        while i + run < n && run < 128 && src[i + run] == src[i] {
            run += 1;
        }
        if run >= 3 {
            dst.push((257 - run) as u8); // -(run-1) 的补码
            dst.push(src[i]);
            i += run;
        } else {
            // literal 段：到下一个 ≥3 重复起点或 128 上限
            let mut j = i;
            let mut lit_end = i;
            while j < n && lit_end - i < 128 {
                let mut r = 1;
                while j + r < n && r < 3 && src[j + r] == src[j] {
                    r += 1;
                }
                if r >= 3 {
                    break;
                }
                j += 1;
                lit_end = j;
            }
            let count = lit_end - i;
            if count == 0 {
                break;
            }
            dst.push((count - 1) as u8);
            dst.extend_from_slice(&src[i..i + count]);
            i += count;
        }
    }
}

#[derive(Clone)]
pub struct PsdChannels {
    /// 平面数据：每通道 w*h 个样本，大端、按深度打包
    pub planes: Vec<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub depth: u16,
    pub mode: u16, // 1=灰度, 3=RGB
}

/// 编码为合法 PSD/PSB（Raw 或 RLE 压缩，可选 1036 内嵌缩略图）
pub fn encode_psd(
    ch: &PsdChannels,
    version: u16,
    compression: u16,
    thumbnail: Option<&[u8]>,
) -> Vec<u8> {
    let w = ch.width;
    let h = ch.height;
    let nch = ch.planes.len() as u16;
    let ls = len_size(version);
    let mut out: Vec<u8> = Vec::new();

    // 头部 26B
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&[0u8; 6]);
    out.extend_from_slice(&nch.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&ch.depth.to_be_bytes());
    out.extend_from_slice(&ch.mode.to_be_bytes());

    // 颜色模式数据：空
    out.extend_from_slice(&vec![0u8; ls]);
    // 图像资源
    let mut res: Vec<u8> = Vec::new();
    if let Some(jpeg) = thumbnail {
        res.extend_from_slice(b"8BIM");
        res.extend_from_slice(&1036u16.to_be_bytes());
        res.extend_from_slice(&[0u8, 0u8]); // 空 Pascal 名（偶数对齐）
        write_len_field(&mut res, jpeg.len() as u64, version);
        res.extend_from_slice(jpeg);
        if jpeg.len() % 2 == 1 {
            res.push(0);
        }
    }
    write_len_field(&mut out, res.len() as u64, version);
    out.extend_from_slice(&res);
    // 图层与蒙版：空
    out.extend_from_slice(&vec![0u8; ls]);

    // 图像数据
    out.extend_from_slice(&compression.to_be_bytes());
    let rows_total = (h as usize) * (nch as usize);
    let row_bytes = (w as usize) * (ch.depth as usize / 8);
    match compression {
        0 => {
            for plane in &ch.planes {
                out.extend_from_slice(plane);
            }
        }
        _ => {
            // 行字节数表
            let mut table: Vec<u64> = Vec::with_capacity(rows_total);
            let mut packed_rows: Vec<Vec<u8>> = Vec::with_capacity(rows_total);
            for plane in &ch.planes {
                for y in 0..h as usize {
                    let mut pr = Vec::with_capacity(row_bytes + row_bytes / 128 + 8);
                    packbits_encode(&plane[y * row_bytes..(y + 1) * row_bytes], &mut pr);
                    table.push(pr.len() as u64);
                    packed_rows.push(pr);
                }
            }
            for t in &table {
                if version == 2 {
                    out.extend_from_slice(&(*t as u32).to_be_bytes());
                } else {
                    out.extend_from_slice(&(*t as u16).to_be_bytes());
                }
            }
            for pr in packed_rows {
                out.extend_from_slice(&pr);
            }
        }
    }
    out
}

fn write_len_field(out: &mut Vec<u8>, len: u64, version: u16) {
    if version == 2 {
        out.extend_from_slice(&len.to_be_bytes());
    } else {
        out.extend_from_slice(&(len as u32).to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deterministic(w: u32, h: u32, nch: usize, depth: u16) -> PsdChannels {
        let mut planes = Vec::new();
        let mut s = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let n = (w as usize) * (h as usize);
        let bps = depth as usize / 8;
        for _ in 0..nch {
            let mut p = Vec::with_capacity(n * bps);
            for _ in 0..n {
                match depth {
                    8 => p.push((next() & 0xff) as u8),
                    16 => p.extend_from_slice(&((next() & 0xffff) as u16).to_be_bytes()),
                    _ => {
                        let v = (next() & 0xff) as f32 / 255.0; // 32 位：0..1
                        p.extend_from_slice(&v.to_be_bytes());
                    }
                }
            }
            planes.push(p);
        }
        PsdChannels { planes, width: w, height: h, depth, mode: if nch >= 3 { 3 } else { 1 } }
    }

    fn expected_rgba(ch: &PsdChannels, has_alpha: bool) -> Vec<[u8; 4]> {
        let n = (ch.width as usize) * (ch.height as usize);
        let bps = ch.depth as usize / 8;
        let sample = |plane: &Vec<u8>, i: usize| -> u8 {
            let o = i * bps;
            match ch.depth {
                8 => plane[o],
                16 => plane[o],
                32 => tone_map(f32::from_be_bytes([
                    plane[o],
                    plane[o + 1],
                    plane[o + 2],
                    plane[o + 3],
                ]), 1.0),
                _ => 0,
            }
        };
        let base = ch.planes.len() - usize::from(has_alpha);
        (0..n)
            .map(|i| {
                let g = sample(&ch.planes[0], i);
                let (r, g2, b) = if base == 3 {
                    (sample(&ch.planes[0], i), sample(&ch.planes[1], i), sample(&ch.planes[2], i))
                } else {
                    (g, g, g)
                };
                let a = if has_alpha { sample(&ch.planes[base], i) } else { 255 };
                [r, g2, b, a]
            })
            .collect()
    }

    #[test]
    fn roundtrip_all_variants() {
        // PSD/PSB × Raw/RLE × 灰度/RGB × 8/16/32 位，含 alpha，奇数尺寸检验对齐
        for &version in &[1u16, 2u16] {
            for &compression in &[0u16, 1u16] {
                for &(nch, _) in &[(2usize, 1u16), (4usize, 3u16)] {
                    for &depth in &[8u16, 16u16, 32u16] {
                        let ch = deterministic(33, 17, nch, depth);
                        let data = encode_psd(&ch, version, compression, None);
                        let info = parse(&data).unwrap();
                        assert_eq!(info.version, version);
                        assert_eq!(info.width, 33);
                        assert_eq!(info.height, 17);
                        assert!(embedded_thumbnail(&data, &info).is_none());
                        let c = decode_composite(&data, &info, 0, 1.0).unwrap();
                        assert_eq!(c.image.dimensions(), (33, 17));
                        let expect = expected_rgba(&ch, true);
                        for (i, px) in c.image.pixels().enumerate() {
                            assert_eq!(px.0, expect[i], "v{version} comp{compression} nch{nch} d{depth} @{}", i);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn embedded_thumbnail_extraction() {
        let ch = deterministic(20, 10, 3, 8);
        let fake_jpeg: Vec<u8> = (0..=255u8).cycle().take(777).collect(); // 奇数长度检验偶数对齐
        let data = encode_psd(&ch, 2, 1, Some(&fake_jpeg));
        let info = parse(&data).unwrap();
        let got = embedded_thumbnail(&data, &info).unwrap();
        assert_eq!(got, fake_jpeg);
        // 解码不受资源段影响
        assert!(decode_composite(&data, &info, 0, 1.0).is_ok());
    }

    #[test]
    fn preview_stride_sampling() {
        let ch = deterministic(100, 60, 3, 8);
        let data = encode_psd(&ch, 2, 1, None);
        let info = parse(&data).unwrap();
        let c = decode_composite(&data, &info, 25, 1.0).unwrap();
        let (pw, ph) = c.image.dimensions();
        assert!(pw <= 25 && ph <= 25, "{pw}×{ph}");
        assert_eq!((c.width, c.height), (100, 60));
    }

    #[test]
    fn corrupted_files_rejected() {
        let ch = deterministic(16, 16, 3, 8);
        let data = encode_psd(&ch, 1, 1, None);

        // 坏签名
        let mut bad = data.clone();
        bad[1] = b'X';
        assert!(matches!(parse(&bad), Err(PsdError::BadSignature)));

        // 截断（头部）
        assert!(matches!(parse(&data[..20]), Err(PsdError::Truncated)));

        // 坏版本
        let mut bad = data.clone();
        bad[4] = 0;
        bad[5] = 9;
        assert!(matches!(parse(&bad), Err(PsdError::BadVersion(9))));

        // 超大尺寸（PSB 上限 300000）
        let mut bad = data.clone();
        bad[14..18].copy_from_slice(&400_000u32.to_be_bytes()); // height
        assert!(matches!(parse(&bad), Err(PsdError::TooLarge(_))));

        // 行表越界：把 RLE 首行长度改到天文数字
        let mut bad = data.clone();
        let info = parse(&bad).unwrap();
        let tbl = info.img_start + 2;
        bad[tbl..tbl + 2].copy_from_slice(&0xffffu16.to_be_bytes());
        let info2 = parse(&bad).unwrap();
        assert!(matches!(
            decode_composite(&bad, &info2, 0, 1.0),
            Err(PsdError::Truncated)
        ));

        // 不支持的压缩（ZIP）
        let mut bad = data.clone();
        let info = parse(&bad).unwrap();
        bad[info.img_start..info.img_start + 2].copy_from_slice(&2u16.to_be_bytes());
        let info2 = parse(&bad).unwrap();
        assert!(matches!(
            decode_composite(&bad, &info2, 0, 1.0),
            Err(PsdError::UnsupportedCompression(2))
        ));

        // 不支持的颜色模式（CMYK=4）
        let mut bad = data.clone();
        bad[24..26].copy_from_slice(&4u16.to_be_bytes());
        let info2 = parse(&bad).unwrap();
        assert!(matches!(
            decode_composite(&bad, &info2, 0, 1.0),
            Err(PsdError::UnsupportedMode(4))
        ));
    }

    #[test]
    fn full_pixel_guard() {
        // 构造一个像素超限的合法头部（不实际生成数据）
        let ch = deterministic(4, 4, 3, 8);
        let mut data = encode_psd(&ch, 2, 1, None);
        // 30000×30000 = 9 亿像素 > 8 亿护栏
        data[14..18].copy_from_slice(&30000u32.to_be_bytes());
        data[18..22].copy_from_slice(&30000u32.to_be_bytes());
        let info = parse(&data).unwrap();
        assert!(matches!(
            decode_composite(&data, &info, 0, 1.0),
            Err(PsdError::TooLarge(_))
        ));
        // 伪造头部的文件没有对应行数据，预览会得到 Truncated（护栏生效，不 panic）
        assert!(matches!(
            decode_composite(&data, &info, 256, 1.0),
            Err(PsdError::Truncated)
        ));

        // 真实大图的预览路径正常（跨步采样不受全量护栏约束）
        let big = deterministic(2000, 1500, 3, 8);
        let data = encode_psd(&big, 2, 1, None);
        let info = parse(&data).unwrap();
        let c = decode_composite(&data, &info, 100, 1.0).unwrap();
        assert!(c.image.width() <= 100 && c.image.height() <= 100);
    }

    #[test]
    fn packbits_codec() {
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![5],
            vec![7; 300],
            (0..255u8).collect(),
            {
                let mut v = vec![1u8; 5];
                v.extend((0..200u8).collect::<Vec<u8>>());
                v.extend(vec![9u8; 3]);
                v
            },
        ];
        for src in cases {
            let mut enc = Vec::new();
            packbits_encode(&src, &mut enc);
            let mut dec = vec![0u8; src.len()];
            packbits_decode(&enc, &mut dec).unwrap();
            assert_eq!(dec, src);
        }
    }
}

/// 线性 → sRGB 基础色调映射（带曝光系数）
pub fn tone_map(v: f32, exposure: f32) -> u8 {
    let v = (v * exposure).clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}
