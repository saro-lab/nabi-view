//! Read-only metadata inspection. PNG chunk layouts follow https://www.w3.org/TR/png-3/.
pub(crate) mod tags;
pub(crate) mod tree;
use flate2::{Crc, read::ZlibDecoder};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
    time::SystemTime,
};

const MIB: usize = 1024 * 1024;
const FIELD_LIMIT: usize = 2 * MIB;
const REPORT_LIMIT: usize = 8 * MIB;
const FIELD_COUNT: usize = 4096;
const PNG_CHUNK_LIMIT: usize = 8 * MIB;
const PNG_TOTAL_INPUT: usize = 32 * MIB;
const PNG_CHUNK_COUNT: usize = 65536;

#[derive(Default)]
pub struct Metadata {
    pub fields: Vec<(String, String)>,
    pub warnings: Vec<String>,
    retained: usize,
    inflated: usize,
}

fn truncate(s: &mut String, bytes: usize) {
    let mut end = bytes.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

impl Metadata {
    pub fn add(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let (mut key, mut value) = (key.into(), value.into());
        if self.fields.len() >= FIELD_COUNT || self.retained >= REPORT_LIMIT {
            self.warn("Report safety limit reached (4,096 fields / 8 MiB). Additional values were omitted.");
            return;
        }
        if key.len() > 1024 {
            truncate(&mut key, 1024);
            self.warn("A metadata field name exceeded 1 KiB and was shortened.");
        }
        let available = REPORT_LIMIT - self.retained;
        if key.len() > available {
            self.warn("Report size limit reached (8 MiB). Additional values were omitted.");
            self.retained = REPORT_LIMIT;
            return;
        }
        let budget = FIELD_LIMIT.min(available - key.len());
        if value.len() > budget {
            truncate(&mut value, budget);
            self.warn(format!(
                "Value truncated: {key} (field limit 2 MiB, report limit 8 MiB)."
            ));
        }
        self.retained += key.len() + value.len();
        self.fields.push((key, value));
    }

    pub fn warn(&mut self, warning: impl Into<String>) {
        let mut warning = warning.into();
        truncate(&mut warning, 1200);
        if self.warnings.len() < 32 && !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        } else if self.warnings.len() == 32 {
            self.warnings
                .push("Further diagnostic messages were suppressed.".into());
        }
    }
}

// Convert a Unix timestamp to a UTC civil date without a locale or timezone dependency.
fn utc_time(time: SystemTime) -> Option<String> {
    let seconds: i64 = match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().try_into().ok()?,
        Err(error) => {
            let delta = error.duration();
            -i64::try_from(delta.as_secs()).ok()? - i64::from(delta.subsec_nanos() != 0)
        }
    };
    let days = seconds.div_euclid(86400) + 719468;
    let era = days.div_euclid(146097);
    let in_era = days.rem_euclid(146097);
    let year = (in_era - in_era / 1460 + in_era / 36524 - in_era / 146096) / 365;
    let doy = in_era - (365 * year + year / 4 - year / 100);
    let month_index = (5 * doy + 2) / 153;
    let day = doy - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year + era * 400 + i64::from(month <= 2);
    let clock = seconds.rem_euclid(86400);
    Some(format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    ))
}

pub fn read(path: &Path) -> Result<Metadata, String> {
    let info = fs::metadata(path).map_err(|e| e.to_string())?;
    if !info.is_file() {
        return Err("Select a regular image file.".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut report = Metadata::default();
    if let Some(name) = path.file_name() {
        report.add("File name", name.to_string_lossy());
    }
    report.add("Path", path.to_string_lossy());
    report.add("File size", file_size(info.len()));
    if let Some(ext) = path.extension() {
        report.add("Extension", ext.to_string_lossy());
    }
    for (key, date) in [("Modified", info.modified()), ("Created", info.created())] {
        if let Some(date) = date.ok().and_then(utc_time) {
            report.add(key, date);
        }
    }
    let header = reader.fill_buf().map_err(|e| e.to_string())?;
    let format = if crate::core::media::avif::matches_header(header) {
        Ok(image::ImageFormat::Avif)
    } else {
        image::guess_format(header)
    };
    let Ok(format) = format else {
        report.warn("Image signature could not be identified.");
        return Ok(report);
    };
    report.add("Format", format!("{format:?}"));
    let result = if format == image::ImageFormat::Avif {
        crate::core::media::avif::read_metadata(path, &mut report)
    } else if format == image::ImageFormat::Png {
        png(reader, info.len(), &mut report)
    } else {
        crate::core::metadata::tags::read(path, format, &mut report)
    };
    if let Err(e) = result {
        report.warn(format!("Metadata scan incomplete. {e}"));
    }
    Ok(report)
}

// Decimal units match KB/MB/GB, while the parenthesized integer preserves the
// exact byte count even when the readable value has been rounded.
fn file_size(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut exact = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            exact.push(',');
        }
        exact.push(digit);
    }
    let (scale, unit) = [
        (1_000_000_000_000_000_000, "EB"),
        (1_000_000_000_000_000, "PB"),
        (1_000_000_000_000, "TB"),
        (1_000_000_000, "GB"),
        (1_000_000, "MB"),
        (1_000, "KB"),
        (1, "B"),
    ]
    .into_iter()
    .find(|(scale, _)| bytes >= *scale)
    .unwrap_or((1, "B"));
    let readable = format!("{:.2}", bytes as f64 / scale as f64);
    let readable = readable.trim_end_matches('0').trim_end_matches('.');
    format!("{readable} {unit} ({exact})")
}

