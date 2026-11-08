//! AVIF stills and image sequences share the bundled libavif decoder.
//!
//! The decoder borrows compressed input, owns its YUV planes, and never retains
//! the Rust-owned RGBA destination. No decoder or borrowed plane leaves this worker.
use super::{
    AnimationFrame, CancelReader, Frame, MAX_EDGE, MAX_FILE_BYTES, MAX_PIXELS, output_size,
};
use eframe::egui;
use image::{DynamicImage, GenericImageView};
use libavif_sys as ffi;
use std::{
    ffi::CStr,
    fs::File,
    io::Read,
    marker::PhantomData,
    path::Path,
    ptr::NonNull,
    time::{Duration, Instant},
};

// Bound container table complexity, independently of retained playback pixels.
const MAX_FRAMES: u32 = 100_000;
const MAX_TIME: Duration = Duration::from_secs(5);

pub(crate) fn matches_header(header: &[u8]) -> bool {
    let bytes = ffi::avifROData {
        data: header.as_ptr(),
        size: header.len(),
    };
    // SAFETY: libavif only inspects the supplied live slice during this call.
    // This recognizes both avif stills and avis sequences (including compatible
    // brands), unlike image::guess_format's avif-major-brand-only signature.
    unsafe { ffi::avifPeekCompatibleFileType(&bytes) != 0 }
}

pub(crate) fn read_metadata(
    path: &Path,
    out: &mut crate::core::metadata::Metadata,
) -> Result<(), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE_BYTES {
        return Err("AVIF exceeds 256 MiB file limit".into());
    }
    let mut data = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err("AVIF exceeds 256 MiB file limit".into());
    }
    // Parse only the container. Never call NextImage, decode AV1 pixels, or
    // allocate RGBA frames just to display the inspector.
    let decoder = Decoder::new_with_metadata(&data, true)?;
    decoder.report_metadata(out)
}

pub(super) fn decode(path: &Path, max_side: u32, live: &dyn Fn() -> bool) -> Result<Frame, String> {
    let mut first = None;
    visit(path, max_side, live, &mut |frame, size, animated| {
        first = Some(Frame {
            animation: (animated && max_side > 256).then(|| {
                super::animation::Animation::new(
                    path,
                    image::ImageFormat::Avif,
                    max_side,
                    size,
                    &frame.image,
                )
            }),
            image: frame.image,
            original_size: size,
        });
        Ok(false)
    })?;
    first.ok_or("Empty AVIF animation".into())
}

pub(super) fn visit(
    path: &Path,
    max_side: u32,
    live: &dyn Fn() -> bool,
    emit: &mut dyn FnMut(AnimationFrame, [u32; 2], bool) -> Result<bool, String>,
) -> Result<usize, String> {
    let started = Instant::now();
    check_work(live, started)?;
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    CancelReader { file, live }
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err("AVIF exceeds 256 MiB file limit".into());
    }
    check_work(live, started)?;
    let mut decoder = Decoder::new(&data)?;
    check_work(live, started)?;
    let count = decoder.frame_count()?;
    let mut original_size = None;
    for index in 0..count {
        // Time the individual decode, never the whole sequence or queue wait.
        let started = Instant::now();
        check_work(live, started)?;
        decoder.next()?;
        check_work(live, started)?;
        let (image, size) = decoder.rgba(max_side)?;
        check_work(live, started)?;
        if original_size.is_some_and(|previous| previous != size) {
            return Err("AVIF sequence changes image dimensions".into());
        }
        original_size = Some(size);
        if !emit(
            AnimationFrame {
                image,
                delay: decoder.delay()?,
            },
            size,
            count > 1,
        )? {
            return Ok(index + 1);
        }
    }
    Ok(count)
}

fn check_work(live: &dyn Fn() -> bool, started: Instant) -> Result<(), String> {
    if !live() {
        return Err("Cancelled".into());
    }
    if started.elapsed() > MAX_TIME {
        return Err("AVIF exceeds decode time limit".into());
    }
    Ok(())
}

struct Decoder<'input> {
    ptr: NonNull<ffi::avifDecoder>,
    _input: PhantomData<&'input [u8]>,
}

