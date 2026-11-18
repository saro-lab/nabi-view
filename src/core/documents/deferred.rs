//! On-demand native documents. Registry locks never span provider I/O, and
//! readers of one page share an atomic copy rather than observing partial data.
use super::*;
use std::{
    sync::{
        TryLockError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

pub(super) type Loader =
    Arc<dyn Fn(&mut File, u64, &dyn Fn() -> bool) -> Result<(), String> + Send + Sync>;

/// A provider-issued identity for one currently accessible resource. The opaque
/// identifier is hashed for disk keys and never persisted verbatim. Provider
/// policy and OS access checks live in the issuing platform adapter.
pub(crate) struct ProviderCache {
    identity: Option<ProviderIdentity>,
    valid: Arc<dyn Fn() -> bool + Send + Sync>,
    thumbnail: Option<ThumbnailLoader>,
}

/// Optional provider-owned small image; it is never a full-size display frame.
pub(crate) type ThumbnailLoader =
    Arc<dyn Fn(u32, &dyn Fn() -> bool) -> Result<eframe::egui::ColorImage, String> + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderIdentity {
    pub bytes: Vec<u8>,
    pub modified: SystemTime,
}

impl ProviderCache {
    pub(crate) fn new(
        identity: Vec<u8>,
        modified: Option<SystemTime>,
        valid: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Option<Self> {
        let modified = modified.filter(|time| *time > SystemTime::UNIX_EPOCH)?;
        if identity.is_empty() || identity.len() > 8192 {
            return None;
        }
        Some(Self {
            identity: Some(ProviderIdentity {
                bytes: identity,
                modified,
            }),
            valid,
            thumbnail: None,
        })
    }

    /// A currently authorized provider may have no stable change stamp. Keep
    /// its access/thumbnail policy without inventing a persistent cache key.
    pub(crate) fn without_identity(valid: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self {
            identity: None,
            valid,
            thumbnail: None,
        }
    }

    pub(crate) fn with_thumbnail(mut self, loader: ThumbnailLoader) -> Self {
        self.thumbnail = Some(loader);
        self
    }
}

pub(super) struct Budget {
    remaining: u64,
    in_flight: usize,
}

impl Budget {
    pub(super) fn new(remaining: u64) -> Self {
        Self {
            remaining,
            in_flight: 0,
        }
    }
}

struct Entry {
    _directory: Arc<tempfile::TempDir>,
    remaining: Arc<Mutex<Budget>>,
    stamp: (u64, Option<SystemTime>),
    actual_bytes: AtomicU64,
    loader: Loader,
    provider_cache: Option<ProviderCache>,
    loaded: Mutex<bool>,
}

pub(crate) struct DeferredFile(Arc<Entry>);

impl DeferredFile {
    pub(crate) fn materialize(&self, path: &Path, live: &dyn Fn() -> bool) -> Result<(), String> {
        materialize_entry(path, &self.0, live)
    }
}

fn entries() -> &'static Mutex<HashMap<PathBuf, Arc<Entry>>> {
    static ENTRIES: OnceLock<Mutex<HashMap<PathBuf, Arc<Entry>>>> = OnceLock::new();
    ENTRIES.get_or_init(Default::default)
}

#[cfg(any(target_os = "ios", target_os = "android", test))]
pub(super) fn register(
    path: PathBuf,
    directory: Arc<tempfile::TempDir>,
    remaining: Arc<Mutex<Budget>>,
    bytes: u64,
    modified: Option<SystemTime>,
    provider_cache: Option<ProviderCache>,
    loader: Loader,
) {
    entries().lock().unwrap().insert(
        path,
        Arc::new(Entry {
            _directory: directory,
            remaining,
            stamp: (bytes, modified),
            actual_bytes: AtomicU64::new(0),
            loader,
            provider_cache,
            loaded: Mutex::new(false),
        }),
    );
}

pub(super) fn unregister(directory: &Path) {
    entries()
        .lock()
        .unwrap()
        .retain(|path, _| !path.starts_with(directory));
}

/// True for the entire owned batch lifetime, including after its copy is ready.
#[cfg(test)]
pub(crate) fn is_deferred(path: &Path) -> bool {
    entries().lock().unwrap().contains_key(path)
}

/// Allows an asynchronous consumer to keep a source alive while preparing an
/// independent share copy, even after the user opens a different folder.
pub(crate) fn retain_deferred(path: &Path) -> Option<DeferredFile> {
    entries()
        .lock()
        .unwrap()
        .get(path)
        .cloned()
        .map(DeferredFile)
}

/// Stable provider metadata for directory watching/cache identities. A copy
/// becoming ready is not a change to the source image and must not cancel it.
pub(crate) fn deferred_stamp(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    entries().lock().unwrap().get(path).map(|entry| entry.stamp)
}

/// Exact copied size for sorting, separate from the frozen watch/cache stamp.
/// A zero sentinel means no successful copy yet; empty copies are rejected.
pub(crate) fn deferred_size(path: &Path) -> Option<u64> {
    entries()
        .lock()
        .unwrap()
        .get(path)
        .map(|entry| entry.actual_bytes.load(Ordering::Acquire))
        .filter(|&bytes| bytes != 0)
}

/// None means no stable provider cache identity (including ordinary files).
/// Err means a revoked capability and forbids a local-path cache fallback.
pub(crate) fn provider_cache_identity(path: &Path) -> Result<Option<ProviderIdentity>, ()> {
    let entry = entries().lock().unwrap().get(path).cloned();
    let Some(entry) = entry else { return Ok(None) };
    let Some(cache) = &entry.provider_cache else {
        return Ok(None);
    };
    (cache.valid)().then(|| cache.identity.clone()).ok_or(())
}

pub(crate) fn has_provider_thumbnail(path: &Path) -> bool {
    entries().lock().unwrap().get(path).is_some_and(|entry| {
        entry
            .provider_cache
            .as_ref()
            .is_some_and(|cache| cache.thumbnail.is_some())
    })
}

/// Speculative strip warming may use local files or a provider's small-image
/// API, but must not copy another unknown-size original just for a thumbnail.
pub(crate) fn can_warm_thumbnail(path: &Path) -> bool {
    let entry = entries().lock().unwrap().get(path).cloned();
    entry.is_none_or(|entry| {
        entry.actual_bytes.load(Ordering::Acquire) > 0
            || entry
                .provider_cache
                .as_ref()
                .is_some_and(|cache| cache.identity.is_some() && cache.thumbnail.is_some())
    })
}

/// No original-file fallback: failure must not turn speculative thumbnails
/// into large provider copies. Permission and cancellation bracket the call.
pub(crate) fn provider_thumbnail(
    path: &Path,
    max_side: u32,
    live: &dyn Fn() -> bool,
) -> Result<eframe::egui::ColorImage, String> {
    let entry = entries().lock().unwrap().get(path).cloned();
    let cache = entry
        .as_ref()
        .and_then(|entry| entry.provider_cache.as_ref());
    let Some(cache) = cache else {
        return Err("Thumbnail unavailable.".into());
    };
    let Some(loader) = &cache.thumbnail else {
        return Err("Thumbnail unavailable.".into());
    };
    let active = || live() && (cache.valid)();
    if !active() {
        return Err("Cancelled".into());
    }
    let max_side = max_side.clamp(1, 256);
    let image = loader(max_side, &active)?;
    if !active() {
        return Err("Cancelled".into());
    }
    if image.size.contains(&0)
        || image.size.iter().any(|&edge| edge > max_side as usize)
        || image.size[0].checked_mul(image.size[1]) != Some(image.pixels.len())
    {
        return Err("Invalid provider thumbnail.".into());
    }
    Ok(image)
}

/// Materializes only registered private placeholders; ordinary files are a no-op.
pub(crate) fn materialize(path: &Path, live: &dyn Fn() -> bool) -> Result<(), String> {
    let entry = entries().lock().unwrap().get(path).cloned();
    let Some(entry) = entry else {
        return Ok(());
    };
    materialize_entry(path, &entry, live)
}

fn materialize_entry(path: &Path, entry: &Entry, live: &dyn Fn() -> bool) -> Result<(), String> {
    let permitted = || {
        entry
            .provider_cache
            .as_ref()
            .is_none_or(|cache| (cache.valid)())
    };
    // Cancel while a different worker copies this page. Keep the directory
    // alive even if a newer folder replaces the owning ImportBatch meanwhile.
    let mut loaded = loop {
        if !live() || !permitted() {
            return Err("Cancelled".into());
        }
        match entry.loaded.try_lock() {
            Ok(loaded) => break loaded,
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(5)),
            Err(TryLockError::Poisoned(_)) => return Err("Document copy failed.".into()),
        }
    };
    if *loaded {
        return Ok(());
    }
    let allowance = loop {
        if !live() || !permitted() {
            return Err("Cancelled".into());
        }
        let mut remaining = entry.remaining.lock().unwrap();
        let requested = if entry.stamp.0 == 0 {
            FILE_LIMIT
        } else {
            entry.stamp.0.min(FILE_LIMIT)
        };
        // A different copy may be holding its worst-case reservation. Wait
        // for its unused allowance, rather than reporting a false total limit.
        if remaining.in_flight > 0 && remaining.remaining < requested {
            drop(remaining);
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let allowance = remaining.remaining.min(requested);
        if allowance == 0 {
            return Err("Selected images exceed the import size limit.".into());
        }
        remaining.remaining -= allowance;
        remaining.in_flight += 1;
        break allowance;
    };
    let result = (|| {
        let mut temporary = tempfile::NamedTempFile::new_in(entry._directory.path())
            .map_err(|error| error.to_string())?;
        (entry.loader)(temporary.as_file_mut(), allowance, &|| {
            live() && permitted()
        })?;
        if !live() || !permitted() {
            return Err("Cancelled".into());
        }
        let bytes = temporary
            .as_file()
            .metadata()
            .map_err(|error| error.to_string())?
            .len();
        if bytes == 0 {
            return Err("The selected file is empty.".into());
        }
        if bytes > allowance {
            return Err("Selected images exceed the import size limit.".into());
        }
        temporary.persist(path).map_err(|error| error.to_string())?;
        entry.actual_bytes.store(bytes, Ordering::Release);
        *loaded = true;
        Ok(bytes)
    })();
    let mut remaining = entry.remaining.lock().unwrap();
    remaining.remaining += allowance - result.as_ref().copied().unwrap_or(0);
    remaining.in_flight -= 1;
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn share_lease_keeps_original_available_after_folder_replacement() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let mut batch = ImportBatch::new_folder(root.path()).unwrap();
        batch
            .add_deferred(
                "picture.png",
                8,
                None,
                Arc::new(|file, _, _| {
                    file.write_all(b"original")
                        .map_err(|error| error.to_string())
                }),
            )
            .unwrap();
        let path = batch.first().unwrap().to_owned();
        let retained = retain_deferred(&path).unwrap();
        drop(batch);
        assert!(!is_deferred(&path));
        retained.materialize(&path, &|| true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        drop(retained);
        assert!(!path.exists());
    }

    #[test]
    fn pending_reservations_do_not_report_a_false_total_limit() {
        use std::sync::mpsc;
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let mut batch = ImportBatch::new_folder(root.path()).unwrap();
        *batch.deferred_budget.lock().unwrap() = Budget::new(16);
        let (begun, ready) = mpsc::channel();
        let (resume, pending) = mpsc::channel();
        let pending = Mutex::new(pending);
        batch
            .add_deferred(
                "background.png",
                0,
                None,
                Arc::new(move |file, _, _| {
                    begun.send(()).unwrap();
                    pending
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(3))
                        .unwrap();
                    file.write_all(b"original")
                        .map_err(|error| error.to_string())
                }),
            )
            .unwrap();
        batch
            .add_deferred(
                "foreground.png",
                0,
                None,
                Arc::new(|file, _, _| {
                    file.write_all(b"original")
                        .map_err(|error| error.to_string())
                }),
            )
            .unwrap();
        let background = batch.files[0].clone();
        let foreground = batch.files[1].clone();
        let first = std::thread::spawn(move || materialize(&background, &|| true));
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        let second = std::thread::spawn(move || materialize(&foreground, &|| true));
        std::thread::sleep(Duration::from_millis(20));
        resume.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        assert_eq!(batch.deferred_budget.lock().unwrap().remaining, 0);
        assert_eq!(batch.deferred_budget.lock().unwrap().in_flight, 0);
    }

    #[test]
    fn oversized_copy_and_invalid_names_leave_no_partial_image() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let mut batch = ImportBatch::new_folder(root.path()).unwrap();
        batch
            .add_deferred(
                &format!("{}.png", "a".repeat(201)),
                0,
                None,
                Arc::new(|_, _, _| panic!("invalid names must never open the provider")),
            )
            .unwrap();
        assert!(batch.first().is_none());
        assert!(batch.notice().is_some());
        batch
            .add_deferred(
                "large.png",
                4,
                None,
                Arc::new(|file, limit, _| {
                    file.set_len(limit + 1).map_err(|error| error.to_string())
                }),
            )
            .unwrap();
        let path = batch.first().unwrap();
        assert!(materialize(path, &|| true).is_err());
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
        assert_eq!(
            batch.deferred_budget.lock().unwrap().remaining,
            FOLDER_BATCH_LIMIT
        );
        assert_eq!(batch.deferred_budget.lock().unwrap().in_flight, 0);
    }

    #[test]
    fn listing_precedes_provider_reads_and_duplicate_requests_share_one_copy() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let mut batch = ImportBatch::new_folder(root.path()).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        batch
            .add_deferred(
                "large.png",
                8,
                None,
                Arc::new(move |file, _, _| {
                    count.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    file.write_all(b"original")
                        .map_err(|error| error.to_string())
                }),
            )
            .unwrap();
        let path = batch.first().unwrap().to_owned();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        let other = path.clone();
        let worker = std::thread::spawn(move || materialize(&other, &|| true));
        materialize(&path, &|| true).unwrap();
        worker.join().unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(deferred_stamp(&path), Some((8, None)));
        assert!(is_deferred(&path));
        drop(batch);
        assert!(!is_deferred(&path));
        assert!(!path.exists());
    }

    #[test]
    fn failed_or_cancelled_copies_keep_placeholder_and_refund_budget() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let mut batch = ImportBatch::new_folder(root.path()).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        batch
            .add_deferred(
                "large.png",
                0,
                None,
                Arc::new(move |file, _, _| {
                    file.write_all(b"partial").unwrap();
                    if count.fetch_add(1, Ordering::SeqCst) == 0 {
                        Err("Cancelled".into())
                    } else {
                        Ok(())
                    }
                }),
            )
            .unwrap();
        let path = batch.first().unwrap();
        assert!(materialize(path, &|| false).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(materialize(path, &|| true).is_err());
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
        assert_eq!(
            batch.deferred_budget.lock().unwrap().remaining,
            FOLDER_BATCH_LIMIT
        );
        materialize(path, &|| true).unwrap();
        assert_eq!(
            batch.deferred_budget.lock().unwrap().remaining,
            FOLDER_BATCH_LIMIT - 7
        );
    }
}