fn png<R: BufRead + Seek>(mut reader: R, size: u64, out: &mut Metadata) -> Result<(), String> {
    let mut offset = 8;
    let mut input = 0;
    let mut pixel_chunks = 0u64;
    let mut pixel_bytes = 0u64;
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    for _ in 0..PNG_CHUNK_COUNT {
        if offset + 12 > size {
            return Err("PNG ended before a complete IEND chunk.".into());
        }
        let mut head = [0; 8];
        reader.read_exact(&mut head).map_err(|e| e.to_string())?;
        let len = u32be(&head[..4]) as usize;
        let kind: &[u8; 4] = head[4..].try_into().unwrap();
        let name = String::from_utf8_lossy(kind);
        let next = offset + len as u64 + 12;
        if next > size {
            return Err(format!("{name} exceeds file bounds at byte {offset}."));
        }
        if kind == b"IDAT" || kind == b"fdAT" {
            pixel_chunks += 1;
            pixel_bytes += len as u64;
            reader
                .seek(SeekFrom::Start(next))
                .map_err(|e| e.to_string())?;
        } else if len > PNG_CHUNK_LIMIT || len > PNG_TOTAL_INPUT - input {
            out.warn(format!("{name} at byte {offset} skipped ({len} bytes; safety limits are 8 MiB per chunk and 32 MiB total)."));
            reader
                .seek(SeekFrom::Start(next))
                .map_err(|e| e.to_string())?;
        } else {
            input += len;
            let mut data = vec![0; len];
            reader.read_exact(&mut data).map_err(|e| e.to_string())?;
            let mut crc = [0; 4];
            reader.read_exact(&mut crc).map_err(|e| e.to_string())?;
            let mut actual = Crc::new();
            actual.update(kind);
            actual.update(&data);
            if actual.sum() != u32be(&crc) {
                out.warn(format!("{name} at byte {offset} has a CRC mismatch."));
            }
            if kind == b"IEND" {
                if len != 0 {
                    out.warn("IEND has a nonempty payload.");
                }
                if next < size {
                    out.warn(format!(
                        "{} trailing bytes after IEND were not interpreted.",
                        size - next
                    ));
                }
                out.add(
                    "PNG image data",
                    format!("{pixel_chunks} IDAT/fdAT chunks, {pixel_bytes} bytes"),
                );
                return Ok(());
            }
            if out.retained >= REPORT_LIMIT || out.fields.len() >= FIELD_COUNT {
                return Err("Report limit reached. Additional chunks were omitted.".into());
            }
            if let Err(e) = png_field(kind, &data, out) {
                out.warn(format!("{name} at byte {offset}: {e}"));
            }
        }
        offset = next;
    }
    Err("PNG chunk count exceeded the 65,536-chunk safety limit.".into())
}

fn u32be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes[..4].try_into().unwrap())
}
fn u16be(bytes: &[u8]) -> u16 {
    u16::from_be_bytes(bytes[..2].try_into().unwrap())
}
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

fn cstr(bytes: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or("missing null separator")?;
    Ok((&bytes[..end], &bytes[end + 1..]))
}

fn inflate(data: &[u8], limit: usize, label: &str, out: &mut Metadata) -> Result<Vec<u8>, String> {
    let available = (16 * MIB).saturating_sub(out.inflated);
    if available == 0 {
        return Err("combined decompressed metadata exceeds the 16 MiB safety limit".into());
    }
    let limit = limit.min(available);
    let mut decoded = Vec::new();
    let result = ZlibDecoder::new(data)
        .take(limit as u64 + 1)
        .read_to_end(&mut decoded);
    out.inflated += decoded.len();
    result.map_err(|e| format!("invalid zlib stream: {e}"))?;
    if decoded.len() > limit {
        decoded.truncate(limit);
        out.warn(format!(
            "{label}: decompressed value truncated to {limit} bytes."
        ));
    }
    Ok(decoded)
}