impl<'input> Decoder<'input> {
    fn new(input: &'input [u8]) -> Result<Self, String> {
        Self::new_with_metadata(input, false)
    }

    fn new_with_metadata(input: &'input [u8], read_metadata: bool) -> Result<Self, String> {
        // SAFETY: allocation is owned by this guard and always freed in Drop.
        let ptr = NonNull::new(unsafe { ffi::avifDecoderCreate() })
            .ok_or("Could not allocate AVIF decoder")?;
        let mut decoder = Self {
            ptr,
            _input: PhantomData,
        };
        // SAFETY: the decoder has not been shared; limits are input fields.
        // SetIOMemory borrows input, whose lifetime is carried by this guard.
        unsafe {
            let state = decoder.ptr.as_mut();
            state.maxThreads = 2;
            state.imageSizeLimit = MAX_PIXELS as u32;
            state.imageDimensionLimit = MAX_EDGE;
            state.imageCountLimit = MAX_FRAMES;
            state.allowProgressive = 0;
            state.ignoreExif = i32::from(!read_metadata);
            state.ignoreXMP = i32::from(!read_metadata);
            check(ffi::avifDecoderSetIOMemory(
                ptr.as_ptr(),
                input.as_ptr(),
                input.len(),
            ))?;
            // AUTO selects the sequence for an 'avis' major brand, and the
            // primary item for an 'avif' still; progressive layers stay stills.
            check(ffi::avifDecoderParse(ptr.as_ptr()))?;
        }
        Ok(decoder)
    }

    fn report_metadata(&self, out: &mut crate::core::metadata::Metadata) -> Result<(), String> {
        let crop = self.clean_aperture()?;
        let image = self.image()?;
        let angle = if image.transformFlags & ffi::AVIF_TRANSFORM_IROT != 0 {
            image.irot.angle
        } else {
            0
        };
        if angle > 3 {
            return Err("Invalid AVIF rotation".into());
        }
        let [width, height] = if angle % 2 == 1 {
            [crop.height, crop.width]
        } else {
            [crop.width, crop.height]
        };
        out.add("Dimensions", format!("{width} × {height} px"));
        out.add("Frame count", self.frame_count()?.to_string());
        out.add("AVIF bit depth", format!("{} bits/sample", image.depth));
        out.add(
            "AVIF alpha",
            if self.state().alphaPresent != 0 {
                "yes"
            } else {
                "no"
            },
        );
        out.add(
            "AVIF YUV format",
            match image.yuvFormat {
                ffi::AVIF_PIXEL_FORMAT_YUV444 => "4:4:4",
                ffi::AVIF_PIXEL_FORMAT_YUV422 => "4:2:2",
                ffi::AVIF_PIXEL_FORMAT_YUV420 => "4:2:0",
                ffi::AVIF_PIXEL_FORMAT_YUV400 => "monochrome",
                _ => "unspecified",
            },
        );
        out.add(
            "AVIF CICP",
            format!(
                "primaries {} · transfer {} · matrix {} · full range {}",
                image.colorPrimaries,
                image.transferCharacteristics,
                image.matrixCoefficients,
                image.yuvRange == ffi::AVIF_RANGE_FULL,
            ),
        );
        if self.state().imageCount > 1 && self.state().timescale > 0 {
            out.add(
                "AVIF duration",
                format!(
                    "{:.3} s",
                    self.state().durationInTimescales as f64 / self.state().timescale as f64,
                ),
            );
        }
        for (name, block) in [
            ("EXIF", &image.exif),
            ("XMP", &image.xmp),
            ("ICC", &image.icc),
        ] {
            if block.size == 0 {
                continue;
            }
            if block.size > 4 * 1024 * 1024 {
                out.warn(format!("AVIF {name} exceeds 4 MiB; skipped."));
                continue;
            }
            if block.data.is_null() {
                out.warn(format!("AVIF {name} has no data."));
                continue;
            }
            // SAFETY: these blocks are owned by the parsed decoder, which is
            // borrowed for this whole method. libavif supplies their lengths;
            // no borrowed bytes escape or survive another decoder operation.
            let bytes = unsafe { std::slice::from_raw_parts(block.data, block.size) };
            match name {
                "EXIF" => crate::core::metadata::tags::exif(bytes, out),
                "ICC" => crate::core::metadata::tags::icc(bytes, out),
                _ => out.add("XMP", String::from_utf8_lossy(bytes).trim_end_matches('\0')),
            }
        }
        Ok(())
    }

