//! Read desktop image metadata without decoding pixels or resolving external resources.
use crate::core::metadata::Metadata;
use exif::{Tag, Value};
use image::ImageFormat;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const BLOCK: usize = 4 << 20;
const TOTAL: usize = 16 << 20;
const SCAN: u64 = 256 << 20;
const RECORDS: usize = 32_768;

pub fn read(path: &Path, format: ImageFormat, out: &mut Metadata) -> Result<(), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut r = BufReader::new(file);
    let result = match format {
        ImageFormat::Jpeg => jpeg(&mut r.take(SCAN), out),
        ImageFormat::Gif => gif(&mut r.take(SCAN), out),
        ImageFormat::WebP => webp(&mut r, size, out),
        ImageFormat::Bmp => bmp(&mut r, size, out),
        ImageFormat::Tiff => {
            let mut data = Vec::new();
            if size > BLOCK as u64 {
                out.warn("TIFF metadata inspection limited to first 4 MiB.");
            }
            r.take(BLOCK as u64)
                .read_to_end(&mut data)
                .map(|_| exif(&data, out))
        }
        _ => Ok(()),
    };
    result.map_err(|e| e.to_string())
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}
fn bytes<const N: usize>(r: &mut impl Read) -> io::Result<[u8; N]> {
    let mut buf = [0; N];
    r.read_exact(&mut buf)?;
    Ok(buf)
}
fn be16(s: &[u8]) -> u16 {
    u16::from_be_bytes([s[0], s[1]])
}
fn be32(s: &[u8]) -> u32 {
    u32::from_be_bytes([s[0], s[1], s[2], s[3]])
}
fn le16(s: &[u8]) -> u16 {
    u16::from_le_bytes([s[0], s[1]])
}
fn le32(s: &[u8]) -> u32 {
    u32::from_le_bytes([s[0], s[1], s[2], s[3]])
}
fn le24(s: &[u8]) -> u32 {
    u32::from_le_bytes([s[0], s[1], s[2], 0])
}
fn dims(out: &mut Metadata, w: impl std::fmt::Display, h: impl std::fmt::Display) {
    out.add("Dimensions", format!("{w} × {h} px"));
}
fn text(s: &[u8]) -> String {
    // JPEG comments and IPTC frequently use Latin-1 when no UTF-8 marker is present.
    match std::str::from_utf8(s) {
        Ok(s) => s.trim_end_matches('\0').to_owned(),
        Err(_) => s
            .iter()
            .map(|&v| char::from(v))
            .collect::<String>()
            .trim_end_matches('\0')
            .to_owned(),
    }
}
fn utf16(s: &[u8], le: bool) -> String {
    let (s, le) = if let Some(s) = s.strip_prefix(&[0xff, 0xfe]) {
        (s, true)
    } else if let Some(s) = s.strip_prefix(&[0xfe, 0xff]) {
        (s, false)
    } else {
        (s, le)
    };
    let u: Vec<_> = s
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| if le { le16(p) } else { be16(p) })
        .collect();
    String::from_utf16_lossy(&u)
        .trim_end_matches('\0')
        .to_owned()
}
// EXIF normally follows the TIFF byte order. Some generators use BOM-less UTF-16BE
// regardless of the TIFF endianness. ASCII-heavy text makes that convention observable.
fn exif_unicode(s: &[u8], tiff_le: bool) -> String {
    let (be_ascii, le_ascii) =
        s.as_chunks::<2>()
            .0
            .iter()
            .take(256)
            .fold((0usize, 0usize), |(be, le), p| {
                let ascii = |c: u8| c.is_ascii_graphic() || c.is_ascii_whitespace();
                (
                    be + usize::from(p[0] == 0 && ascii(p[1])),
                    le + usize::from(p[1] == 0 && ascii(p[0])),
                )
            });
    let le = if be_ascii >= 3 && be_ascii > 4 * le_ascii {
        false
    } else if le_ascii >= 3 && le_ascii > 4 * be_ascii {
        true
    } else {
        tiff_le
    };
    utf16(s, le)
}
fn describe(s: &[u8]) -> String {
    if let Ok(v) = std::str::from_utf8(s) {
        let v = v.trim_end_matches('\0');
        if !v.is_empty()
            && v.chars()
                .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        {
            return v.to_owned();
        }
    }
    let hex: Vec<_> = s.iter().take(32).map(|v| format!("{v:02X}")).collect();
    format!(
        "{} bytes · {}{}",
        s.len(),
        hex.join(" "),
        if s.len() > 32 { " …" } else { "" }
    )
}

