//! Optional formats use the system's installed WIC codecs on Windows.
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
pub fn decode(_: &std::path::Path) -> Result<image::DynamicImage, String> {
    Err("This format needs an additional platform codec in this build".into())
}
