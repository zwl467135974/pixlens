//! Explorer"日期"同款：内容元数据日期（对齐资源管理器排序）
//!
//! Explorer 的"日期"列/排序对媒体文件优先用内容元数据日期，无元数据才回退
//! 文件修改时间（实测：NEF 显示 EXIF 拍摄日期 2009-02-10、mp4 显示容器内录制
//! 时间，两者都与文件系统 mtime 不同）。本模块提供同款取值：
//!   图片（jpg/tiff 容器 + RAW）：EXIF DateTimeOriginal
//!   视频（mp4/m4v/mov）：mvhd creation_time（moov 在头尾均可）
//!   其余 / 解析失败：回退 mtime（由前端兜底，本模块不返回即回退）
//!
//! 返回 ISO 字符串而非 ms：EXIF 无时区（前端按本地时区解析），mp4 是 UTC
//! （带 Z 后缀），让 JS 各按正确语义解析。
//!
//! 性能：排序惰性触发（打开文件夹不读内容）；rayon 并发 + 每文件只读头部/
//! 尾部定位元数据；会话内由前端缓存，不重复读。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MediaDate {
    pub path: String,
    /// EXIF → "yyyy-MM-ddTHH:mm:ss"（本地时区）；mp4 → "…Z"（UTC）
    pub iso: String,
}

const HEAD_BYTES: u64 = 256 * 1024;

#[tauri::command]
pub fn get_media_dates(paths: Vec<String>) -> Vec<MediaDate> {
    use rayon::prelude::*;
    paths
        .into_par_iter()
        .filter_map(|p| {
            let path = std::path::Path::new(&p);
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let iso = match ext.as_str() {
                // 图片：EXIF DateTimeOriginal（TIFF 系容器，kamadak 自行 seek）
                "jpg" | "jpeg" | "tif" | "tiff" | "cr2" | "cr3" | "nef" | "arw" | "dng"
                | "orf" | "rw2" | "raf" | "pef" | "srw" | "x3f" | "erf" | "mrw" | "kdc"
                | "dcr" | "iiq" | "rwl" | "gpr" | "3fr" => exif_datetime_original(path),
                "mp4" | "m4v" | "mov" => mp4_creation_time(path),
                _ => None,
            }?;
            Some(MediaDate { path: p, iso })
        })
        .collect()
}

/// EXIF DateTimeOriginal → "yyyy-MM-ddTHH:mm:ss"（无时区，前端按本地解析）
fn exif_datetime_original(path: &std::path::Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let exif = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)).ok()?;
    let field = exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)?;
    // display_value 对 ASCII 带引号；分隔符厂商不统一（':' 或 '-'），按数字段提取
    let s = field.display_value().to_string();
    let digits: Vec<u32> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse().ok())
        .collect();
    if digits.len() < 6 {
        return None;
    }
    let (y, mo, d, h, mi, sec) = (digits[0], digits[1], digits[2], digits[3], digits[4], digits[5]);
    if !(1..=9999).contains(&y)
        || !(1..=12).contains(&mo)
        || !(1..=31).contains(&d)
        || h > 23
        || mi > 59
        || sec > 60
    {
        return None;
    }
    Some(format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{sec:02}"))
}

/// mp4/mov 顶层 box 走查 → moov → mvhd → creation_time → ISO UTC
fn mp4_creation_time(path: &std::path::Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let mut pos = 0u64;
    while pos + 8 <= size {
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        f.read_exact(&mut header).ok()?;
        let mut box_len = u32::from_be_bytes(header[0..4].try_into().ok()?) as u64;
        let box_type: [u8; 4] = header.get(4..8)?.try_into().ok()?;
        if box_len == 1 {
            // 64 位 size
            let mut big = [0u8; 8];
            f.read_exact(&mut big).ok()?;
            box_len = u64::from_be_bytes(big);
        } else if box_len == 0 {
            box_len = size - pos; // 到文件尾
        }
        if box_len < 8 {
            return None;
        }
        if &box_type == b"moov" {
            return mvhd_creation_utc(&mut f, pos + 8, pos + box_len);
        }
        pos += box_len;
    }
    None
}