pub fn exif(bytes: &[u8], out: &mut Metadata) {
    let bytes = bytes.strip_prefix(b"Exif\0\0").unwrap_or(bytes);
    if bytes.len() > BLOCK {
        out.warn("EXIF block exceeds 4 MiB; skipped.");
        return;
    }
    let le = bytes.starts_with(b"II");
    let exif = exif::Reader::new()
        .continue_on_error(true)
        .read_raw(bytes.to_vec())
        .or_else(|e| {
            e.distill_partial_result(|errors| {
                out.warn(format!("EXIF: {} malformed entries", errors.len()))
            })
        });
    let Ok(exif) = exif else {
        out.warn("Unrecognized or truncated EXIF data.");
        return;
    };
    let mut cost = 0usize;
    for (n, f) in exif.fields().enumerate() {
        if n >= 2048 || cost > TOTAL {
            out.warn("EXIF field limit reached.");
            break;
        }
        let key = format!("EXIF IFD{} · {}", f.ifd_num.0, f.tag);
        let raw = match &f.value {
            Value::Byte(v) | Value::Undefined(v, _) => Some(v.as_slice()),
            _ => None,
        };
        let name = match f.tag.1 {
            0x9c9b => "XPTitle",
            0x9c9c => "XPComment",
            0x9c9d => "XPAuthor",
            0x9c9e => "XPKeywords",
            0x9c9f => "XPSubject",
            _ => "",
        };
        let value = if let Some(v) = raw {
            cost += v.len();
            if f.tag == Tag::UserComment {
                if let Some(v) = v.strip_prefix(b"UNICODE\0") {
                    exif_unicode(v, le)
                } else if let Some(v) = v.strip_prefix(b"ASCII\0\0\0") {
                    text(v)
                } else if let Some(v) = v.strip_prefix(&[0; 8]) {
                    text(v)
                } else {
                    describe(v)
                }
            } else if !name.is_empty() {
                out.add(format!("EXIF IFD{} · {name}", f.ifd_num.0), utf16(v, true));
                continue;
            } else if f.tag.1 == 700 {
                text(v)
            } else if v.len() > 128 {
                describe(v)
            } else {
                f.display_value().with_unit(&exif).to_string()
            }
        } else if let Value::Ascii(v) = &f.value {
            cost += v.iter().map(Vec::len).sum::<usize>();
            v.iter().map(|s| text(s)).collect::<Vec<_>>().join("\n")
        } else {
            let count = match &f.value {
                Value::Short(v) => v.len(),
                Value::Long(v) => v.len(),
                Value::Rational(v) => v.len(),
                Value::SByte(v) => v.len(),
                Value::SShort(v) => v.len(),
                Value::SLong(v) => v.len(),
                Value::SRational(v) => v.len(),
                Value::Float(v) => v.len(),
                Value::Double(v) => v.len(),
                _ => 0,
            };
            cost += count * 8;
            if count > 2048 {
                format!("{count} numeric values (display limit)")
            } else {
                f.display_value().with_unit(&exif).to_string()
            }
        };
        out.add(key, value);
    }
}