    fn state(&self) -> &ffi::avifDecoder {
        // SAFETY: self owns a live decoder; the shared borrow prevents next().
        unsafe { self.ptr.as_ref() }
    }

    fn frame_count(&self) -> Result<usize, String> {
        let count = self.state().imageCount;
        if count <= 0 || count > MAX_FRAMES as i32 {
            return Err("AVIF exceeds frame limit or has no images".into());
        }
        Ok(count as usize)
    }

    fn next(&mut self) -> Result<(), String> {
        // SAFETY: decoder and compressed input are live; no plane references
        // survive this call. libavif handles inter-frame AV1 dependencies.
        check(unsafe { ffi::avifDecoderNextImage(self.ptr.as_ptr()) })
    }

    fn image(&self) -> Result<&ffi::avifImage, String> {
        // SAFETY: after successful Parse/NextImage the decoder owns this image.
        // Its reference cannot outlive the decoder or a subsequent next().
        unsafe { self.state().image.as_ref() }.ok_or("AVIF has no decoded image".into())
    }

    fn dimensions(&self) -> Result<[u32; 2], String> {
        let image = self.image()?;
        if image.width == 0
            || image.height == 0
            || image.width > MAX_EDGE
            || image.height > MAX_EDGE
            || u64::from(image.width) * u64::from(image.height) > MAX_PIXELS
        {
            return Err("AVIF exceeds pixel limit".into());
        }
        Ok([image.width, image.height])
    }

    fn clean_aperture(&self) -> Result<ffi::avifCropRect, String> {
        let [width, height] = self.dimensions()?;
        let image = self.image()?;
        let mut crop = ffi::avifCropRect {
            x: 0,
            y: 0,
            width,
            height,
        };
        if image.transformFlags & ffi::AVIF_TRANSFORM_CLAP != 0 {
            let mut diagnostics = ffi::avifDiagnostics::default();
            // SAFETY: all arguments are initialized, owned values or borrows of
            // the live decoder. The helper validates fractions, bounds, and
            // subsampling alignment without retaining any pointers.
            let valid = unsafe {
                ffi::avifCropRectConvertCleanApertureBox(
                    &mut crop,
                    &image.clap,
                    width,
                    height,
                    image.yuvFormat,
                    &mut diagnostics,
                )
            };
            if valid == 0 {
                return Err("Invalid AVIF clean aperture".into());
            }
        }
        if crop.width == 0
            || crop.height == 0
            || crop
                .x
                .checked_add(crop.width)
                .is_none_or(|right| right > width)
            || crop
                .y
                .checked_add(crop.height)
                .is_none_or(|bottom| bottom > height)
        {
            return Err("AVIF clean aperture exceeds image bounds".into());
        }
        Ok(crop)
    }