fn png_text(kind: &[u8; 4], data: &[u8], out: &mut Metadata) -> Result<(), String> {
    let (key, rest) = cstr(data)?;
    if key.is_empty() || key.len() > 79 {
        return Err("invalid PNG keyword length".into());
    }
    let key = latin1(key);
    let kind_name = String::from_utf8_lossy(kind);
    let label = format!("PNG {kind_name} · {key}");
    let limit = FIELD_LIMIT.min(REPORT_LIMIT.saturating_sub(out.retained));
    if kind == b"tEXt" {
        out.add(label, latin1(rest));
        return Ok(());
    }
    if kind == b"zTXt" {
        let payload = rest
            .strip_prefix(&[0])
            .ok_or("unknown PNG compression method")?;
        let decoded = inflate(payload, limit, &label, out)?;
        out.add(label, latin1(&decoded));
        return Ok(());
    }
    let [flag, method, rest @ ..] = rest else {
        return Err("incomplete iTXt header".into());
    };
    if *flag > 1 || *method != 0 {
        return Err("unknown iTXt compression settings".into());
    }
    let (lang, rest) = cstr(rest)?;
    let (translated, payload) = cstr(rest)?;
    if !lang.is_empty() {
        out.add(format!("{label} · language"), String::from_utf8_lossy(lang));
    }
    if !translated.is_empty() {
        out.add(
            format!("{label} · translated keyword"),
            String::from_utf8_lossy(translated),
        );
    }
    let decoded;
    let payload = if *flag == 1 {
        decoded = inflate(payload, limit, &label, out)?;
        decoded.as_slice()
    } else {
        payload
    };
    let value = String::from_utf8_lossy(payload);
    if matches!(value, std::borrow::Cow::Owned(_)) {
        out.warn(format!(
            "{label} contains invalid or truncated UTF-8; replacement characters are shown."
        ));
    }
    out.add(label, value);
    Ok(())
}

fn png_field(kind: &[u8; 4], data: &[u8], out: &mut Metadata) -> Result<(), String> {
    let name = String::from_utf8_lossy(kind);
    match kind {
        b"tEXt" | b"zTXt" | b"iTXt" => return png_text(kind, data, out),
        b"IHDR" if data.len() == 13 => {
            out.add(
                "Dimensions",
                format!("{} × {} px", u32be(data), u32be(&data[4..])),
            );
            let color = match data[9] {
                0 => "grayscale",
                2 => "RGB",
                3 => "indexed",
                4 => "grayscale + alpha",
                6 => "RGBA",
                _ => "unknown",
            };
            out.add(
                "PNG IHDR",
                format!(
                    "{} bits/sample · {color} ({}) · compression {} · filter {} · interlace {}",
                    data[8], data[9], data[10], data[11], data[12]
                ),
            );
        }
        b"eXIf" => crate::core::metadata::tags::exif(data, out),
        b"iCCP" => {
            let (name, rest) = cstr(data)?;
            let payload = rest
                .strip_prefix(&[0])
                .ok_or("unknown ICC profile compression method")?;
            out.add("PNG ICC profile name", latin1(name));
            let decoded = inflate(payload, FIELD_LIMIT, "PNG ICC profile", out)?;
            crate::core::metadata::tags::icc(&decoded, out);
        }
        b"pHYs" if data.len() == 9 => {
            let units = match data[8] {
                1 => "pixels/metre",
                0 => "unspecified unit",
                _ => "unknown unit",
            };
            out.add(
                "PNG pixel density",
                format!("{} × {} ({units})", u32be(data), u32be(&data[4..])),
            );
        }
        b"gAMA" if data.len() == 4 => {
            out.add("PNG gamma", format!("{}", u32be(data) as f64 / 100000.0))
        }
        b"cHRM" if data.len() == 32 => {
            for (xy, v) in ["White", "Red", "Green", "Blue"]
                .iter()
                .zip(data.as_chunks::<8>().0)
            {
                out.add(
                    format!("PNG chromaticity · {xy}"),
                    format!(
                        "x={:.5}, y={:.5}",
                        u32be(v) as f64 / 100000.0,
                        u32be(&v[4..]) as f64 / 100000.0
                    ),
                );
            }
        }
        b"sRGB" if data.len() == 1 => out.add(
            "PNG sRGB intent",
            format!(
                "{} ({})",
                data[0],
                [
                    "perceptual",
                    "relative colorimetric",
                    "saturation",
                    "absolute colorimetric"
                ]
                .get(data[0] as usize)
                .unwrap_or(&"unknown")
            ),
        ),
        b"tIME" if data.len() == 7 => out.add(
            "PNG modification time",
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
                u16be(data),
                data[2],
                data[3],
                data[4],
                data[5],
                data[6]
            ),
        ),
        b"acTL" if data.len() == 8 => {
            let frames = u32be(data);
            if frames == 0 {
                return Err("APNG animation has no frames".into());
            }
            out.add("Frame count", frames.to_string());
            out.add("APNG plays (0 = forever)", u32be(&data[4..]).to_string());
        }
        b"cICP" if data.len() == 4 => out.add(
            "PNG cICP",
            format!(
                "primaries {} · transfer {} · matrix {} · full range {}",
                data[0], data[1], data[2], data[3]
            ),
        ),
        // Keep the existence, byte length and a labelled preview of unfamiliar binary metadata.
        _ => {
            let prefix = data
                .iter()
                .take(256)
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let note = if data.len() > 256 {
                "; first 256 bytes shown"
            } else {
                ""
            };
            out.add(
                format!("PNG {name}"),
                format!("{} bytes{note}\n{prefix}", data.len()),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
