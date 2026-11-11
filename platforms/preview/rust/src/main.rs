//! Freedesktop thumbnailer. Only a supplied local file is read; no viewer starts.
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

fn input_path(value: &str) -> Result<PathBuf, String> {
    let path = if let Some(uri) = value.strip_prefix("file://") {
        let path = if let Some(local) = uri.strip_prefix("localhost/") {
            format!("/{local}")
        } else if uri.starts_with('/') {
            uri.to_owned()
        } else {
            return Err("Only local file URIs are accepted.".into());
        };
        let bytes = path.as_bytes();
        let mut decoded = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' {
                let text = path.get(index + 1..index + 3).ok_or("Invalid URI escape")?;
                decoded.push(u8::from_str_radix(text, 16).map_err(|_| "Invalid URI escape")?);
                index += 3;
            } else {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
        if decoded.contains(&0) {
            return Err("Invalid NUL in URI".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(decoded))
        }
        #[cfg(not(unix))]
        {
            PathBuf::from(String::from_utf8(decoded).map_err(|_| "Invalid URI")?)
        }
    } else if value.contains("://") {
        return Err("Remote URIs are not supported.".into());
    } else {
        PathBuf::from(value)
    };
    path.canonicalize().map_err(|error| error.to_string())
}

fn uri(path: &Path) -> String {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes()
    };
    #[cfg(not(unix))]
    let owned = path.to_string_lossy();
    #[cfg(not(unix))]
    let bytes = owned.as_bytes();
    let mut result = String::from("file://");
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(byte) {
            result.push(*byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

fn text_chunk(key: &str, value: &str) -> Vec<u8> {
    let mut payload = Vec::from(b"tEXt".as_slice());
    payload.extend_from_slice(key.as_bytes());
    payload.push(0);
    payload.extend_from_slice(value.as_bytes());
    let mut crc = !0u32;
    for byte in &payload {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    let mut chunk = ((payload.len() - 4) as u32).to_be_bytes().to_vec();
    chunk.extend(payload);
    chunk.extend((!crc).to_be_bytes());
    chunk
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut size = None;
    let mut input = None;
    let mut output = None;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--size" => {
                size = Some(
                    args.next()
                        .ok_or("Missing size")?
                        .parse::<u32>()
                        .map_err(|_| "Invalid size")?,
                )
            }
            "--input" => input = Some(args.next().ok_or("Missing input")?),
            "--output" => output = Some(PathBuf::from(args.next().ok_or("Missing output")?)),
            "--help" => {
                println!("nabi-thumbnailer --size PIXELS --input FILE_OR_URI --output PNG");
                return Ok(());
            }
            _ => return Err(format!("Unknown option: {option}")),
        }
    }
    let size = size.ok_or("Missing --size")?;
    let input = input_path(&input.ok_or("Missing --input")?)?;
    let output = output.ok_or("Missing --output")?;
    if output.canonicalize().is_ok_and(|path| path == input) {
        return Err("Output cannot replace the original image.".into());
    }
    let metadata = std::fs::metadata(&input).map_err(|error| error.to_string())?;
    let image = nabi_view::preview::decode_path(&input, size)?;
    let pixels = image::RgbaImage::from_raw(image.width, image.height, image.rgba)
        .ok_or("Invalid pixels")?;
    let mut png = std::io::Cursor::new(Vec::new());
    pixels
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    let mut bytes = png.into_inner();
    let mut attributes = text_chunk("Thumb::URI", &uri(&input));
    let modified = metadata
        .modified()
        .map_err(|error| error.to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    attributes.extend(text_chunk("Thumb::MTime", &modified.to_string()));
    attributes.extend(text_chunk("Thumb::Size", &metadata.len().to_string()));
    attributes.extend(text_chunk("Software", "NABI VIEW"));
    bytes.splice(33..33, attributes); // Immediately after the mandatory IHDR.
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    file.persist(output).map_err(|error| error.to_string())?;
    Ok(())
}

fn main() {
    #[cfg(target_os = "linux")]
    unsafe {
        // A separate thumbnailer process may be terminated safely. This is a
        // real wall-clock ceiling, unlike cancelling a synchronous codec call.
        unsafe extern "C" {
            fn alarm(seconds: u32) -> u32;
        }
        alarm(15);
    }
    if let Err(error) = run() {
        eprintln!("NABI preview: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_uri_round_trip_and_reject_remote() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("공백 % image.png");
        std::fs::write(&path, []).unwrap();
        assert_eq!(
            input_path(&uri(&path)).unwrap(),
            path.canonicalize().unwrap()
        );
        assert!(input_path("file://remote/tmp/a.png").is_err());
        assert!(input_path("https://example.com/a.png").is_err());
    }
}