pub fn icc(data: &[u8], out: &mut Metadata) {
    if let Some(warning) = crate::core::media::color::icc_warning(data) {
        out.warn(warning);
    }
    out.add("ICC profile", format!("{} bytes", data.len()));
    if data.len() < 132 || &data[36..40] != b"acsp" {
        out.warn("Incomplete ICC profile header.");
        return;
    }
    out.add(
        "ICC version",
        format!("{}.{}.{}", data[8], data[9] >> 4, data[9] & 15),
    );
    for (k, i) in [
        ("ICC class", 12),
        ("ICC color space", 16),
        ("ICC PCS", 20),
        ("ICC manufacturer", 48),
        ("ICC model", 52),
        ("ICC creator", 80),
    ] {
        if data[i..i + 4] != [0; 4] {
            out.add(k, text(&data[i..i + 4]))
        }
    }
    let count = be32(&data[128..]) as usize;
    if count > 1024 {
        out.warn("ICC tag count exceeds display limit.");
    }
    let mut budget = TOTAL;
    for i in 0..count.min(1024) {
        let Some(tag) = data.get(132 + 12 * i..144 + 12 * i) else {
            out.warn("Truncated ICC tag directory.");
            break;
        };
        let off = be32(&tag[4..]) as usize;
        let len = be32(&tag[8..]) as usize;
        let Some(chunk) = data.get(off..off.saturating_add(len)) else {
            out.warn("Invalid ICC tag offsets.");
            continue;
        };
        if chunk.len() < 8 {
            continue;
        }
        let key = format!("ICC {}", text(&tag[..4]));
        if chunk.len() > budget {
            out.warn("ICC tag display limit reached.");
            break;
        }
        budget -= chunk.len();
        match &chunk[..4] {
            b"text" => out.add(key, text(&chunk[8..])),
            b"desc" if chunk.len() >= 12 => {
                let n = (be32(&chunk[8..]) as usize).min(chunk.len() - 12);
                out.add(key, text(&chunk[12..12 + n]));
            }
            b"mluc" if chunk.len() >= 16 => {
                let n = be32(&chunk[8..]) as usize;
                let step = be32(&chunk[12..]) as usize;
                if step < 12 {
                    continue;
                }
                for record in chunk[16..].chunks(step).take(n.min(128)) {
                    if record.len() < 12 {
                        break;
                    }
                    let len = be32(&record[4..]) as usize;
                    let off = be32(&record[8..]) as usize;
                    if let Some(s) = chunk.get(off..off.saturating_add(len)) {
                        if len > budget {
                            out.warn("ICC localized text limit reached.");
                            break;
                        }
                        budget -= len;
                        out.add(
                            format!("{key} {}-{}", text(&record[..2]), text(&record[2..4])),
                            utf16(s, false),
                        )
                    }
                }
            }
            _ => out.add(key, format!("{} · {len} bytes", text(&chunk[..4]))),
        }
    }
}