    fn rgba(&self, max_side: u32) -> Result<(egui::ColorImage, [u32; 2]), String> {
        let [width, height] = self.dimensions()?;
        let crop = self.clean_aperture()?;
        let image = self.image()?;
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let mut rgb = std::mem::MaybeUninit::<ffi::avifRGBImage>::uninit();
        // SAFETY: SetDefaults initializes every RGB field. Conversion writes
        // tightly packed, 8-bit RGBA into the checked Rust allocation, and does
        // not retain its pointer. Never call avifRGBImageFreePixels on this Vec.
        unsafe {
            ffi::avifRGBImageSetDefaults(rgb.as_mut_ptr(), image);
            let mut rgb = rgb.assume_init();
            rgb.depth = 8;
            rgb.format = ffi::AVIF_RGB_FORMAT_RGBA;
            rgb.alphaPremultiplied = 0;
            rgb.pixels = pixels.as_mut_ptr();
            rgb.rowBytes = width * 4;
            check(ffi::avifImageYUVToRGB(image, &mut rgb))?;
        }
        let rgba =
            image::RgbaImage::from_raw(width, height, pixels).ok_or("Invalid AVIF RGBA buffer")?;
        let mut rgba = DynamicImage::ImageRgba8(rgba);
        // Apply the validated clean aperture before resizing and orientation.
        // Pixel aspect ratio (pasp) is not corrected by this display path yet.
        if crop.x != 0 || crop.y != 0 || crop.width != width || crop.height != height {
            rgba = rgba.crop_imm(crop.x, crop.y, crop.width, crop.height);
        }
        let [out_w, out_h] = output_size(crop.width, crop.height, max_side);
        if [out_w, out_h] != [crop.width, crop.height] {
            rgba = rgba.thumbnail_exact(out_w, out_h);
        }
        // libavif exposes container transforms but does not apply them to RGB.
        // HEIF applies rotation before mirroring; irot is counter-clockwise.
        let angle = if image.transformFlags & ffi::AVIF_TRANSFORM_IROT != 0 {
            image.irot.angle
        } else {
            0
        };
        let mirror = if image.transformFlags & ffi::AVIF_TRANSFORM_IMIR != 0 {
            Some(image.imir.axis)
        } else {
            None
        };
        rgba = orient(rgba, angle, mirror)?;
        let size = if angle % 2 == 1 {
            [crop.height, crop.width]
        } else {
            [crop.width, crop.height]
        };
        let (out_w, out_h) = rgba.dimensions();
        let rgba = rgba.into_rgba8();
        Ok((
            egui::ColorImage::from_rgba_unmultiplied(
                [out_w as usize, out_h as usize],
                rgba.as_raw(),
            ),
            size,
        ))
    }

    fn delay(&self) -> Result<Duration, String> {
        let timing = self.state().imageTiming;
        frame_delay(timing.durationInTimescales, timing.timescale)
    }
}

impl Drop for Decoder<'_> {
    fn drop(&mut self) {
        // SAFETY: sole owner, paired with avifDecoderCreate; borrowed input is
        // still live during destruction due to the guard's input lifetime.
        unsafe { ffi::avifDecoderDestroy(self.ptr.as_ptr()) }
    }
}

fn frame_delay(ticks: u64, timescale: u64) -> Result<Duration, String> {
    if timescale == 0 {
        return Err("AVIF sequence has invalid frame timing".into());
    }
    let nanos = (u128::from(ticks % timescale) * 1_000_000_000 / u128::from(timescale)) as u32;
    let delay = Duration::new(ticks / timescale, nanos).max(Duration::from_millis(10));
    if Instant::now().checked_add(delay).is_none() {
        return Err("AVIF sequence has invalid frame timing".into());
    }
    Ok(delay)
}

fn orient(image: DynamicImage, angle: u8, mirror: Option<u8>) -> Result<DynamicImage, String> {
    let image = match angle {
        0 => image,
        1 => image.rotate270(),
        2 => image.rotate180(),
        3 => image.rotate90(),
        _ => return Err("Invalid AVIF rotation".into()),
    };
    match mirror {
        None => Ok(image),
        Some(0) => Ok(image.flipv()),
        Some(1) => Ok(image.fliph()),
        _ => Err("Invalid AVIF mirror axis".into()),
    }
}

