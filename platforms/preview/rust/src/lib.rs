//! C ABI for OS preview extensions. No app process, settings, or cache is needed.
use std::{
    ffi::{CStr, c_char},
    path::Path,
    ptr,
};

#[repr(C)]
pub struct NabiPreview {
    pub width: u32,
    pub height: u32,
    pub length: usize,
    pub rgba: *mut u8,
}

fn publish(
    output: *mut NabiPreview,
    work: impl FnOnce() -> Result<nabi_view::preview::Preview, String>,
) -> i32 {
    if output.is_null() {
        return 1;
    }
    // SAFETY: callers provide a writable, empty output structure.
    unsafe {
        ptr::write(
            output,
            NabiPreview {
                width: 0,
                height: 0,
                length: 0,
                rgba: ptr::null_mut(),
            },
        );
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(Ok(image)) => {
            let bytes = image.rgba.into_boxed_slice();
            let length = bytes.len();
            let rgba = Box::into_raw(bytes).cast::<u8>();
            unsafe {
                ptr::write(
                    output,
                    NabiPreview {
                        width: image.width,
                        height: image.height,
                        length,
                        rgba,
                    },
                );
            }
            0
        }
        _ => 2,
    }
}

/// # Safety
/// path is a valid NUL-terminated UTF-8 string; output points to an empty struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nabi_preview_decode_path(
    path: *const c_char,
    size: u32,
    output: *mut NabiPreview,
) -> i32 {
    publish(output, || {
        if path.is_null() {
            return Err("Missing path".into());
        }
        let path = unsafe { CStr::from_ptr(path) }
            .to_str()
            .map_err(|_| "Invalid UTF-8 path")?;
        nabi_view::preview::decode_path(Path::new(path), size)
    })
}

/// # Safety
/// bytes points to length readable bytes, extension is NUL-terminated UTF-8,
/// output is writable and empty. The buffer need not outlive this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nabi_preview_decode_bytes(
    bytes: *const u8,
    length: usize,
    extension: *const c_char,
    size: u32,
    output: *mut NabiPreview,
) -> i32 {
    publish(output, || {
        if bytes.is_null()
            || length == 0
            || length as u64 > nabi_view::preview::MAX_INPUT_BYTES
            || extension.is_null()
        {
            return Err("Invalid preview input".into());
        }
        let bytes = unsafe { std::slice::from_raw_parts(bytes, length) };
        let extension = unsafe { CStr::from_ptr(extension) }
            .to_str()
            .map_err(|_| "Invalid extension")?;
        nabi_view::preview::decode_stream(bytes, extension, size)
    })
}

/// # Safety
/// output is a struct returned by one of the decode functions, not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nabi_preview_free(output: *mut NabiPreview) {
    if output.is_null() {
        return;
    }
    let image = unsafe { &mut *output };
    if !image.rgba.is_null() {
        let slice = ptr::slice_from_raw_parts_mut(image.rgba, image.length);
        unsafe {
            drop(Box::from_raw(slice));
        }
    }
    image.rgba = ptr::null_mut();
    image.length = 0;
    image.width = 0;
    image.height = 0;
}