/// 在 moov 内容 [start, end) 的前几 KB 里找 mvhd
fn mvhd_creation_utc(f: &mut File, start: u64, end: u64) -> Option<String> {
    let want = (end - start).min(HEAD_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = vec![0u8; want as usize];
    f.read_exact(&mut buf).ok()?;
    // mvhd 前一般还有个 4 字节版本/标志头（full box），直接搜 "mvhd" 标记
    let mut i = 0usize;
    while i + 4 <= buf.len() {
        if &buf[i..i + 4] == b"mvhd" {
            let body = &buf[i + 4..];
            // full box: version(1) + flags(3)
            if body.len() < 8 {
                return None;
            }
            let version = body[0];
            let creation = if version == 1 {
                if body.len() < 16 {
                    return None;
                }
                u64::from_be_bytes(body[4..12].try_into().ok()?)
            } else {
                if body.len() < 8 {
                    return None;
                }
                u32::from_be_bytes(body[4..8].try_into().ok()?) as u64
            };
            if creation == 0 {
                return None;
            }
            // QuickTime 纪元 1904-01-01 UTC → Unix 秒
            let epoch = creation as i64 - 2_082_844_800;
            return Some(epoch_s_to_iso_utc(epoch));
        }
        i += 4;
    }
    None
}

/// Unix 秒 → "yyyy-MM-ddTHH:mm:ssZ"（Hinnant civil 算法）
fn epoch_s_to_iso_utc(s: i64) -> String {
    let days = s.div_euclid(86400);
    let secs = s.rem_euclid(86400);
    // civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, (secs % 3600) / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_conversion() {
        // 2009-02-10 19:47:00 UTC
        assert_eq!(epoch_s_to_iso_utc(1_234_295_220), "2009-02-10T19:47:00Z");
        // 纪元起点：1904-01-01 00:00 UTC → -2082844800
        assert_eq!(epoch_s_to_iso_utc(-2_082_844_800), "1904-01-01T00:00:00Z");
    }

    /// 探针：dump mp4 moov 内 mvhd/tkhd/mdhd 的 creation（定位 Explorer 读的哪个）
    #[test]
    #[ignore]
    fn probe_mp4_boxes() {
        let p = r"D:\PSD缩略图测试\15c1d04c3c2523c6462b9adcf921ed45.mp4";
        let mut f = File::open(p).unwrap();
        let size = f.metadata().unwrap().len();
        let mut pos = 0u64;
        while pos + 8 <= size {
            f.seek(SeekFrom::Start(pos)).unwrap();
            let mut header = [0u8; 8];
            f.read_exact(&mut header).unwrap();
            let mut box_len = u32::from_be_bytes(header[0..4].try_into().unwrap()) as u64;
            let box_type: [u8; 4] = header[4..8].try_into().unwrap();
            if box_len == 1 {
                let mut big = [0u8; 8];
                f.read_exact(&mut big).unwrap();
                box_len = u64::from_be_bytes(big);
            } else if box_len == 0 {
                box_len = size - pos;
            }
            println!("box {} @{} len={}", String::from_utf8_lossy(&box_type), pos, box_len);
            if &box_type == b"moov" {
                // 读整个 moov（测试文件不大）
                let want = box_len.min(4 * 1024 * 1024);
                f.seek(SeekFrom::Start(pos + 8)).unwrap();
                let mut buf = vec![0u8; (want - 8) as usize];
                f.read_exact(&mut buf).unwrap();
                for tag in ["mvhd", "tkhd", "mdhd"] {
                    let mut i = 0usize;
                    let mut n = 0;
                    while i + 4 <= buf.len() && n < 3 {
                        if &buf[i..i + 4] == tag.as_bytes() {
                            let body = &buf[i + 4..];
                            let (version, creation) = if body.len() >= 12 && body[0] == 1 {
                                (1u8, u64::from_be_bytes(body[4..12].try_into().unwrap()))
                            } else if body.len() >= 8 {
                                (0u8, u32::from_be_bytes(body[4..8].try_into().unwrap()) as u64)
                            } else {
                                (0, 0)
                            };
                            if creation > 0 {
                                println!(
                                    "  {tag}#{n} v{version} creation={} => {}",
                                    creation,
                                    epoch_s_to_iso_utc(creation as i64 - 2_082_844_800)
                                );
                            }
                            n += 1;
                        }
                        i += 4;
                    }
                }
                return;
            }
            pos += box_len;
        }
    }

    /// 本机实拍：测试文件夹的 NEF（2009-02-10 拍摄）与一支 mp4
    #[test]
    #[ignore]
    fn live_dates() {
        let samples = [
            r"D:\PSD缩略图测试\RAW_NIKON_D90.NEF",
            r"D:\PSD缩略图测试\RAW_LEICA_M8.DNG",
            r"D:\PSD缩略图测试\15c1d04c3c2523c6462b9adcf921ed45.mp4",
            r"D:\PSD缩略图测试\d37fa848fd5d40109b1bdbc8752afa9b.mp4",
            r"D:\PSD缩略图测试\游戏内-弹窗.psd",
        ];
        let dates = get_media_dates(samples.iter().map(|s| s.to_string()).collect());
        for d in dates {
            println!("{} => {}", d.path, d.iso);
        }
    }
}

#[cfg(test)]
mod probe2 {
    use super::*;
    #[test]
    #[ignore]
    fn probe_nef_exif() {
        let p = r"D:\PSD缩略图测试\RAW_NIKON_D90.NEF";
        let file = File::open(p).unwrap();
        match exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)) {
            Ok(exif) => {
                println!("NEF EXIF OK, fields={}", exif.fields().len());
                match exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY) {
                    Some(f) => {
                        let s = f.display_value().to_string();
                        println!("DateTimeOriginal raw: '{s}' len={}", s.len());
                    }
                    None => println!("DateTimeOriginal: absent"),
                }
            }
            Err(e) => println!("NEF EXIF 解析失败: {e}"),
        }
    }
}