fn check(result: ffi::avifResult) -> Result<(), String> {
    if result == ffi::AVIF_RESULT_OK {
        return Ok(());
    }
    // SAFETY: libavif returns a static, NUL-terminated string for every result.
    let reason = unsafe { CStr::from_ptr(ffi::avifResultToString(result)) }.to_string_lossy();
    Err(format!("AVIF decode failed: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::animation_fixtures::avif_sample as sample;

    #[test]
    fn avif_inspector_recognizes_avis_and_reads_resolution_and_frame_count() {
        let path = sample();
        let report = crate::core::metadata::read(&path).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        for (key, value) in [
            ("Format", "Avif"),
            ("Dimensions", "498 × 286 px"),
            ("Frame count", "22"),
            ("AVIF duration", "3.080 s"),
        ] {
            assert!(report.fields.contains(&(key.into(), value.into())), "{key}");
        }
        let data = std::fs::read(&path).unwrap();
        // Peek requires the complete ftyp box, including all compatible
        // brands. This fixture's box is 44 bytes, not a fixed 32-byte header.
        let ftyp_size = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
        assert_eq!(&data[4..8], b"ftyp");
        assert!(matches_header(&data[..ftyp_size]));
        assert!(!matches_header(&data[..ftyp_size - 1]));
        assert!(!matches_header(b"not an AVIF file"));
        let decoder = Decoder::new_with_metadata(&data, true).unwrap();
        let mut report = crate::core::metadata::Metadata::default();
        decoder.report_metadata(&mut report).unwrap();
        assert_eq!(decoder.state().imageIndex, -1);
        assert!(
            decoder
                .image()
                .unwrap()
                .yuvPlanes
                .iter()
                .all(|p| p.is_null())
        );
        assert!(decoder.image().unwrap().alphaPlane.is_null());
    }

    #[test]
    fn avif_metadata_honors_container_crop_and_rotation_before_decoding() {
        let data = std::fs::read(sample()).unwrap();
        let mut decoder = Decoder::new_with_metadata(&data, true).unwrap();
        // SAFETY: the test exclusively owns this decoder and changes only
        // container transform metadata before any pixel decoding.
        unsafe {
            let image = &mut *decoder.ptr.as_mut().image;
            image.transformFlags = ffi::AVIF_TRANSFORM_CLAP | ffi::AVIF_TRANSFORM_IROT;
            image.clap = ffi::avifCleanApertureBox {
                widthN: 202,
                widthD: 1,
                heightN: 102,
                heightD: 1,
                horizOffN: 0,
                horizOffD: 1,
                vertOffN: 0,
                vertOffD: 1,
            };
            image.irot.angle = 1;
        }
        let mut report = crate::core::metadata::Metadata::default();
        decoder.report_metadata(&mut report).unwrap();
        assert!(
            report
                .fields
                .contains(&("Dimensions".into(), "102 × 202 px".into()))
        );
        assert_eq!(decoder.state().imageIndex, -1);
    }

    #[test]
    fn supplied_avif_preserves_all_frames_and_timing() {
        // Use the public worker routing, so falling back to platform frame zero
        // also fails this regression test.
        let decoded = super::super::decode(&sample(), 1024, &|| true).unwrap();
        assert_eq!(decoded.original_size, [498, 286]);
        assert!(decoded.animation.is_some());
        let mut previous = None;
        let mut total = Duration::ZERO;
        let count = visit(&sample(), 1024, &|| true, &mut |frame, size, animated| {
            assert!(animated);
            assert_eq!(size, [498, 286]);
            assert_eq!(frame.delay, Duration::from_millis(140));
            if let Some(previous) = &previous {
                assert_ne!(&frame.image.pixels, previous);
            } else {
                assert_eq!(frame.image.pixels, decoded.image.pixels);
            }
            total += frame.delay;
            previous = Some(frame.image.pixels);
            Ok(true)
        })
        .unwrap();
        assert_eq!(count, 22);
        assert_eq!(total, Duration::from_millis(3080));
    }

    #[test]
    fn avif_thumbnail_is_static_and_cancelled_work_stops() {
        let decoded = decode(&sample(), 128, &|| true).unwrap();
        assert_eq!(decoded.original_size, [498, 286]);
        assert_eq!(
            decoded.image.size,
            output_size(498, 286, 128).map(|v| v as usize)
        );
        assert!(decoded.animation.is_none());
        assert!(decode(&sample(), 1024, &|| false).is_err());
    }

    #[test]
    fn primary_item_avif_is_a_single_still_image() {
        let mut data = std::fs::read(sample()).unwrap();
        assert_eq!(&data[4..12], b"ftypavis");
        // The fixture contains both a primary item and an image sequence.
        // The major avif brand chooses the primary item, even at display size.
        data[8..12].copy_from_slice(b"avif");
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("still.avif");
        std::fs::write(&path, data).unwrap();
        let decoded = super::super::decode(&path, 1024, &|| true).unwrap();
        assert_eq!(decoded.original_size, [498, 286]);
        assert_eq!(decoded.image.size, [498, 286]);
        assert!(decoded.animation.is_none());
        assert!(decoded.image.pixels.iter().any(|pixel| pixel.r() != 0));
        let report = crate::core::metadata::read(&path).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert!(report.fields.contains(&("Frame count".into(), "1".into())));
        assert!(
            report
                .fields
                .contains(&("Dimensions".into(), "498 × 286 px".into()))
        );
    }

    #[test]
    fn clean_aperture_is_cropped_before_rotation_and_invalid_crops_fail() {
        let data = std::fs::read(sample()).unwrap();
        let mut decoder = Decoder::new(&data).unwrap();
        decoder.next().unwrap();
        let (full, _) = decoder.rgba(1024).unwrap();
        // SAFETY: the test owns the decoder exclusively and changes only its
        // transform metadata. The decoded image and planes remain unchanged.
        unsafe {
            let image = &mut *decoder.ptr.as_mut().image;
            image.transformFlags =
                ffi::AVIF_TRANSFORM_CLAP | ffi::AVIF_TRANSFORM_IROT | ffi::AVIF_TRANSFORM_IMIR;
            image.clap = ffi::avifCleanApertureBox {
                widthN: 202,
                widthD: 1,
                heightN: 102,
                heightD: 1,
                horizOffN: 0,
                horizOffD: 1,
                vertOffN: 0,
                vertOffD: 1,
            };
            image.irot.angle = 1;
            image.imir.axis = 1;
        }
        let crop = decoder.clean_aperture().unwrap();
        assert_eq!(
            [crop.x, crop.y, crop.width, crop.height],
            [148, 92, 202, 102]
        );
        let (cropped, original_size) = decoder.rgba(1024).unwrap();
        assert_eq!(original_size, [102, 202]);
        assert_eq!(cropped.size, [102, 202]);
        // Counter-clockwise rotation then horizontal mirror reverses both
        // source axes: output first/last pixels are crop bottom-right/top-left.
        assert_eq!(cropped.pixels[0], full.pixels[(92 + 101) * 498 + 148 + 201]);
        assert_eq!(*cropped.pixels.last().unwrap(), full.pixels[92 * 498 + 148]);
        let (thumbnail, thumbnail_original) = decoder.rgba(64).unwrap();
        assert_eq!(thumbnail_original, [102, 202]);
        assert_eq!(thumbnail.size[1], 64);
        // SAFETY: same exclusive metadata mutation as above.
        unsafe {
            (*decoder.ptr.as_mut().image).clap.widthN = 1000;
        }
        assert!(decoder.rgba(1024).is_err());
        unsafe {
            (*decoder.ptr.as_mut().image).clap.widthD = 0;
        }
        assert!(decoder.clean_aperture().is_err());
    }

    #[test]
    fn truncated_avif_is_an_error_instead_of_a_partial_animation() {
        let data = std::fs::read(sample()).unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("truncated.avif");
        for length in [32, 1024, data.len() - 100] {
            std::fs::write(&path, &data[..length]).unwrap();
            assert!(visit(&path, 1024, &|| true, &mut |_, _, _| Ok(true)).is_err());
        }
    }

    #[test]
    fn avif_limits_and_exact_fractional_delays() {
        assert_eq!(
            frame_delay(140_000, 1_000_000).unwrap(),
            Duration::from_millis(140)
        );
        assert_eq!(
            frame_delay(1, 30).unwrap(),
            Duration::from_nanos(33_333_333)
        );
        assert_eq!(frame_delay(0, 1).unwrap(), Duration::from_millis(10));
        assert!(frame_delay(1, 0).is_err());
        assert!(frame_delay(u64::MAX, 1).is_err());
        assert!(check_work(&|| true, Instant::now() - Duration::from_secs(6)).is_err());
    }

    #[test]
    fn avif_container_rotation_then_mirror_preserves_alpha() {
        let image =
            image::RgbaImage::from_fn(2, 3, |x, y| image::Rgba([(y * 2 + x) as u8, 0, 0, 64]));
        let transformed = orient(DynamicImage::ImageRgba8(image), 1, Some(1))
            .unwrap()
            .into_rgba8();
        assert_eq!(transformed.dimensions(), (3, 2));
        let values: Vec<_> = transformed.pixels().map(|p| p.0[0]).collect();
        assert_eq!(values, [5, 3, 1, 4, 2, 0]);
        assert!(transformed.pixels().all(|p| p.0[3] == 64));
    }
}
