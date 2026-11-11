//! Embedded profiles and scene-linear HDR are converted without an 8-bit intermediate.
//! HDR pixels use linear sRGB primaries; 1.0 is diffuse SDR white (203 cd/m² for PQ).
use eframe::egui;
use image::{DynamicImage, GenericImageView};
use moxcms::{ColorProfile, DataColorSpace, Layout, ParsingOptions, TransformOptions};

pub(super) const MAX_PROFILE_BYTES: usize = 4 * 1024 * 1024;
const HDR_WHITE_NITS: f32 = 203.;

#[derive(Clone, Debug)]
pub(crate) struct HdrImage {
    pub size: [usize; 2],
    /// Straight alpha, linear sRGB. Finite RGB values may exceed one.
    pub rgba: Vec<f32>,
}
impl HdrImage {
    pub fn bytes(&self) -> usize {
        self.rgba.len() * size_of::<f32>()
    }
}

fn profile(icc: &[u8]) -> Result<ColorProfile, String> {
    if icc.len() > MAX_PROFILE_BYTES {
        return Err("Embedded ICC profile exceeds 4 MiB limit".into());
    }
    let profile = ColorProfile::new_from_slice_with_options(
        icc,
        ParsingOptions {
            max_profile_size: MAX_PROFILE_BYTES + 1,
            max_allowed_clut_size: 4 * 1024 * 1024,
            max_allowed_trc_size: 40000,
        },
    )
    .map_err(|e| format!("Invalid embedded ICC profile: {e}"))?;
    if !matches!(
        profile.color_space,
        DataColorSpace::Gray | DataColorSpace::Rgb
    ) {
        return Err(
            "Embedded ICC needs original non-RGB channels unavailable after decoding".into(),
        );
    }
    Ok(profile)
}
/// Information inspector warning; a bad/unsupported ICC never hides valid pixels.
pub(crate) fn icc_warning(icc: &[u8]) -> Option<String> {
    let result = profile(icc).and_then(|profile| {
        let layout = if profile.color_space == DataColorSpace::Gray {
            Layout::GrayAlpha
        } else {
            Layout::Rgba
        };
        profile
            .create_transform_8bit(
                layout,
                &ColorProfile::new_srgb(),
                Layout::Rgba,
                TransformOptions::default(),
            )
            .map(|_| ())
            .map_err(|e| format!("ICC transform unavailable: {e}"))
    });
    result
        .err()
        .map(|e| format!("{e}; displaying decoded colors without the embedded profile."))
}

pub(super) fn to_srgb(image: DynamicImage, icc: Option<&[u8]>) -> Result<DynamicImage, String> {
    let Some(icc) = icc.filter(|p| !p.is_empty()) else {
        return Ok(image);
    };
    let Ok(profile) = profile(icc) else {
        return Ok(image);
    };
    let (width, height) = image.dimensions();
    let gray = profile.color_space == DataColorSpace::Gray;
    let layout = if gray {
        Layout::GrayAlpha
    } else {
        Layout::Rgba
    };
    let stride = width as usize * if gray { 2 } else { 4 };
    let target = ColorProfile::new_srgb();
    macro_rules! convert {
        ($source:expr, $method:ident, $variant:ident, $component:ty) => {{
            let Ok(transform) =
                profile.$method(layout, &target, Layout::Rgba, TransformOptions::default())
            else {
                return Ok(image);
            };
            let source = $source;
            let mut pixels = vec![<$component>::default(); width as usize * height as usize * 4];
            for (source, dest) in source
                .chunks_exact(stride)
                .zip(pixels.chunks_exact_mut(width as usize * 4))
            {
                transform
                    .transform(source, dest)
                    .map_err(|e| format!("ICC color conversion failed: {e}"))?;
            }
            Ok(DynamicImage::$variant(
                image::ImageBuffer::from_raw(width, height, pixels)
                    .ok_or("Invalid color image dimensions")?,
            ))
        }};
    }
    match image.color() {
        image::ColorType::L16
        | image::ColorType::La16
        | image::ColorType::Rgb16
        | image::ColorType::Rgba16 => {
            convert!(
                if gray {
                    image.into_luma_alpha16().into_raw()
                } else {
                    image.into_rgba16().into_raw()
                },
                create_transform_16bit,
                ImageRgba16,
                u16
            )
        }
        image::ColorType::Rgb32F | image::ColorType::Rgba32F => {
            let converted: Result<DynamicImage, String> = convert!(
                if gray {
                    image.to_luma_alpha32f().into_raw()
                } else {
                    image.into_rgba32f().into_raw()
                },
                create_transform_f32,
                ImageRgba32F,
                f32
            );
            let mut converted = converted?.into_rgba32f();
            for p in converted.pixels_mut() {
                for c in &mut p.0[..3] {
                    *c = if *c <= 0.04045 {
                        *c / 12.92
                    } else {
                        ((*c + 0.055) / 1.055).powf(2.4)
                    };
                }
            }
            Ok(DynamicImage::ImageRgba32F(converted))
        }
        _ => {
            convert!(
                if gray {
                    image.into_luma_alpha8().into_raw()
                } else {
                    image.into_rgba8().into_raw()
                },
                create_transform_8bit,
                ImageRgba8,
                u8
            )
        }
    }
}

