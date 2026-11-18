//! Optional formats use the system's installed WIC codecs on Windows.

/// Upright pixels scaled while decoding, with the upright original size.
pub struct Scaled {
    pub image: image::RgbaImage,
    pub original: [u32; 2],
    /// Embedded profile still to apply; None means the pixels are sRGB.
    pub icc: Option<Vec<u8>>,
}

/// Output size for a `w` x `h` source within `max_side` and `max_pixels`.
#[cfg(windows)]
fn scaled_size(w: u32, h: u32, max_side: u32, max_pixels: u64) -> [u32; 2] {
    let scale = (f64::from(max_side) / f64::from(w.max(h)))
        .min((max_pixels as f64 / (f64::from(w) * f64::from(h))).sqrt())
        .min(1.0);
    [
        ((f64::from(w) * scale).floor() as u32).max(1),
        ((f64::from(h) * scale).floor() as u32).max(1),
    ]
}

/// WIC's own decode-time scaling (the JPEG codec scales in the DCT), then
/// EXIF orientation from metadata. None when no installed codec reads it.
/// COM and the factory are created once per worker thread.
#[cfg(windows)]
pub fn decode_scaled(
    path: &std::path::Path,
    max_side: u32,
    max_pixels: u64,
) -> Result<Option<Scaled>, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{Graphics::Imaging::*, System::Com::*},
        core::{PCWSTR, w},
    };
    struct Com(bool);
    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() }
            }
        }
    }
    thread_local! {
        static WIC: (Com, Option<IWICImagingFactory>) = unsafe {
            // S_OK and S_FALSE both need a matching uninitialize; a thread
            // already in another apartment can still use the in-process factory.
            let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            (
                Com(initialized),
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok(),
            )
        };
    }
    let Some(factory) = WIC.with(|(_, factory)| factory.clone()) else {
        return Ok(None);
    };
    unsafe {
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let Ok(decoder) = factory.CreateDecoderFromFilename(
            PCWSTR(name.as_ptr()),
            None,
            windows::Win32::Foundation::GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        ) else {
            return Ok(None);
        };
        let frame = decoder.GetFrame(0).map_err(|e| e.to_string())?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h).map_err(|e| e.to_string())?;
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 256_000_000 {
            return Err("Image dimensions exceed the decoding limit.".into());
        }
        // CMYK needs its own conversion; the Rust decoders handle it.
        if frame.GetPixelFormat().ok() == Some(GUID_WICPixelFormat32bppCMYK) {
            return Ok(None);
        }
        // WIC does not color-manage: hand the embedded profile to the caller.
        let icc = (|| {
            let mut count = 0;
            frame.GetColorContexts(&mut [], &mut count).ok()?;
            let mut contexts: Vec<Option<IWICColorContext>> = (0..count.min(4))
                .map(|_| factory.CreateColorContext().ok())
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .map(Some)
                .collect();
            frame.GetColorContexts(&mut contexts, &mut count).ok()?;
            contexts.into_iter().flatten().find_map(|context| {
                if context.GetType().ok()? != WICColorContextProfile {
                    return None;
                }
                let mut length = 0;
                context.GetProfileBytes(&mut [], &mut length).ok()?;
                if length == 0 || length > 4 * 1024 * 1024 {
                    return None;
                }
                let mut profile = vec![0; length as usize];
                context.GetProfileBytes(&mut profile, &mut length).ok()?;
                Some(profile)
            })
        })();
        let orientation = frame
            .GetMetadataQueryReader()
            .ok()
            .and_then(|reader| {
                let mut value =
                    windows::Win32::System::Com::StructuredStorage::PROPVARIANT::default();
                reader
                    .GetMetadataByName(w!("System.Photo.Orientation"), &mut value)
                    .ok()?;
                let value = &value.Anonymous.Anonymous;
                (value.vt == windows::Win32::System::Variant::VT_UI2).then(|| value.Anonymous.uiVal)
            })
            .and_then(|value| u8::try_from(value).ok())
            .and_then(image::metadata::Orientation::from_exif)
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let [out_w, out_h] = scaled_size(w, h, max_side, max_pixels);
        let source: IWICBitmapSource = if [out_w, out_h] == [w, h] {
            frame.into()
        } else {
            let scaler = factory.CreateBitmapScaler().map_err(|e| e.to_string())?;
            scaler
                .Initialize(&frame, out_w, out_h, WICBitmapInterpolationModeFant)
                .map_err(|e| e.to_string())?;
            scaler.into()
        };
        let converter = factory.CreateFormatConverter().map_err(|e| e.to_string())?;
        converter
            .Initialize(
                &source,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(|e| e.to_string())?;
        let mut pixels = vec![0; out_w as usize * out_h as usize * 4];
        converter
            .CopyPixels(std::ptr::null(), out_w * 4, &mut pixels)
            .map_err(|e| e.to_string())?;
        let mut image = image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(out_w, out_h, pixels).ok_or("Invalid pixel buffer")?,
        );
        image.apply_orientation(orientation);
        let swapped = matches!(
            orientation,
            image::metadata::Orientation::Rotate90
                | image::metadata::Orientation::Rotate270
                | image::metadata::Orientation::Rotate90FlipH
                | image::metadata::Orientation::Rotate270FlipH
        );
        Ok(Some(Scaled {
            image: image.into_rgba8(),
            original: if swapped { [h, w] } else { [w, h] },
            icc,
        }))
    }
}