// Use the buffered reader to pass entropy-coded scans without loading image data into memory.
fn jpeg_marker(r: &mut impl BufRead) -> io::Result<u8> {
    loop {
        let buf = r.fill_buf()?;
        if buf.is_empty() {
            return Err(invalid("JPEG ended before EOI or reached the scan limit"));
        }
        let i = buf.iter().position(|&b| b == 0xff);
        let n = i.map_or(buf.len(), |i| i + 1);
        r.consume(n);
        if i.is_none() {
            continue;
        }
        let mut mark = bytes::<1>(r)?[0];
        while mark == 0xff {
            mark = bytes::<1>(r)?[0]
        }
        if mark != 0 && !(0xd0..=0xd7).contains(&mark) {
            return Ok(mark);
        }
    }
}
fn jpeg(r: &mut impl BufRead, out: &mut Metadata) -> io::Result<()> {
    if bytes::<2>(r)? != [0xff, 0xd8] {
        return Err(invalid("Invalid JPEG signature"));
    }
    let mut icc_parts = BTreeMap::new();
    let mut icc_total = 0;
    let mut budget = TOTAL;
    for _ in 0..RECORDS {
        let mark = jpeg_marker(r)?;
        if mark == 0xd9 {
            if icc_total > 0 {
                if (1..=icc_total).all(|i| icc_parts.contains_key(&i))
                    && icc_parts.len() == icc_total as usize
                {
                    let data: Vec<u8> = icc_parts.into_values().flatten().collect();
                    icc(&data, out)
                } else {
                    out.warn("Incomplete multipart JPEG ICC profile.")
                }
            }
            return Ok(());
        }
        if mark == 1 || mark == 0xd8 {
            continue;
        }
        let n = u16::from_be_bytes(bytes(r)?) as usize;
        if n < 2 {
            return Err(invalid("Invalid JPEG segment length"));
        }
        let mut data = vec![0; n - 2];
        r.read_exact(&mut data)?;
        if matches!(mark, 0xe0..=0xef | 0xfe) {
            if data.len() > budget {
                return Err(invalid("JPEG metadata budget exceeded"));
            }
            budget -= data.len();
        }
        match mark {
            0xfe => out.add("JPEG Comment", text(&data)),
            0xe1 if data.starts_with(b"Exif\0\0") => exif(&data, out),
            0xe1 if data.starts_with(b"http://ns.adobe.com/xap/1.0/\0") => {
                out.add("XMP", text(&data[29..]))
            }
            0xe1 if data.starts_with(b"http://ns.adobe.com/xmp/extension/\0")
                && data.len() >= 75 =>
            {
                out.add(
                    format!(
                        "Extended XMP {} · offset {}/{}",
                        text(&data[35..67]),
                        be32(&data[71..]),
                        be32(&data[67..])
                    ),
                    text(&data[75..]),
                )
            }
            0xe2 if data.starts_with(b"ICC_PROFILE\0") && data.len() >= 14 => {
                icc_total = data[13];
                icc_parts.insert(data[12], data[14..].to_vec());
            }
            0xe0 if data.starts_with(b"JFIF\0") && data.len() >= 14 => {
                out.add(
                    "JFIF",
                    format!(
                        "{}.{} · density {} × {} (unit {})",
                        data[5],
                        data[6],
                        be16(&data[8..]),
                        be16(&data[10..]),
                        data[7]
                    ),
                );
            }
            0xed if data.starts_with(b"Photoshop 3.0\0") => photoshop(&data[14..], out),
            0xc0..=0xcf if !matches!(mark, 0xc4 | 0xc8 | 0xcc) && data.len() >= 6 => {
                dims(out, be16(&data[3..]), be16(&data[1..]));
                out.add(
                    "JPEG sample precision / components",
                    format!("{} bits / {}", data[0], data[5]),
                );
            }
            0xe0..=0xef => out.add(format!("JPEG APP{}", mark - 0xe0), describe(&data)),
            _ => {}
        }
    }
    Err(invalid("JPEG segment limit reached"))
}
fn photoshop(mut s: &[u8], out: &mut Metadata) {
    while s.starts_with(b"8BIM") && s.len() >= 12 {
        let id = be16(&s[4..]);
        let off = 6 + (1 + s[6] as usize).next_multiple_of(2);
        if s.len() < off + 4 {
            break;
        }
        let len = be32(&s[off..]) as usize;
        let start = off + 4;
        let Some(payload) = s.get(start..start.saturating_add(len)) else {
            break;
        };
        let key = format!("Photoshop resource 0x{id:04X}");
        match id {
            0x0424 => out.add("Photoshop XMP", text(payload)),
            0x040f => icc(payload, out),
            0x0404 => iptc(payload, out),
            _ => out.add(key, describe(payload)),
        }
        s = s.get(start + len + (len % 2)..).unwrap_or_default();
    }
}
fn iptc(mut s: &[u8], out: &mut Metadata) {
    while s.len() >= 5 && s[0] == 0x1c {
        let (record, dataset) = (s[1], s[2]);
        let n = be16(&s[3..]) as usize;
        s = &s[5..];
        let len = if n & 0x8000 == 0 {
            n
        } else {
            let k = n & 0x7fff;
            if k == 0 || k > 4 || k > s.len() {
                break;
            }
            let v = s[..k]
                .iter()
                .fold(0usize, |v, &b| (v << 8) | usize::from(b));
            s = &s[k..];
            v
        };
        if len > s.len() {
            break;
        }
        out.add(format!("IPTC {record}:{dataset}"), describe(&s[..len]));
        s = &s[len..];
    }
}

