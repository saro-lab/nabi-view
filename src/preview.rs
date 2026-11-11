//! Bounded, read-only first-frame decoding for independent OS preview hosts.
//! This entry point never opens a viewer, settings database, or application cache.
use std::{
    io::{Read, Write},
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

pub const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_SIDE: u32 = 2048;
static DECODER: Mutex<()> = Mutex::new(());

#[derive(Debug)]
pub struct Preview {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha, sRGB RGBA8, top row first.
    pub rgba: Vec<u8>,
}

fn decode(path: &Path, max_side: u32) -> Result<Preview, String> {
    if !(1..=MAX_SIDE).contains(&max_side) {
        return Err("Preview size must be between 1 and 2048 pixels.".into());
    }
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_INPUT_BYTES {
        return Err("Preview input must be a regular file no larger than 64 MiB.".into());
    }
    let started = Instant::now();
    let frame = crate::core::media::decode_still(path, max_side)?;
    // The shared decoder checks dimensions/allocation before decoding. Native
    // codec calls cannot be interrupted here; the OS owns the host lifetime.
    if started.elapsed() > Duration::from_secs(10) {
        return Err("Preview decoding exceeded its time budget.".into());
    }
    let [width, height] = frame.image.size;
    if width == 0 || height == 0 || width > max_side as usize || height > max_side as usize {
        return Err("Preview dimensions exceed the requested size.".into());
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for color in frame.image.pixels {
        rgba.extend_from_slice(&color.to_srgba_unmultiplied());
    }
    Ok(Preview {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

pub fn decode_path(path: &Path, max_side: u32) -> Result<Preview, String> {
    let _permit = DECODER.try_lock().map_err(|_| "Preview decoder is busy.")?;
    decode(path, max_side)
}

/// The supplied stream is copied into a bounded private temporary file. Neither
/// the source stream nor any viewer state is modified. Temporary files are
/// removed before returning, including decoder errors.
pub fn decode_stream(source: impl Read, extension: &str, max_side: u32) -> Result<Preview, String> {
    let _permit = DECODER.try_lock().map_err(|_| "Preview decoder is busy.")?;
    if !(1..=MAX_SIDE).contains(&max_side) {
        return Err("Preview size must be between 1 and 2048 pixels.".into());
    }
    if extension.len() > 16 || !extension.bytes().all(|value| value.is_ascii_alphanumeric()) {
        return Err("Invalid image extension.".into());
    }
    let mut file = tempfile::Builder::new()
        .prefix("nabi-preview-")
        .suffix(&format!(".{extension}"))
        .tempfile_in(temporary_directory()?)
        .map_err(|error| error.to_string())?;
    let copied = std::io::copy(&mut source.take(MAX_INPUT_BYTES + 1), &mut file)
        .map_err(|error| error.to_string())?;
    if copied > MAX_INPUT_BYTES {
        return Err("Preview input exceeds 64 MiB.".into());
    }
    file.flush().map_err(|error| error.to_string())?;
    decode(file.path(), max_side)
}

#[cfg(not(windows))]
fn temporary_directory() -> Result<std::path::PathBuf, String> {
    Ok(std::env::temp_dir())
}

#[cfg(windows)]
fn temporary_directory() -> Result<std::path::PathBuf, String> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};
    // The system preview surrogate runs at low integrity. Its private staging
    // files must go to LocalLow, never the user's ordinary medium-integrity Temp.
    #[repr(C)]
    struct Guid {
        a: u32,
        b: u16,
        c: u16,
        d: [u8; 8],
    }
    const LOCAL_LOW: Guid = Guid {
        a: 0xa520a1a4,
        b: 0x1780,
        c: 0x4ff6,
        d: [0xbd, 0x18, 0x16, 0x73, 0x43, 0xc5, 0xaf, 0x16],
    };
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            id: *const Guid,
            flags: u32,
            token: *mut std::ffi::c_void,
            path: *mut *mut u16,
        ) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(value: *mut std::ffi::c_void);
    }
    let mut raw = ptr::null_mut();
    // SAFETY: OS writes an allocated NUL-terminated path; it is freed exactly once.
    unsafe {
        let status = SHGetKnownFolderPath(&LOCAL_LOW, 0, ptr::null_mut(), &mut raw);
        if status < 0 || raw.is_null() {
            return Err("Low-integrity temporary storage is unavailable.".into());
        }
        let mut length = 0;
        while *raw.add(length) != 0 {
            length += 1;
        }
        let path =
            std::path::PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw, length)));
        CoTaskMemFree(raw.cast());
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_preserves_alpha_and_bounds_without_viewer_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        let image = image::RgbaImage::from_pixel(20, 10, image::Rgba([255, 0, 0, 128]));
        image.save(&path).unwrap();
        let source = std::fs::read(&path).unwrap();
        let result = decode_path(&path, 8).unwrap();
        assert_eq!((result.width, result.height), (8, 4));
        assert_eq!(&result.rgba[..4], &[255, 0, 0, 128]);
        assert_eq!(std::fs::read(&path).unwrap(), source);
        let streamed = decode_stream(source.as_slice(), "png", 8).unwrap();
        assert_eq!(streamed.rgba, result.rgba);
        assert!(decode_path(&path, MAX_SIDE + 1).is_err());
        assert!(decode_stream(source.as_slice(), "../png", 8).is_err());
        assert!(decode_stream(&b"not an image"[..], "png", 8).is_err());
    }
}