#[cfg(windows)]
pub fn decode(path: &std::path::Path) -> Result<image::DynamicImage, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{Graphics::Imaging::*, System::Com::*},
        core::PCWSTR,
    };
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe { CoUninitialize() }
            }
        }
        let _com = Com;
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let decoder = factory
            .CreateDecoderFromFilename(
                PCWSTR(name.as_ptr()),
                None,
                windows::Win32::Foundation::GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(|e| format!("System codec unavailable or invalid image: {e}"))?;
        let frame = decoder.GetFrame(0).map_err(|e| e.to_string())?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h).map_err(|e| e.to_string())?;
        if w == 0 || h == 0 || w > 32768 || h > 32768 || u64::from(w) * u64::from(h) > 64_000_000 {
            return Err("Image exceeds pixel limit".into());
        }
        let converter = factory.CreateFormatConverter().map_err(|e| e.to_string())?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(|e| e.to_string())?;
        let mut pixels = vec![0; w as usize * h as usize * 4];
        converter
            .CopyPixels(std::ptr::null(), w * 4, &mut pixels)
            .map_err(|e| e.to_string())?;
        Ok(image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(w, h, pixels).ok_or("Invalid pixel buffer")?,
        ))
    }
}
/// ImageIO's thumbnail path scales while decoding and applies the EXIF
/// transform; pixels are drawn once into a Rust buffer in sRGB.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn decode_scaled(
    path: &std::path::Path,
    max_side: u32,
    max_pixels: u64,
) -> Result<Option<Scaled>, String> {
    use std::{ffi::c_void, os::unix::ffi::OsStrExt};
    type Alloc = unsafe extern "C" fn(*mut c_void, usize) -> *mut u8;
    unsafe extern "C" {
        fn nabi_decode_image_scaled(
            path: *const std::ffi::c_char,
            max_side: u32,
            max_pixels: u64,
            alloc: Alloc,
            context: *mut c_void,
            width: *mut u32,
            height: *mut u32,
            original: *mut u32,
        ) -> i32;
    }
    struct Target {
        buffer: Vec<u8>,
        limit: usize,
    }
    unsafe extern "C" fn alloc(context: *mut c_void, length: usize) -> *mut u8 {
        let target = unsafe { &mut *context.cast::<Target>() };
        if length == 0 || length > target.limit {
            return std::ptr::null_mut();
        }
        target.buffer.resize(length, 0);
        target.buffer.as_mut_ptr()
    }
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut target = Target {
        buffer: Vec::new(),
        // ImageIO rounds its longest edge; allow a sliver over the budget.
        limit: usize::try_from(max_pixels.saturating_mul(4).saturating_add(max_pixels / 25))
            .unwrap_or(usize::MAX),
    };
    let (mut width, mut height, mut original) = (0, 0, [0u32; 2]);
    let result = unsafe {
        nabi_decode_image_scaled(
            path.as_ptr(),
            max_side,
            max_pixels,
            alloc,
            (&mut target as *mut Target).cast(),
            &mut width,
            &mut height,
            original.as_mut_ptr(),
        )
    };
    if result != 0 || original.contains(&0) {
        return Ok(None);
    }
    let Some(image) = image::RgbaImage::from_raw(width, height, target.buffer) else {
        return Ok(None);
    };
    Ok(Some(Scaled {
        image,
        original,
        icc: None,
    }))
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn decode(path: &std::path::Path) -> Result<image::DynamicImage, String> {
    unsafe extern "C" {
        fn nabi_decode_image(
            path: *const std::ffi::c_char,
            pixels: *mut *mut u8,
            w: *mut u32,
            h: *mut u32,
        ) -> i32;
        fn nabi_free_image(pixels: *mut u8);
    }
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let (mut pixels, mut w, mut h) = (std::ptr::null_mut(), 0, 0);
    unsafe {
        if nabi_decode_image(path.as_ptr(), &mut pixels, &mut w, &mut h) != 0 {
            return Err("System codec unavailable or invalid image".into());
        }
        let data = std::slice::from_raw_parts(pixels, w as usize * h as usize * 4).to_vec();
        nabi_free_image(pixels);
        Ok(image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(w, h, data).ok_or("Invalid pixel buffer")?,
        ))
    }
}
#[cfg(not(any(windows, target_os = "macos", target_os = "ios")))]
pub fn decode_scaled(_: &std::path::Path, _: u32, _: u64) -> Result<Option<Scaled>, String> {
    Ok(None)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "ios")))]
pub fn decode(_: &std::path::Path) -> Result<image::DynamicImage, String> {
    Err("This format needs an additional platform codec in this build".into())
}