fn webp(r: &mut (impl Read + Seek), file_size: u64, out: &mut Metadata) -> io::Result<()> {
    let h = bytes::<12>(r)?;
    if &h[..4] != b"RIFF" || &h[8..] != b"WEBP" {
        return Err(invalid("Invalid WebP header"));
    }
    let end = u64::from(le32(&h[4..])) + 8;
    if end > file_size || end < 12 {
        return Err(invalid("Invalid WebP RIFF length"));
    }
    let mut budget = TOTAL;
    let mut frames = 0u64;
    for _ in 0..RECORDS {
        let pos = r.stream_position()?;
        if pos == end {
            if frames > 0 {
                out.add("Frame count", frames.to_string())
            }
            return Ok(());
        }
        if end - pos < 8 {
            return Err(invalid("Incomplete RIFF chunk header"));
        }
        let ch = bytes::<8>(r)?;
        let tag = &ch[..4];
        let n = le32(&ch[4..]) as usize;
        let next = pos + 8 + n as u64 + (n % 2) as u64;
        if next > end {
            return Err(invalid("WebP chunk exceeds RIFF length"));
        }
        let head = match tag {
            b"VP8X" | b"VP8 " => 10,
            b"VP8L" => 5,
            b"ANIM" => 6,
            b"ANMF" => 16,
            b"ALPH" => 1,
            _ => n,
        };
        let k = n.min(head).min(BLOCK).min(budget);
        let mut s = vec![0; k];
        r.read_exact(&mut s)?;
        budget -= k;
        if k < n.min(head) {
            out.warn("WebP metadata exceeds display limit.");
        }
        match tag {
            b"VP8X" if k >= 10 => {
                dims(out, 1 + le24(&s[4..]), 1 + le24(&s[7..]));
                out.add("WebP feature flags", format!("0x{:02X}", s[0]));
            }
            b"VP8 " if k >= 10 && s[3..6] == [0x9d, 0x01, 0x2a] => {
                dims(out, le16(&s[6..]) & 0x3fff, le16(&s[8..]) & 0x3fff)
            }
            b"VP8L" if k >= 5 && s[0] == 0x2f => {
                let packed = le32(&s[1..]);
                dims(out, 1 + (packed & 0x3fff), 1 + ((packed >> 14) & 0x3fff));
            }
            b"EXIF" => exif(&s, out),
            b"XMP " => out.add("XMP", text(&s)),
            b"ICCP" => icc(&s, out),
            b"ANIM" if k >= 6 => {
                out.add("WebP loop count", le16(&s[4..]).to_string());
                out.add("WebP background BGRA", format!("{:02X?}", &s[..4]));
            }
            b"ANMF" => {
                frames += 1;
                if k >= 16 {
                    out.add(
                        format!("WebP frame {frames}"),
                        format!(
                            "{} × {} at {}, {} · {} ms · flags 0x{:02X}",
                            1 + le24(&s[6..]),
                            1 + le24(&s[9..]),
                            2 * le24(&s),
                            2 * le24(&s[3..]),
                            le24(&s[12..]),
                            s[15]
                        ),
                    );
                }
            }
            b"ALPH" if k >= 1 => out.add("WebP alpha flags", format!("0x{:02X}", s[0])),
            _ => out.add(format!("WebP {}", text(tag)), describe(&s)),
        }
        r.seek(SeekFrom::Start(next))?;
    }
    Err(invalid("WebP chunk limit reached"))
}

