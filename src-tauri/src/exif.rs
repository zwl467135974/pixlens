//! EXIF 读取（backlog 首项，doc/01 §8）
//!
//! kamadak-exif 解析 JPEG/TIFF 容器；PSD 内嵌的 EXIF（0x0424 资源）v1 暂不支持。
//! GPS 按度分秒有理数换算为十进制度。

use std::fs::File;
use std::io::BufReader;

use serde::Serialize;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExifInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    /// "50 mm" 或 "24-70 mm"（含焦距换算视场）
    pub focal_length: Option<String>,
    pub focal_length_35mm: Option<String>,
    /// "f/2.8"
    pub f_number: Option<String>,
    /// "1/250 s" / "2 s"
    pub exposure: Option<String>,
    pub iso: Option<u32>,
    /// 原始拍摄时间（yyyy:MM:dd HH:mm:ss）
    pub datetime: Option<String>,
    /// "39.9042° N, 116.4074° E"
    pub gps: Option<String>,
    /// 方向值 1-8
    pub orientation: Option<u8>,
    pub software: Option<String>,
}

fn field_str(exif: &exif::Exif, tag: exif::Tag) -> Option<String> {
    let s = exif.get_field(tag, exif::In::PRIMARY)?.display_value().to_string();
    // display_value 对 ASCII 值带引号，产品展示去掉
    Some(s.trim_matches('"').to_string())
}

fn rational(value: &exif::Value) -> Option<(u32, u32)> {
    match value {
        exif::Value::Rational(ref v) if !v.is_empty() => Some((v[0].num as u32, v[0].denom as u32)),
        _ => None,
    }
}

#[tauri::command]
pub fn read_exif(path: String) -> Result<ExifInfo, String> {
    let file = File::open(&path).map_err(|e| e.to_string())?;
    let exif = exif::Reader::new()
        .read_from_container(&mut BufReader::new(file))
        .map_err(|e| e.to_string())?;

    let mut info = ExifInfo {
        make: field_str(&exif, exif::Tag::Make),
        model: field_str(&exif, exif::Tag::Model),
        software: field_str(&exif, exif::Tag::Software),
        datetime: field_str(&exif, exif::Tag::DateTimeOriginal),
        ..Default::default()
    };

    // 镜头：LensModel 优先，退回 LensSpecification
    if let Some(lens) = field_str(&exif, exif::Tag::LensModel) {
        info.lens = Some(lens);
    } else if let Some(f) = exif.get_field(exif::Tag::LensSpecification, exif::In::PRIMARY) {
        if let exif::Value::Rational(ref v) = f.value {
            if v.len() == 4 && v[0].num != v[1].num {
                info.lens = Some(format!("{}-{} mm", v[0], v[1]));
            } else if !v.is_empty() {
                info.lens = Some(format!("{} mm", v[0]));
            }
        }
    }

    // 焦距
    if let Some(f) = exif.get_field(exif::Tag::FocalLength, exif::In::PRIMARY) {
        if let Some((n, d)) = rational(&f.value) {
            if d > 0 {
                info.focal_length = Some(format!("{} mm", (n as f64 / d as f64).round()));
            }
        }
    }
    if let Some(f) = exif.get_field(exif::Tag::FocalLengthIn35mmFilm, exif::In::PRIMARY) {
        if let exif::Value::Short(ref v) = f.value {
            if v.first().map(|&x| x > 0).unwrap_or(false) {
                info.focal_length_35mm = Some(format!("{} mm 等效", v[0]));
            }
        }
    }

    // 光圈
    if let Some(f) = exif.get_field(exif::Tag::FNumber, exif::In::PRIMARY) {
        if let Some((n, d)) = rational(&f.value) {
            if d > 0 {
                info.f_number = Some(format!("f/{:.1}", n as f64 / d as f64));
            }
        }
    }

    // 快门
    if let Some(f) = exif.get_field(exif::Tag::ExposureTime, exif::In::PRIMARY) {
        if let Some((n, d)) = rational(&f.value) {
            if n > 0 && d > 0 {
                info.exposure = if d == 1 || n > d {
                    Some(format!("{} s", n / d))
                } else {
                    Some(format!("1/{} s", (d as f64 / n as f64).round() as u32))
                };
            }
        }
    }

    // ISO
    if let Some(f) = exif.get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY) {
        if let exif::Value::Short(ref v) = f.value {
            if v.first().map(|&x| x > 0).unwrap_or(false) {
                info.iso = Some(v[0] as u32);
            }
        }
    }

    // 方向
    if let Some(f) = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY) {
        if let exif::Value::Short(ref v) = f.value {
            info.orientation = v.first().map(|&x| x as u8);
        }
    }

    // GPS
    if let Some(gps) = to_gps(&exif) {
        info.gps = Some(gps);
    }

    Ok(info)
}

