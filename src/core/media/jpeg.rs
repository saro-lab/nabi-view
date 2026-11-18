//! Reduced-size JPEG decoding. The DCT is evaluated at 1/2, 1/4 or 1/8 scale,
//! so a thumbnail or a screen-sized view never materializes every source pixel.
use super::{CancelReader, Frame, MAX_DECODE_BYTES, MAX_PIXELS, finish_still, output_size};
use image::{DynamicImage, GrayImage, RgbImage};
use jpeg_decoder::{CodingProcess, Decoder, PixelFormat};
use std::{fs, io::BufReader, path::Path};

/// Source pixels accepted when the decoded buffer is reduced. Progressive and
/// full-size decoding keep the general `MAX_PIXELS` limit, because they hold
/// coefficients or pixels at the source resolution.
pub(super) const MAX_SCALED_PIXELS: u64 = 256_000_000;

/// `Ok(None)` leaves the file to the general decoder: no reduction is possible
/// (a full-size request), or the stream is a kind this path does not convert
/// (CMYK, 16-bit, lossless, damaged), whose errors the general decoder reports.
pub(super) fn decode_scaled(
    path: &Path,
    max_side: u32,
    live: &dyn Fn() -> bool,
) -> Result<Option<Frame>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = Decoder::new(BufReader::new(CancelReader { file, live }));
    decoder.set_max_decoding_buffer_size(MAX_DECODE_BYTES as usize);
    if decoder.read_info().is_err() {
        return cancelled_or_none(live);
    }
    let Some(info) = decoder.info() else {
        return Ok(None);
    };
    let [w, h] = [u32::from(info.width), u32::from(info.height)];
    let pixels = u64::from(w) * u64::from(h);
    if pixels == 0 || !matches!(info.pixel_format, PixelFormat::L8 | PixelFormat::RGB24) {
        return Ok(None);
    }
    if pixels > MAX_SCALED_PIXELS {
        return Err("Image dimensions exceed the decoding limit.".into());
    }
    match info.coding_process {
        CodingProcess::DctSequential => {}
        CodingProcess::DctProgressive if pixels <= MAX_PIXELS => {}
        _ => return Ok(None),
    }
    let [out_w, out_h] = output_size(w, h, max_side);
    // The decoder picks the smallest factor still covering the request in one
    // axis. A request above half size decodes at full size: keep that on the
    // general decoder, which is faster at 1:1.
    if out_w > w.div_ceil(2) && out_h > h.div_ceil(2) {
        return Ok(None);
    }
    if decoder.scale(out_w as u16, out_h as u16).is_err() {
        return cancelled_or_none(live);
    }
    let Ok(data) = decoder.decode() else {
        return cancelled_or_none(live);
    };
    let Some(info) = decoder.info() else {
        return Ok(None);
    };
    let size = (u32::from(info.width), u32::from(info.height));
    let image = match info.pixel_format {
        PixelFormat::L8 => GrayImage::from_raw(size.0, size.1, data).map(DynamicImage::ImageLuma8),
        PixelFormat::RGB24 => RgbImage::from_raw(size.0, size.1, data).map(DynamicImage::ImageRgb8),
        _ => None,
    };
    let Some(image) = image else {
        return Ok(None);
    };
    let orientation = decoder
        .exif_data()
        .and_then(image::metadata::Orientation::from_exif_chunk)
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let icc = decoder.icc_profile();
    finish_still(image, [w, h], max_side, icc.as_deref(), orientation, live).map(Some)
}

fn cancelled_or_none(live: &dyn Fn() -> bool) -> Result<Option<Frame>, String> {
    if live() {
        Ok(None)
    } else {
        Err("Cancelled".into())
    }
}