fn gif_blocks(r: &mut impl Read, cap: usize, app: bool) -> io::Result<(usize, Vec<u8>)> {
    let mut result = Vec::new();
    let mut total = 0;
    let mut buf = [0; 255];
    loop {
        let n = usize::from(bytes::<1>(r)?[0]);
        if n == 0 {
            return Ok((total, result));
        }
        r.read_exact(&mut buf[..n])?;
        // Adobe's GIF XMP packet includes the sub-block size bytes as part of the UTF-8 XML.
        if app && result.starts_with(b"XMP DataXMP") {
            total += 1;
            if result.len() < cap {
                result.push(n as u8);
            }
        }
        total += n;
        result.extend_from_slice(&buf[..n.min(cap - result.len())]);
    }
}
fn gif(r: &mut impl Read, out: &mut Metadata) -> io::Result<()> {
    let h = bytes::<13>(r)?;
    if &h[..6] != b"GIF89a" && &h[..6] != b"GIF87a" {
        return Err(invalid("Invalid GIF header"));
    }
    out.add("GIF version", text(&h[3..6]));
    dims(out, le16(&h[6..]), le16(&h[8..]));
    out.add("GIF background index", h[11].to_string());
    out.add("GIF pixel aspect byte", h[12].to_string());
    fn palette(r: &mut impl Read, flags: u8) -> io::Result<()> {
        if flags & 0x80 != 0 {
            let mut table = [0; 768];
            let n = 3 * (2usize << (flags & 7));
            r.read_exact(&mut table[..n])?;
        }
        Ok(())
    }
    palette(r, h[10])?;
    let mut frames = 0;
    let mut budget = TOTAL;
    for _ in 0..RECORDS {
        match bytes::<1>(r)?[0] {
            0x3b => {
                if frames > 0 {
                    out.add("Frame count", frames.to_string());
                }
                return Ok(());
            }
            0x2c => {
                let image = bytes::<9>(r)?;
                frames += 1;
                out.add(
                    format!("GIF frame {frames}"),
                    format!(
                        "{} × {} at {}, {}",
                        le16(&image[4..]),
                        le16(&image[6..]),
                        le16(&image),
                        le16(&image[2..])
                    ),
                );
                palette(r, image[8])?;
                bytes::<1>(r)?;
                gif_blocks(r, 0, false)?;
            }
            0x21 => {
                let label = bytes::<1>(r)?[0];
                let (n, data) = gif_blocks(r, BLOCK.min(budget), label == 0xff)?;
                budget -= data.len();
                if n > data.len() {
                    out.warn(format!(
                        "GIF extension truncated ({} of {n} bytes).",
                        data.len()
                    ));
                }
                match label {
                    0xfe => out.add("GIF Comment", text(&data)),
                    0xf9 if data.len() >= 4 => out.add(
                        "GIF Graphic Control",
                        format!(
                            "{} ms · disposal {} · transparent index {}",
                            u32::from(le16(&data[1..])) * 10,
                            (data[0] >> 2) & 7,
                            if data[0] & 1 != 0 {
                                data[3].to_string()
                            } else {
                                "none".into()
                            }
                        ),
                    ),
                    0xff if data.len() >= 11 => {
                        let name = &data[..11];
                        let payload = &data[11..];
                        if name == b"XMP DataXMP" {
                            let chunk = payload.strip_suffix(&GIF_XMP_TRAILER).unwrap_or(payload);
                            out.add("XMP", text(chunk))
                        } else if matches!(name, b"NETSCAPE2.0" | b"ANIMEXTS1.0")
                            && payload.len() >= 3
                            && payload[0] == 1
                        {
                            out.add("GIF loop count", le16(&payload[1..]).to_string());
                        } else if name == b"ICCRGBG1012" {
                            icc(payload, out)
                        } else {
                            out.add(format!("GIF application {}", text(name)), describe(payload))
                        }
                    }
                    0x01 if data.len() >= 12 => out.add("GIF plain text", text(&data[12..])),
                    _ => out.add(format!("GIF extension 0x{label:02X}"), describe(&data)),
                }
            }
            0 => continue, // padding after an XMP trailer
            _ => return Err(invalid("Unknown GIF block")),
        }
    }
    Err(invalid("GIF block limit reached"))
}

// The final zero terminates the sub-block stream and is consumed by gif_blocks.
const GIF_XMP_TRAILER: [u8; 257] = {
    let mut trailer = [0; 257];
    trailer[0] = 1;
    let mut i = 0;
    while i < 256 {
        trailer[i + 1] = 255 - i as u8;
        i += 1;
    }
    trailer
};

