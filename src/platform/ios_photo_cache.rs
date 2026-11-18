//! PhotoKit display-cache policy; shared cache code only sees an opaque identity
//! and a current-access guard. This module does not request or expand permission.
use crate::core::{documents::ProviderCache, ios_access::PhotoStatus};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    sync::Arc,
    time::SystemTime,
};

pub(super) fn from_authorized_import(
    source: &str,
    asset: &CStr,
    name: &str,
    resource_type: i32,
    modified: Option<SystemTime>,
    revision: u64,
) -> Option<ProviderCache> {
    if !crate::core::photo_library::is_ios(source)
        || asset.to_bytes().is_empty()
        || name.is_empty()
        || resource_type <= 0
    {
        return None;
    }
    let valid: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
        let status = super::mobile::photo_status();
        revision == super::mobile::access_revision()
            && status.readable()
            && (status == PhotoStatus::Authorized || !super::mobile::photo_validation_pending())
    });
    let Some(modified) = modified.filter(|date| *date > SystemTime::UNIX_EPOCH) else {
        // Unknown change stamps cannot safely identify a Photos rendition. An
        // explicit unavailable thumbnail capability also prevents a visible
        // strip request from silently copying its full original instead.
        return Some(
            ProviderCache::without_identity(valid)
                .with_thumbnail(Arc::new(|_, _| Err("Thumbnail unavailable.".into()))),
        );
    };
    // FullSizePhoto and Photo with the same original name are distinct data.
    // A nullable modificationDate is never replaced by creationDate here.
    let mut identity = b"ios-photokit-resource-v1".to_vec();
    identity.extend(resource_type.to_le_bytes());
    for part in [asset.to_bytes(), name.as_bytes()] {
        identity.extend((part.len() as u64).to_le_bytes());
        identity.extend(part);
    }
    let thumbnail_asset = asset.to_owned();
    let thumbnail_name = CString::new(name).ok()?;
    let modified_seconds = modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    ProviderCache::new(identity, Some(modified), valid).map(|cache| {
        cache.with_thumbnail(Arc::new(move |edge, live| {
            thumbnail(
                &thumbnail_asset,
                &thumbnail_name,
                resource_type,
                modified_seconds,
                edge,
                live,
            )
        }))
    })
}

// The native adapter owns PhotoKit/UIKit work and returns a bounded sRGB small
// image. It never stores a Rust pointer, opens network access or reads originals.
fn thumbnail(
    asset: &CStr,
    name: &CStr,
    resource_type: i32,
    modified: f64,
    edge: u32,
    live: &dyn Fn() -> bool,
) -> Result<eframe::egui::ColorImage, String> {
    unsafe extern "C" {
        fn nabi_photo_thumbnail(
            token: *const c_char,
            name: *const c_char,
            resource_type: i32,
            modified: f64,
            edge: u32,
            rgba: *mut u8,
            capacity: usize,
            width: *mut u32,
            height: *mut u32,
            live: unsafe extern "C" fn(*mut c_void) -> bool,
            context: *mut c_void,
        ) -> i32;
    }
    unsafe extern "C" fn still_live(context: *mut c_void) -> bool {
        let live = unsafe { &*(context.cast::<&dyn Fn() -> bool>()) };
        live()
    }
    let edge = edge.clamp(1, 256);
    let mut rgba = vec![0u8; edge as usize * edge as usize * 4];
    let (mut width, mut height) = (0, 0);
    let mut live = live;
    let result = unsafe {
        nabi_photo_thumbnail(
            asset.as_ptr(),
            name.as_ptr(),
            resource_type,
            modified,
            edge,
            rgba.as_mut_ptr(),
            rgba.len(),
            &mut width,
            &mut height,
            still_live,
            (&mut live as *mut &dyn Fn() -> bool).cast(),
        )
    };
    if result == -1 || !live() {
        return Err("Cancelled".into());
    }
    if result != 0 || width == 0 || height == 0 || width > edge || height > edge {
        return Err("Thumbnail unavailable.".into());
    }
    rgba.truncate(width as usize * height as usize * 4);
    Ok(eframe::egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        &rgba,
    ))
}
