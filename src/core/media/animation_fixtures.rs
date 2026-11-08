//! Deterministic test inputs, without requiring personal sample files.
use libavif_sys as ffi;
use std::{io::Write, path::Path, sync::OnceLock};

pub(crate) fn avif_sample() -> tempfile::TempPath {
    static DATA: OnceLock<Vec<u8>> = OnceLock::new();
    let bytes = DATA.get_or_init(|| avif_bytes(498, 286, 22));
    let mut file = tempfile::Builder::new().suffix(".avif").tempfile().unwrap();
    file.write_all(bytes).unwrap();
    file.into_temp_path()
}

pub(crate) fn avif_bytes(width: u32, height: u32, count: usize) -> Vec<u8> {
    struct Native {
        encoder: *mut ffi::avifEncoder,
        image: *mut ffi::avifImage,
        output: ffi::avifRWData,
    }
    impl Drop for Native {
        fn drop(&mut self) {
            // SAFETY: these resources are exclusively owned by this fixture.
            unsafe {
                ffi::avifRWDataFree(&mut self.output);
                if !self.image.is_null() {
                    ffi::avifImageDestroy(self.image);
                }
                if !self.encoder.is_null() {
                    ffi::avifEncoderDestroy(self.encoder);
                }
            }
        }
    }
    // SAFETY: all pointers stay local, dimensions match the Rust pixel buffer,
    // RGBToYUV copies pixels, and the encoded output is copied before cleanup.
    unsafe {
        let mut native = Native {
            encoder: ffi::avifEncoderCreate(),
            image: ffi::avifImageCreate(width, height, 8, ffi::AVIF_PIXEL_FORMAT_YUV444),
            output: ffi::avifRWData::default(),
        };
        assert!(!native.encoder.is_null() && !native.image.is_null());
        (*native.encoder).maxThreads = 1;
        (*native.encoder).speed = 10;
        (*native.encoder).timescale = 1000;
        for index in 0..count {
            let mut pixels = image::RgbaImage::from_fn(width, height, |x, y| {
                image::Rgba([
                    40u8.wrapping_add((index * 7) as u8),
                    (x % 256) as u8,
                    (y % 256) as u8,
                    255,
                ])
            });
            let mut rgb = ffi::avifRGBImage::default();
            ffi::avifRGBImageSetDefaults(&mut rgb, native.image);
            rgb.format = ffi::AVIF_RGB_FORMAT_RGBA;
            rgb.rowBytes = width * 4;
            rgb.pixels = pixels.as_mut_ptr();
            assert_eq!(
                ffi::avifImageRGBToYUV(native.image, &rgb),
                ffi::AVIF_RESULT_OK
            );
            assert_eq!(
                ffi::avifEncoderAddImage(
                    native.encoder,
                    native.image,
                    140,
                    ffi::AVIF_ADD_IMAGE_FLAG_NONE
                ),
                ffi::AVIF_RESULT_OK
            );
        }
        assert_eq!(
            ffi::avifEncoderFinish(native.encoder, &mut native.output),
            ffi::AVIF_RESULT_OK
        );
        std::slice::from_raw_parts(native.output.data, native.output.size).to_vec()
    }
}

pub(crate) fn gif(path: &Path, count: usize, size: u32) {
    let mut encoder = image::codecs::gif::GifEncoder::new(std::fs::File::create(path).unwrap());
    for index in 0..count {
        encoder
            .encode_frame(image::Frame::from_parts(
                image::RgbaImage::from_pixel(
                    size,
                    size,
                    image::Rgba([index as u8, (index / 256) as u8, 0, 255]),
                ),
                0,
                0,
                image::Delay::from_numer_denom_ms(10 + (index % 3) as u32 * 10, 1),
            ))
            .unwrap();
    }
}

pub(crate) fn apng(path: &Path) {
    fn chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut crc = flate2::Crc::new();
        crc.update(tag);
        crc.update(payload);
        [
            &(payload.len() as u32).to_be_bytes()[..],
            tag,
            payload,
            &crc.sum().to_be_bytes(),
        ]
        .concat()
    }
    let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
    data.extend(chunk(b"IHDR", &[0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0, 0, 0]));
    data.extend(chunk(b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]));
    for index in 0..2u8 {
        let sequence = if index == 0 { 0u32 } else { 1 };
        let control = [
            &sequence.to_be_bytes()[..],
            &2u32.to_be_bytes(),
            &2u32.to_be_bytes(),
            &[0; 8],
            &((index as u16 + 1) * 100).to_be_bytes(),
            &1000u16.to_be_bytes(),
            &[0, 0],
        ]
        .concat();
        data.extend(chunk(b"fcTL", &control));
        let mut compressed =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        for _ in 0..2 {
            compressed.write_all(&[0]).unwrap();
            for _ in 0..2 {
                compressed
                    .write_all(&[255 * (1 - index), 0, 255 * index, 255])
                    .unwrap();
            }
        }
        let pixels = compressed.finish().unwrap();
        if index == 0 {
            data.extend(chunk(b"IDAT", &pixels));
        } else {
            data.extend(chunk(b"fdAT", &[&2u32.to_be_bytes()[..], &pixels].concat()));
        }
    }
    data.extend(chunk(b"IEND", &[]));
    std::fs::write(path, data).unwrap();
}

pub(crate) fn webp(path: &Path) {
    use image::ImageEncoder;
    fn chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut result = [
            tag.as_slice(),
            &(payload.len() as u32).to_le_bytes(),
            payload,
        ]
        .concat();
        if !payload.len().is_multiple_of(2) {
            result.push(0);
        }
        result
    }
    let mut body = b"WEBP".to_vec();
    body.extend(chunk(b"VP8X", &[2, 0, 0, 0, 1, 0, 0, 1, 0, 0]));
    body.extend(chunk(b"ANIM", &[0; 6]));
    for index in 0..2u8 {
        let pixels = [255 * (1 - index), 0, 255 * index, 255].repeat(4);
        let mut encoded = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)
            .write_image(&pixels, 2, 2, image::ExtendedColorType::Rgba8)
            .unwrap();
        let mut frame = vec![
            0,
            0,
            0,
            0,
            0,
            0,
            1,
            0,
            0,
            1,
            0,
            0,
            (index + 1) * 100,
            0,
            0,
            2,
        ];
        frame.extend_from_slice(&encoded[12..]);
        body.extend(chunk(b"ANMF", &frame));
    }
    std::fs::write(
        path,
        [
            b"RIFF".as_slice(),
            &(body.len() as u32).to_le_bytes(),
            &body,
        ]
        .concat(),
    )
    .unwrap();
}
