//! Android MediaStore and Storage Access Framework sources, opened on demand.
//! Tokens (a MediaStore volume/ID or a document URI) stay in memory only; disk
//! cache identities hash them, and no URI or grant is persisted here. The Java
//! host owns permission checks and every ContentResolver call.
use crate::core::documents::ProviderCache;
use std::{
    fs::File,
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

/// MediaStore image while the app can read every photo.
pub(crate) const MEDIA_STORE: i32 = 1;
/// A document inside a granted SAF tree.
pub(crate) const DOCUMENT: i32 = 2;
/// MediaStore image under Android 14+ partial ("selected photos") access.
pub(crate) const MEDIA_STORE_PARTIAL: i32 = 3;

/// Bumped when full photo access is lost. MediaStore identities captured
/// before then stop being valid at once, like a revoked iOS capability.
static ACCESS_REVISION: AtomicU64 = AtomicU64::new(0);

pub(crate) fn access_changed() {
    ACCESS_REVISION.fetch_add(1, Ordering::AcqRel);
}

/// Java reports milliseconds since the epoch; zero or less means unknown.
pub(crate) fn modified_time(millis: i64) -> Option<SystemTime> {
    (millis > 0).then(|| SystemTime::UNIX_EPOCH + Duration::from_millis(millis as u64))
}

/// Namespace, version and a fixed hash of the token: bounded in size, stable
/// across releases, and never the URI itself.
fn identity(kind: i32, token: &str, bytes: u64) -> Option<Vec<u8>> {
    let namespace: &[u8] = match kind {
        MEDIA_STORE => b"android-mediastore-v1",
        DOCUMENT => b"android-document-v1",
        // Partial access can change outside the app: no persistent identity.
        _ => return None,
    };
    if token.is_empty() {
        return None;
    }
    let hash = |basis: u64| {
        token.bytes().fold(basis, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    };
    let mut identity = namespace.to_vec();
    identity.extend(hash(0xcbf2_9ce4_8422_2325).to_le_bytes());
    identity.extend(hash(0x6c62_272e_07bb_0142).to_le_bytes());
    identity.extend(bytes.to_le_bytes());
    Some(identity)
}

/// Cache policy for one listed source. Without a change stamp the source still
/// opens and gets OS thumbnails, but never a persistent cache key.
pub(crate) fn provider_cache(
    kind: i32,
    token: &str,
    modified_millis: i64,
    bytes: u64,
) -> ProviderCache {
    let valid: Arc<dyn Fn() -> bool + Send + Sync> = if kind == DOCUMENT {
        // A revoked tree grant fails the read itself.
        Arc::new(|| true)
    } else {
        let revision = ACCESS_REVISION.load(Ordering::Acquire);
        Arc::new(move || ACCESS_REVISION.load(Ordering::Acquire) == revision)
    };
    let cache = identity(kind, token, bytes)
        .and_then(|identity| {
            ProviderCache::new(identity, modified_time(modified_millis), valid.clone())
        })
        .unwrap_or_else(|| ProviderCache::without_identity(valid));
    #[cfg(target_os = "android")]
    if kind == MEDIA_STORE || kind == MEDIA_STORE_PARTIAL {
        let token = token.to_owned();
        return cache.with_thumbnail(Arc::new(move |edge, live| {
            ffi::media_thumbnail(&token, edge, live)
        }));
    }
    cache
}

/// Copies at most `limit + 1` bytes, so the caller can reject an oversized
/// source without reading all of it. Checks cancellation between chunks.
pub(crate) fn copy_limited(
    mut source: impl Read,
    output: &mut File,
    limit: u64,
    live: &dyn Fn() -> bool,
) -> Result<(), String> {
    let mut buffer = vec![0u8; 256 * 1024];
    let mut copied = 0u64;
    loop {
        if !live() {
            return Err("Cancelled".into());
        }
        let read = source
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(());
        }
        copied += read as u64;
        if copied > limit {
            return Err("Selected images exceed the import size limit.".into());
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
    }
}

/// Reads one source when a decoder first needs its pixels.
#[cfg(target_os = "android")]
pub(crate) fn loader(
    kind: i32,
    token: String,
) -> Arc<dyn Fn(&mut File, u64, &dyn Fn() -> bool) -> Result<(), String> + Send + Sync> {
    use std::os::fd::FromRawFd;
    Arc::new(move |output, limit, live| {
        if !live() {
            return Err("Cancelled".into());
        }
        let fd = ffi::open_source(kind, &token);
        if fd < 0 {
            return Err("The selected document could not be opened.".into());
        }
        // The host detaches an owned descriptor; File closes it on every path.
        let source = unsafe { File::from_raw_fd(fd) };
        copy_limited(source, output, limit, live)
    })
}

/// Decodes through Android's ImageDecoder at most `max_side` and `max_pixels`,
/// upright and in sRGB. None lets the caller fall back to the Rust decoders.
#[cfg(target_os = "android")]
pub(crate) fn decode_scaled(
    path: &std::path::Path,
    max_side: u32,
    max_pixels: u64,
) -> Option<(eframe::egui::ColorImage, [u32; 2])> {
    ffi::decode_scaled(path, max_side, max_pixels)
}

#[cfg(target_os = "android")]
mod ffi {
    use eframe::egui;
    use std::ffi::c_void;

    type Alloc = unsafe extern "C" fn(*mut c_void, usize) -> *mut u8;

    unsafe extern "C" {
        fn nabi_android_open_source(kind: i32, token: *const u16, length: usize) -> i32;
        fn nabi_android_media_thumbnail(
            token: *const u16,
            length: usize,
            edge: u32,
            alloc: Alloc,
            context: *mut c_void,
            width: *mut u32,
            height: *mut u32,
        ) -> i32;
        fn nabi_android_decode_scaled(
            path: *const u16,
            path_length: usize,
            max_side: u32,
            max_pixels: u64,
            alloc: Alloc,
            context: *mut c_void,
            width: *mut u32,
            height: *mut u32,
            original: *mut u32,
        ) -> i32;
    }

    /// Output buffer that the native side fills; refuses more than `limit`.
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

    fn image(target: Target, width: u32, height: u32) -> Option<egui::ColorImage> {
        let size = [width as usize, height as usize];
        (width > 0 && height > 0 && target.buffer.len() == size[0] * size[1] * 4)
            // Android ARGB_8888 bitmaps hold premultiplied RGBA bytes.
            .then(|| egui::ColorImage::from_rgba_premultiplied(size, &target.buffer))
    }

    pub(super) fn open_source(kind: i32, token: &str) -> i32 {
        let token: Vec<u16> = token.encode_utf16().collect();
        unsafe { nabi_android_open_source(kind, token.as_ptr(), token.len()) }
    }

    pub(super) fn media_thumbnail(
        token: &str,
        edge: u32,
        live: &dyn Fn() -> bool,
    ) -> Result<egui::ColorImage, String> {
        if !live() {
            return Err("Cancelled".into());
        }
        let edge = edge.clamp(1, 256);
        let token: Vec<u16> = token.encode_utf16().collect();
        let mut target = Target {
            buffer: Vec::new(),
            limit: edge as usize * edge as usize * 4,
        };
        let (mut width, mut height) = (0, 0);
        let result = unsafe {
            nabi_android_media_thumbnail(
                token.as_ptr(),
                token.len(),
                edge,
                alloc,
                (&mut target as *mut Target).cast(),
                &mut width,
                &mut height,
            )
        };
        if !live() {
            return Err("Cancelled".into());
        }
        if result != 0 || width > edge || height > edge {
            return Err("Thumbnail unavailable.".into());
        }
        image(target, width, height).ok_or_else(|| "Thumbnail unavailable.".into())
    }

    pub(super) fn decode_scaled(
        path: &std::path::Path,
        max_side: u32,
        max_pixels: u64,
    ) -> Option<(egui::ColorImage, [u32; 2])> {
        let path: Vec<u16> = path.to_str()?.encode_utf16().collect();
        let mut target = Target {
            buffer: Vec::new(),
            limit: usize::try_from(max_pixels.saturating_mul(4)).ok()?,
        };
        let (mut width, mut height) = (0, 0);
        let mut original = [0u32; 2];
        let result = unsafe {
            nabi_android_decode_scaled(
                path.as_ptr(),
                path.len(),
                max_side,
                max_pixels,
                alloc,
                (&mut target as *mut Target).cast(),
                &mut width,
                &mut height,
                original.as_mut_ptr(),
            )
        };
        if result != 0 || width > max_side || height > max_side || original.contains(&0) {
            return None;
        }
        Some((image(target, width, height)?, original))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::documents::{ImportBatch, provider_cache_identity};

    /// `access_changed` is process-wide; media identity checks must not race it.
    static ACCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn register(
        root: &std::path::Path,
        kind: i32,
        token: &str,
        modified: i64,
    ) -> (ImportBatch, std::path::PathBuf) {
        let mut batch = ImportBatch::new_folder(root).unwrap();
        batch
            .add_deferred_with_cache(
                "photo.jpg",
                1234,
                modified_time(modified),
                Some(provider_cache(kind, token, modified, 1234)),
                Arc::new(|file, _, _| file.write_all(b"jpeg").map_err(|e| e.to_string())),
            )
            .unwrap();
        let path = batch.first().unwrap().to_owned();
        (batch, path)
    }

    #[test]
    fn same_android_source_keeps_its_disk_key_across_imports() {
        let _serial = ACCESS.lock().unwrap_or_else(|poison| poison.into_inner());
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let cache = crate::core::disk_cache::Cache::new(root.path().join("cache"), 8 * 1024 * 1024);
        let key = |kind, token, modified| {
            let (batch, path) = register(root.path(), kind, token, modified);
            let name = cache.key(&path, 2048).map(|key| key.name().to_owned());
            drop(batch);
            name
        };
        let first = key(MEDIA_STORE, "external_primary\n42", 1_700_000_000_000);
        assert!(first.is_some());
        // A new import makes a new placeholder path; the key stays the same.
        assert_eq!(
            first,
            key(MEDIA_STORE, "external_primary\n42", 1_700_000_000_000)
        );
        // An edit, another item, or another namespace is a different entry.
        assert_ne!(
            first,
            key(MEDIA_STORE, "external_primary\n42", 1_700_000_001_000)
        );
        assert_ne!(
            first,
            key(MEDIA_STORE, "external_primary\n43", 1_700_000_000_000)
        );
        assert_ne!(
            first,
            key(DOCUMENT, "external_primary\n42", 1_700_000_000_000)
        );
    }

    #[test]
    fn unknown_stamp_or_partial_access_has_no_persistent_identity() {
        let _serial = ACCESS.lock().unwrap_or_else(|poison| poison.into_inner());
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let (_batch, unknown) = register(root.path(), MEDIA_STORE, "external\n7", 0);
        assert_eq!(provider_cache_identity(&unknown), Ok(None));
        let (_partial_batch, partial) = register(
            root.path(),
            MEDIA_STORE_PARTIAL,
            "external\n7",
            1_700_000_000_000,
        );
        assert_eq!(provider_cache_identity(&partial), Ok(None));
        let (_document_batch, document) = register(
            root.path(),
            DOCUMENT,
            "content://provider/document/7",
            1_700_000_000_000,
        );
        let identity = provider_cache_identity(&document).unwrap().unwrap();
        // The URI itself never appears in the identity.
        assert!(
            !identity
                .bytes
                .windows(b"content://".len())
                .any(|window| window == b"content://")
        );
    }

    #[test]
    fn losing_full_photo_access_invalidates_media_but_not_documents() {
        let _serial = ACCESS.lock().unwrap_or_else(|poison| poison.into_inner());
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let (_media_batch, media) =
            register(root.path(), MEDIA_STORE, "external\n9", 1_700_000_000_000);
        let (_document_batch, document) =
            register(root.path(), DOCUMENT, "content://p/d/9", 1_700_000_000_000);
        assert!(provider_cache_identity(&media).unwrap().is_some());
        access_changed();
        assert_eq!(provider_cache_identity(&media), Err(()));
        assert!(provider_cache_identity(&document).unwrap().is_some());
    }

    #[test]
    fn on_demand_copy_stops_at_the_limit_and_on_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let mut output = File::create(dir.path().join("copy")).unwrap();
        copy_limited(&b"12345678"[..], &mut output, 8, &|| true).unwrap();
        assert!(copy_limited(&b"123456789"[..], &mut output, 8, &|| true).is_err());
        assert_eq!(
            copy_limited(&b"1"[..], &mut output, 8, &|| false),
            Err("Cancelled".into())
        );
    }
}