/// Final presentation conversion. Float EXR/Radiance data stays available for HDR
/// output, with a fixed Reinhard SDR fallback for other displays.
pub(super) fn display(image: DynamicImage) -> (egui::ColorImage, Option<HdrImage>) {
    let (w, h) = image.dimensions();
    let size = [w as usize, h as usize];
    if !matches!(
        image.color(),
        image::ColorType::Rgb32F | image::ColorType::Rgba32F
    ) {
        return (
            egui::ColorImage::from_rgba_unmultiplied(size, image.into_rgba8().as_raw()),
            None,
        );
    }
    let mut rgba = image.into_rgba32f().into_raw();
    for pixel in rgba.chunks_exact_mut(4) {
        for channel in &mut pixel[..3] {
            *channel = finite(*channel).clamp(0., 65504.);
        }
        pixel[3] = finite(pixel[3]).clamp(0., 1.);
    }
    let mut sdr = Vec::with_capacity(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        let y = luminance(pixel);
        // Pixel-local exposure/curve: a crop or a smaller preview must never
        // change the brightness of the same source sample.
        let scale = 1. / (1. + y);
        for c in &pixel[..3] {
            sdr.push((linear_to_srgb(*c * scale).clamp(0., 1.) * 255.).round() as u8);
        }
        sdr.push((pixel[3] * 255.).round() as u8);
    }
    (
        egui::ColorImage::from_rgba_unmultiplied(size, &sdr),
        Some(HdrImage { size, rgba }),
    )
}

fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0. }
}
fn luminance(p: &[f32]) -> f32 {
    (p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722).max(0.)
}
fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }
}