fn bmp(r: &mut (impl Read + Seek), size: u64, out: &mut Metadata) -> io::Result<()> {
    let file = bytes::<14>(r)?;
    if &file[..2] != b"BM" {
        return Err(invalid("Invalid BMP signature"));
    }
    let header = u32::from_le_bytes(bytes(r)?);
    if !(12..=124).contains(&header) {
        return Err(invalid("Unsupported BMP DIB header size"));
    }
    let mut data = vec![0; header as usize];
    data[..4].copy_from_slice(&header.to_le_bytes());
    r.read_exact(&mut data[4..])?;
    out.add("BMP DIB header", format!("{header} bytes"));
    out.add("BMP pixel offset", le32(&file[10..]).to_string());
    if header == 12 {
        dims(out, le16(&data[4..]), le16(&data[6..]));
        out.add("BMP bits per pixel", le16(&data[10..]).to_string());
        return Ok(());
    }
    if header < 40 {
        return Err(invalid("Unsupported BMP DIB header"));
    }
    let height = le32(&data[8..]) as i32;
    dims(
        out,
        (le32(&data[4..]) as i32).unsigned_abs(),
        height.unsigned_abs(),
    );
    out.add(
        "BMP row order",
        if height < 0 { "top-down" } else { "bottom-up" },
    );
    out.add("BMP bits per pixel", le16(&data[14..]).to_string());
    out.add("BMP compression", le32(&data[16..]).to_string());
    out.add(
        "BMP resolution (pixels/m)",
        format!(
            "{} × {}",
            le32(&data[24..]) as i32,
            le32(&data[28..]) as i32
        ),
    );
    out.add(
        "BMP palette / important colors",
        format!("{} / {}", le32(&data[32..]), le32(&data[36..])),
    );
    if header >= 108 {
        out.add("BMP color space", format!("0x{:08x}", le32(&data[56..])));
        out.add(
            "BMP red/green/blue/alpha masks",
            format!(
                "{:08x} / {:08x} / {:08x} / {:08x}",
                le32(&data[40..]),
                le32(&data[44..]),
                le32(&data[48..]),
                le32(&data[52..])
            ),
        );
    }
    if header == 124 {
        out.add("BMP rendering intent", le32(&data[108..]).to_string());
        let n = le32(&data[116..]) as usize;
        let offset = 14 + u64::from(le32(&data[112..]));
        if n > BLOCK {
            out.warn("BMP profile exceeds 4 MiB; skipped.");
        } else if n > 0 {
            if offset + n as u64 > size {
                return Err(invalid("BMP profile exceeds file size"));
            }
            r.seek(SeekFrom::Start(offset))?;
            let mut profile = vec![0; n];
            r.read_exact(&mut profile)?;
            // PROFILE_LINKED contains an external path. Show only the stored name.
            if le32(&data[56..]) == 0x4c494e4b {
                out.add("BMP linked profile name", text(&profile));
            } else {
                icc(&profile, out);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn animated_gif_and_webp_expose_the_same_frame_count_field() {
        let mut data = b"GIF89a\x02\0\x03\0\0\0\0".to_vec();
        for _ in 0..2 {
            // Image descriptor, LZW code size, empty compressed sub-blocks.
            data.extend(b"\x2c\0\0\0\0\x02\0\x03\0\0\x02\0");
        }
        data.push(0x3b);
        let mut report = Metadata::default();
        gif(&mut data.as_slice(), &mut report).unwrap();
        assert!(report.fields.contains(&("Frame count".into(), "2".into())));
        assert!(
            report
                .fields
                .contains(&("Dimensions".into(), "2 × 3 px".into()))
        );

        let mut riff = b"WEBPVP8X\x0a\0\0\0\x02\0\0\0\x01\0\0\x02\0\0".to_vec();
        for _ in 0..2 {
            riff.extend(b"ANMF\x10\0\0\0");
            riff.extend([0; 16]);
        }
        let mut data = b"RIFF".to_vec();
        data.extend((riff.len() as u32).to_le_bytes());
        data.extend(riff);
        let mut report = Metadata::default();
        webp(&mut Cursor::new(&data), data.len() as u64, &mut report).unwrap();
        assert!(report.fields.contains(&("Frame count".into(), "2".into())));
        assert!(
            report
                .fields
                .contains(&("Dimensions".into(), "2 × 3 px".into()))
        );
    }

    fn entry(tag: u16, ty: u16, count: u32, value: u32) -> Vec<u8> {
        [
            tag.to_le_bytes().as_slice(),
            &ty.to_le_bytes(),
            &count.to_le_bytes(),
            &value.to_le_bytes(),
        ]
        .concat()
    }

    #[test]
    fn jpeg_piexif_unicode_comment_survives_mixed_endianness() {
        // A common AUTOMATIC1111 export uses piexif's BOM-less UTF-16BE UserComment.
        let prompt = "sunrise, 파도\nNegative prompt: blur\nSteps: 20, Sampler: Euler, Seed: 7";
        let mut comment = b"UNICODE\0".to_vec();
        comment.extend(prompt.encode_utf16().flat_map(u16::to_be_bytes));
        let mut tiff = b"II\x2a\0\x08\0\0\0\x01\0".to_vec();
        tiff.extend(entry(0x8769, 4, 1, 26));
        tiff.extend([0; 4]);
        tiff.extend([1, 0]);
        tiff.extend(entry(0x9286, 7, comment.len() as u32, 44));
        tiff.extend([0; 4]);
        tiff.extend(comment);
        let exif = [b"Exif\0\0".as_slice(), &tiff].concat();
        let mut jpeg_data = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg_data.extend(((exif.len() + 2) as u16).to_be_bytes());
        jpeg_data.extend(exif);
        jpeg_data.extend([0xff, 0xfe, 0, 6, b's', b'e', b'e', b'd', 0xff, 0xd9]);
        let mut out = Metadata::default();
        jpeg(&mut Cursor::new(jpeg_data), &mut out).unwrap();
        assert!(
            out.fields
                .iter()
                .any(|(k, v)| k.contains("UserComment") && v == prompt)
        );
        assert!(
            out.fields
                .iter()
                .any(|(k, v)| k == "JPEG Comment" && v == "seed")
        );
    }

    #[test]
    fn webp_reads_xmp_after_odd_size_image_chunk() {
        let xmp = b"<xmp>prompt: clouds</xmp>";
        let mut riff = b"WEBPVP8L\x05\0\0\0\x2f\0\0\0\0\0XMP ".to_vec();
        riff.extend((xmp.len() as u32).to_le_bytes());
        riff.extend(xmp);
        if !xmp.len().is_multiple_of(2) {
            riff.push(0)
        }
        let mut file = b"RIFF".to_vec();
        file.extend((riff.len() as u32).to_le_bytes());
        file.extend(riff);
        let mut out = Metadata::default();
        let len = file.len() as u64;
        webp(&mut Cursor::new(file), len, &mut out).unwrap();
        assert!(
            out.fields
                .iter()
                .any(|(k, v)| k == "XMP" && v.as_bytes() == xmp)
        );
    }

    #[test]
    fn gif_adobe_xmp_preserves_sub_block_size_characters() {
        let xmp = b"<x:xmpmeta>workflow or generator prompt</x:xmpmeta>";
        let mut data = b"GIF89a\x01\0\x01\0\0\0\0\x21\xff\x0bXMP DataXMP".to_vec();
        data.extend(xmp);
        data.extend(GIF_XMP_TRAILER);
        data.extend([0, 0, 0x3b]);
        let mut out = Metadata::default();
        gif(&mut data.as_slice(), &mut out).unwrap();
        assert!(
            out.fields
                .iter()
                .any(|(k, v)| k == "XMP" && v.as_bytes() == xmp)
        );
    }
}