fn to_gps(exif: &exif::Exif) -> Option<String> {
    let lat = coord(exif, exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef)?;
    let lon = coord(exif, exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef)?;
    Some(format!("{:.5}° {}, {:.5}° {}", lat.0, lat.1, lon.0, lon.1))
}

/// 度分秒有理数 → (十进制度, N/S/E/W)
fn coord(exif: &exif::Exif, tag: exif::Tag, ref_tag: exif::Tag) -> Option<(f64, String)> {
    let f = exif.get_field(tag, exif::In::PRIMARY)?;
    let exif::Value::Rational(ref v) = f.value else {
        return None;
    };
    if v.len() < 3 {
        return None;
    }
    let mut deg = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
    let dir = exif
        .get_field(ref_tag, exif::In::PRIMARY)
        .and_then(|r| r.value.display_as(ref_tag).to_string().chars().next())
        .unwrap_or('N');
    if dir == 'S' || dir == 'W' {
        deg = -deg;
    }
    Some((deg, dir.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 程序构造带 EXIF 的 JPEG（APP1 段），验证解析链路
    #[test]
    fn parse_constructed_jpeg() {
        // 最小 JPEG：SOI + APP1(EXIF, 含 Make="PixLensTest") + EOI
        let mut jpeg: Vec<u8> = vec![0xFF, 0xD8];
        let tiff: Vec<u8> = build_tiff_ifd();
        let mut app1 = vec![b'E', b'x', b'i', b'f', 0x00, 0x00];
        app1.extend_from_slice(&tiff);
        jpeg.push(0xFF);
        jpeg.push(0xE1);
        // APP1 长度字段 = 段内容（含自身 2 字节）
        jpeg.extend_from_slice(&(((app1.len() + 2) as u16).to_be_bytes()));
        jpeg.extend_from_slice(&app1);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);

        let dir = std::env::temp_dir().join("pixlens_exif_test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("t.jpg");
        std::fs::write(&p, &jpeg).unwrap();

        let info = read_exif(p.to_string_lossy().into_owned()).unwrap();
        assert_eq!(info.make.as_deref(), Some("PixLensTest"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 小端 TIFF 头 + IFD0：Make(ASCII) 一项
    fn build_tiff_ifd() -> Vec<u8> {
        let mut t = vec![b'I', b'I', 0x2A, 0x00]; // 小端
        t.extend_from_slice(&8u32.to_le_bytes()); // IFD 偏移
        t.extend_from_slice(&1u16.to_le_bytes()); // 1 个条目
        // 条目：tag=0x010F(Make) type=2(ASCII) count=12 值偏移=26
        t.extend_from_slice(&0x010Fu16.to_le_bytes());
        t.extend_from_slice(&2u16.to_le_bytes());
        t.extend_from_slice(&12u32.to_le_bytes());
        t.extend_from_slice(&26u32.to_le_bytes());
        t.extend_from_slice(&0u32.to_le_bytes()); // 下一 IFD = 0
        // 字符串区（偏移 26 = 8 + 2 + 12 + 4）
        let s = b"PixLensTest\0";
        t.extend_from_slice(s);
        t
    }

    #[test]
    fn non_exif_file_returns_error() {
        let dir = std::env::temp_dir().join("pixlens_exif_test2");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("no_exif.jpg");
        std::fs::write(&p, vec![0xFF, 0xD8, 0xFF, 0xD9]).unwrap();
        assert!(read_exif(p.to_string_lossy().into_owned()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