/// PQ/ST2084 is absolute; HLG uses a 1000-nit reference display and 1.2 system gamma.
/// The caller supplies RGB after YUV conversion, before any ICC transformation.
pub(super) fn cicp_hdr(
    image: DynamicImage,
    primaries: u16,
    transfer: u16,
) -> Result<DynamicImage, String> {
    if transfer != 16 && transfer != 18 {
        return Ok(image);
    }
    if !matches!(primaries, 1 | 9 | 12) {
        return Err(format!("Unsupported HDR color primaries: {primaries}"));
    }
    let mut image = image.into_rgba32f();
    for pixel in image.pixels_mut() {
        let mut rgb = [0.; 3];
        for i in 0..3 {
            rgb[i] = if transfer == 16 {
                pq_eotf(pixel[i]) / HDR_WHITE_NITS
            } else {
                hlg_inverse(pixel[i])
            };
        }
        if transfer == 18 {
            let y = if primaries == 9 {
                rgb[0] * 0.2627 + rgb[1] * 0.678 + rgb[2] * 0.0593
            } else {
                luminance(&rgb)
            };
            let scale = y.max(0.).powf(0.2) * 1000. / HDR_WHITE_NITS;
            for c in &mut rgb {
                *c *= scale;
            }
        }
        let matrix = match primaries {
            9 => [
                [1.660491, -0.587641, -0.072850],
                [-0.124550, 1.132900, -0.008349],
                [-0.018151, -0.100579, 1.118730],
            ],
            12 => [
                [1.224745, -0.224904, 0.],
                [-0.042058, 1.042081, 0.],
                [-0.019642, -0.078655, 1.098537],
            ],
            _ => [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        };
        for row in 0..3 {
            pixel[row] =
                matrix[row][0] * rgb[0] + matrix[row][1] * rgb[1] + matrix[row][2] * rgb[2];
        }
    }
    Ok(DynamicImage::ImageRgba32F(image))
}
fn pq_eotf(v: f32) -> f32 {
    let p = v.clamp(0., 1.).powf(32. / 2523.);
    10000.
        * ((p - 3424. / 4096.).max(0.) / (2413. / 128. - 2392. / 128. * p))
            .max(0.)
            .powf(16384. / 2610.)
}
fn hlg_inverse(v: f32) -> f32 {
    let v = v.clamp(0., 1.);
    if v <= 0.5 {
        v * v / 3.
    } else {
        (((v - 0.5599107) / 0.17883277).exp() + 0.28466892) / 12.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tone_mapping_is_identical_for_a_region_and_full_image() {
        let pixel = vec![2., 1., 0.5, 1.];
        let mut full = pixel.clone();
        full.extend_from_slice(&[100., 100., 100., 1.]);
        let (region, _) = display(DynamicImage::ImageRgba32F(
            image::ImageBuffer::from_raw(1, 1, pixel).unwrap(),
        ));
        let (full, _) = display(DynamicImage::ImageRgba32F(
            image::ImageBuffer::from_raw(2, 1, full).unwrap(),
        ));
        assert_eq!(region.pixels[0], full.pixels[0]);
    }
    #[test]
    fn wide_gamut_profile_changes_color_and_keeps_alpha() {
        let profile = ColorProfile::new_adobe_rgb().encode().unwrap();
        let source = image::RgbaImage::from_pixel(2, 1, image::Rgba([120, 150, 80, 97]));
        let converted = to_srgb(DynamicImage::ImageRgba8(source.clone()), Some(&profile))
            .unwrap()
            .into_rgba8();
        assert_ne!(
            &converted.get_pixel(0, 0).0[..3],
            &source.get_pixel(0, 0).0[..3]
        );
        assert_eq!(converted.get_pixel(0, 0).0[3], 97);
    }
    #[test]
    fn srgb_roundtrip_and_bad_profiles_are_bounded() {
        let source = image::RgbaImage::from_pixel(2, 1, image::Rgba([20, 140, 210, 128]));
        let profile = ColorProfile::new_srgb().encode().unwrap();
        let converted = to_srgb(DynamicImage::ImageRgba8(source.clone()), Some(&profile))
            .unwrap()
            .into_rgba8();
        for (a, b) in converted.as_raw().iter().zip(source.as_raw()) {
            assert!(a.abs_diff(*b) <= 1);
        }
        let fallback = to_srgb(DynamicImage::ImageRgba8(source.clone()), Some(&[0; 128]))
            .unwrap()
            .into_rgba8();
        assert_eq!(fallback, source);
        assert!(icc_warning(&[0; 128]).is_some());
    }
    #[test]
    fn profile_conversion_keeps_sixteen_bit_precision() {
        let source =
            image::ImageBuffer::from_pixel(1, 1, image::Rgba([40001u16, 25001, 12345, 12345]));
        let profile = ColorProfile::new_srgb().encode().unwrap();
        let converted = to_srgb(DynamicImage::ImageRgba16(source.clone()), Some(&profile)).unwrap();
        assert_eq!(converted.color(), image::ColorType::Rgba16);
        let converted = converted.into_rgba16();
        for (a, b) in converted.as_raw().iter().zip(source.as_raw()) {
            assert!(a.abs_diff(*b) <= 3);
        }
    }
    #[test]
    fn hdr_preserves_highlights_and_sanitizes_nonfinite_samples() {
        let rgba = image::ImageBuffer::from_raw(
            3,
            1,
            vec![
                0.25,
                0.25,
                0.25,
                1.,
                4.,
                4.,
                4.,
                0.5,
                f32::NAN,
                f32::INFINITY,
                -1.,
                1.,
            ],
        )
        .unwrap();
        let (sdr, hdr) = display(DynamicImage::ImageRgba32F(rgba));
        let hdr = hdr.unwrap();
        assert_eq!(hdr.rgba[4], 4.);
        assert_eq!(hdr.rgba[7], 0.5);
        assert_eq!(&hdr.rgba[8..11], &[0., 0., 0.]);
        assert_eq!(hdr.bytes(), 48);
        // Color32 stores premultiplied channels. Compare the original colors,
        // not an opaque shadow with a half-transparent highlight's stored red.
        let shadow = sdr.pixels[0].to_srgba_unmultiplied();
        let highlight = sdr.pixels[1].to_srgba_unmultiplied();
        assert_eq!(shadow, [124, 124, 124, 255]); // sRGB(0.25 / 1.25)
        for channel in &highlight[..3] {
            // sRGB(4 / 5), allowing one byte for alpha round-trip rounding.
            assert!(channel.abs_diff(231) <= 1);
        }
        assert_eq!(highlight[3], 128);
        assert_eq!(sdr.pixels[2], egui::Color32::BLACK);
    }
    #[test]
    fn pq_and_hlg_reference_points_and_alpha_are_preserved() {
        assert!((pq_eotf(0.5080784) - 100.).abs() < 0.05);
        assert!((pq_eotf(1.) - 10000.).abs() < 1.);
        assert!((hlg_inverse(0.5) - 1. / 12.).abs() < 1e-6);
        let source = image::ImageBuffer::from_pixel(1, 1, image::Rgba([1., 1., 1., 0.25]));
        let hdr = cicp_hdr(DynamicImage::ImageRgba32F(source), 9, 16)
            .unwrap()
            .into_rgba32f();
        assert!((hdr.get_pixel(0, 0)[0] - 10000. / 203.).abs() < 0.1);
        assert_eq!(hdr.get_pixel(0, 0)[3], 0.25);
    }
}
