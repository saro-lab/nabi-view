//! Bounded, disposable image cache. All filesystem work belongs on a worker.
//! Cache files never contain source paths and never replace user documents.
use eframe::egui;
use image::{ImageDecoder, ImageEncoder};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// Display entries: a 24-byte header followed by a QOI image (lossless RGBA).
const MAGIC: &[u8; 8] = b"NABIC002";
const MAX_PIXELS: u64 = 6 * 1024 * 1024;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const KEY_VERSION: &[u8] = b"display-rgba-v4-qoi";
/// Re-list the directories after this many writes even when the running total
/// stays under the limit, so other processes' writes cannot drift unnoticed.
const RESCAN_WRITES: u32 = 64;

/// Running size estimate per cache root, shared by every `Cache` value in this
/// process. A missing root means unknown: the next write lists the directories.
static USAGE: Mutex<Option<HashMap<PathBuf, Usage>>> = Mutex::new(None);

#[derive(Clone, Copy)]
struct Usage {
    bytes: u64,
    writes: u32,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Cache {
    root: PathBuf,
    limit: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: SystemTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SourceIdentity {
    File(Stamp),
    /// An extracted archive page: archive path and inner path, with the
    /// archive's stamp. A new extraction folder keeps the same key.
    Archive {
        archive: PathBuf,
        inner: PathBuf,
        stamp: Stamp,
    },
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    Provider(crate::core::documents::ProviderIdentity),
}

/// Source identity captured before decoding, invalidated by edits and clear-all.
pub(crate) struct Key {
    name: String,
    source: PathBuf,
    identity: SourceIdentity,
    generation: u64,
}

impl Key {
    /// Whether two keys name the same cache entry (for de-duplicating writes).
    pub(crate) fn same_entry(&self, other: &Key) -> bool {
        self.name == other.name && self.generation == other.generation
    }

    #[cfg(test)]
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

pub(crate) struct CachedImage {
    pub image: egui::ColorImage,
    pub original_size: [u32; 2],
}

#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Stats {
    pub files: u64,
    pub bytes: u64,
}

impl Cache {
    /// `root` is the application's versioned cache directory, shared by windows.
    pub fn new(root: PathBuf, limit_bytes: u64) -> Self {
        Self {
            root,
            limit: limit_bytes,
        }
    }

    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Lock-free: entries are published by atomic rename and checked against
    /// the clear generation, so readers never wait for writers or pruning.
    pub fn key(&self, source: &Path, max_side: u32) -> Option<Key> {
        self.make_key(source, max_side, KEY_VERSION)
    }

    /// Provider-rendered small images must never satisfy a full display request,
    /// even when both happen to ask for the same number of pixels.
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    pub fn provider_thumbnail_key(&self, source: &Path, max_side: u32) -> Option<Key> {
        self.make_key(source, max_side, b"provider-thumbnail-srgba-v1")
    }

    fn make_key(&self, source: &Path, max_side: u32, version: &[u8]) -> Option<Key> {
        if self.limit == 0 {
            return None;
        }
        let source = crate::core::paths::canonicalize(source).ok()?;
        let identity = source_identity(&source)?;
        let (mut bytes, modified) = match &identity {
            SourceIdentity::File(stamp) => {
                let mut bytes = source.as_os_str().as_encoded_bytes().to_vec();
                bytes.extend(stamp.size.to_le_bytes());
                (bytes, stamp.modified)
            }
            SourceIdentity::Archive {
                archive,
                inner,
                stamp,
            } => {
                let mut bytes = b"archive-page-v1".to_vec();
                for part in [archive.as_os_str(), inner.as_os_str()] {
                    let part = part.as_encoded_bytes();
                    bytes.extend((part.len() as u64).to_le_bytes());
                    bytes.extend(part);
                }
                bytes.extend(stamp.size.to_le_bytes());
                (bytes, stamp.modified)
            }
            #[cfg(any(target_os = "ios", target_os = "android", test))]
            SourceIdentity::Provider(provider) => (provider.bytes.clone(), provider.modified),
        };
        bytes.extend(
            modified
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_nanos()
                .to_le_bytes(),
        );
        bytes.extend(max_side.to_le_bytes());
        bytes.extend(version);
        let hash = |basis: u64| {
            bytes.iter().fold(basis, |hash, &byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        };
        Some(Key {
            name: format!(
                "{:016x}{:016x}",
                hash(0xcbf2_9ce4_8422_2325),
                hash(0x6c62_272e_07bb_0142)
            ),
            source,
            identity,
            generation: self.generation(),
        })
    }

    /// Whether a current entry exists, without reading its pixels.
    pub fn contains(&self, key: &Key) -> bool {
        self.limit != 0 && self.current(key) && self.path(key).is_file()
    }

    /// Corrupt, stale and unsupported entries are ordinary misses. Lock-free:
    /// a concurrent clear is caught by the generation check after reading.
    pub fn load(&self, key: &Key) -> Option<CachedImage> {
        if self.limit == 0 || !self.current(key) {
            return None;
        }
        let path = self.path(key);
        match read_image(&path) {
            Ok(image) => {
                // Clear-all, permission or content revision may change while
                // decoding the entry.
                if !self.current(key) {
                    return None;
                }
                let _ = touch(&path);
                Some(image)
            }
            Err(error) => {
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
                ) {
                    // Entries are published by atomic rename, so this is
                    // truly corrupt rather than a write in progress.
                    let _ = fs::remove_file(path);
                }
                None
            }
        }
    }

    pub fn store(
        &self,
        key: &Key,
        image: &egui::ColorImage,
        original_size: [u32; 2],
    ) -> io::Result<()> {
        let [width, height] = image.size;
        let pixels = width.checked_mul(height).ok_or_else(invalid)?;
        if pixels == 0
            || pixels as u64 > MAX_PIXELS
            || pixels != image.pixels.len()
            || original_size.contains(&0)
            || original_size.iter().any(|&v| v > 262_144)
            || u64::from(original_size[0]) * u64::from(original_size[1]) > 1_000_000_000
        {
            return Ok(());
        }
        // Encode outside the cross-process lock; another window may be reading.
        let mut rgba = Vec::with_capacity(pixels * 4);
        for pixel in &image.pixels {
            rgba.extend(pixel.to_srgba_unmultiplied());
        }
        let mut encoded = Vec::new();
        image::codecs::qoi::QoiEncoder::new(&mut encoded)
            .write_image(
                &rgba,
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(io::Error::other)?;
        if encoded.len() as u64 + 24 > self.limit.min(MAX_FILE) {
            return Ok(());
        }
        let _lock = self.lock()?;
        if !self.current(key) {
            return Ok(());
        }
        let directory = self.root.join("display");
        fs::create_dir_all(&directory)?;
        let target = self.path(key);
        let replaced = fs::metadata(&target).map_or(0, |meta| meta.len());
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary.write_all(MAGIC)?;
        for value in [
            original_size[0],
            original_size[1],
            width as u32,
            height as u32,
        ] {
            temporary.write_all(&value.to_le_bytes())?;
        }
        temporary.write_all(&encoded)?;
        temporary.persist(&target).map_err(|error| error.error)?;
        self.after_write_locked(24 + encoded.len() as u64, replaced)
    }

    fn current(&self, key: &Key) -> bool {
        key.generation == self.generation()
            && source_identity(&key.source).as_ref() == Some(&key.identity)
    }

    /// Capture before thumbnail decoding, so clear-all also invalidates work
    /// that is still preparing its pixels outside the cross-process lock.
    pub fn thumbnail_generation(&self) -> io::Result<u64> {
        let _lock = self.lock()?;
        Ok(self.generation())
    }

    /// Lock-free generation for decoders; `store_thumbnail` re-checks it
    /// under the lock, so a clear during decoding still discards the result.
    pub fn current_generation(&self) -> u64 {
        self.generation()
    }

    /// The library thumbnail store, shared by the viewer's strip.
    pub fn thumbnail_dir(&self) -> PathBuf {
        self.root.join("thumbnails")
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// This process's running size estimate; None until a write or a prune
    /// has listed the directories.
    pub fn estimated_bytes(&self) -> Option<u64> {
        USAGE
            .lock()
            .unwrap()
            .as_ref()?
            .get(&self.root)
            .map(|usage| usage.bytes)
    }

    /// Publish a thumbnail only while its generation and current policy allow
    /// it. Returning false leaves the decoded pixels usable without a cache key.
    pub fn store_thumbnail(
        &self,
        generation: u64,
        key: &str,
        png: &[u8],
        current_limit: impl FnOnce() -> u64,
    ) -> io::Result<bool> {
        if !crate::core::library::is_thumbnail_key(key) {
            return Err(invalid());
        }
        let _lock = self.lock()?;
        let limit = current_limit();
        if generation != self.generation() || limit == 0 || png.len() as u64 > limit.min(MAX_FILE) {
            return Ok(false);
        }
        let directory = self.root.join("thumbnails");
        fs::create_dir_all(&directory)?;
        let target = crate::core::library::thumbnail_file(&directory, key);
        let replaced = fs::metadata(&target).map_or(0, |meta| meta.len());
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary.write_all(png)?;
        temporary.persist(&target).map_err(|error| error.error)?;
        Self::new(self.root.clone(), limit).after_write_locked(png.len() as u64, replaced)?;
        Ok(true)
    }

    /// Includes display images and the migrated library thumbnail cache.
    pub fn stats(&self) -> io::Result<Stats> {
        let _lock = self.lock()?;
        Ok(self
            .entries()?
            .into_iter()
            .fold(Stats::default(), |mut total, entry| {
                total.files += 1;
                total.bytes = total.bytes.saturating_add(entry.bytes);
                total
            }))
    }

    pub fn prune(&self) -> io::Result<()> {
        let _lock = self.lock()?;
        self.prune_locked()
    }

    pub fn clear(&self) -> io::Result<()> {
        let _lock = self.lock()?;
        let generation = self.generation().wrapping_add(1);
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)?;
        temporary.write_all(&generation.to_le_bytes())?;
        temporary
            .persist(self.root.join("generation"))
            .map_err(|error| error.error)?;
        self.set_usage(None);
        for entry in self.entries()? {
            fs::remove_file(entry.path)?;
        }
        self.set_usage(Some(0));
        Ok(())
    }

    fn path(&self, key: &Key) -> PathBuf {
        self.root.join("display").join(format!("{}.nvic", key.name))
    }

    fn generation(&self) -> u64 {
        fs::read(self.root.join("generation"))
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
            .unwrap_or(0)
    }

    fn lock_file(&self) -> io::Result<File> {
        fs::create_dir_all(&self.root)?;
        File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("cache.lock"))
    }

    fn lock(&self) -> io::Result<File> {
        let file = self.lock_file()?;
        file.lock()?;
        Ok(file)
    }

    /// Updates the running size after a write (lock held). The directories
    /// are listed only when the estimate is unknown, over the limit, or stale.
    fn after_write_locked(&self, added: u64, replaced: u64) -> io::Result<()> {
        let needs_scan = {
            let mut usage = USAGE.lock().unwrap();
            match usage.get_or_insert_with(HashMap::new).get_mut(&self.root) {
                Some(entry) => {
                    entry.bytes = entry.bytes.saturating_sub(replaced).saturating_add(added);
                    entry.writes += 1;
                    entry.bytes > self.limit || entry.writes >= RESCAN_WRITES
                }
                None => true,
            }
        };
        if needs_scan {
            self.prune_locked()
        } else {
            Ok(())
        }
    }

    fn set_usage(&self, bytes: Option<u64>) {
        let mut usage = USAGE.lock().unwrap();
        let map = usage.get_or_insert_with(HashMap::new);
        match bytes {
            Some(bytes) => {
                map.insert(self.root.clone(), Usage { bytes, writes: 0 });
            }
            None => {
                map.remove(&self.root);
            }
        }
    }

    fn entries(&self) -> io::Result<Vec<Entry>> {
        let mut entries = Vec::new();
        for directory in ["display", "thumbnails"] {
            let contents = match fs::read_dir(self.root.join(directory)) {
                Ok(contents) => contents,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            for entry in contents {
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                let owned = if directory == "display" {
                    name.strip_suffix(".nvic").is_some_and(valid_key)
                } else {
                    crate::core::library::thumbnail_key_of_file(name).is_some()
                };
                if !owned || !entry.file_type()?.is_file() {
                    continue;
                }
                let metadata = entry.metadata()?;
                entries.push(Entry {
                    path: entry.path(),
                    bytes: metadata.len(),
                    used: metadata.modified().unwrap_or(UNIX_EPOCH),
                });
            }
        }
        Ok(entries)
    }

    fn prune_locked(&self) -> io::Result<()> {
        let mut entries = self.entries()?;
        let mut bytes: u64 = entries.iter().map(|entry| entry.bytes).sum();
        entries.sort_by(|a, b| a.used.cmp(&b.used).then_with(|| a.path.cmp(&b.path)));
        for entry in entries {
            if bytes <= self.limit {
                break;
            }
            match fs::remove_file(&entry.path) {
                Ok(()) => bytes = bytes.saturating_sub(entry.bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    bytes = bytes.saturating_sub(entry.bytes)
                }
                Err(error) => {
                    self.set_usage(None);
                    return Err(error);
                }
            }
        }
        self.set_usage(Some(bytes));
        Ok(())
    }
}

struct Entry {
    path: PathBuf,
    bytes: u64,
    used: SystemTime,
}
fn valid_key(key: &str) -> bool {
    key.len() == 32
        && key
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid cached image")
}
fn stamp(path: &Path) -> Option<Stamp> {
    let metadata = fs::metadata(path).ok().filter(|meta| meta.is_file())?;
    Some(Stamp {
        size: metadata.len(),
        modified: metadata.modified().ok()?,
    })
}

fn source_identity(path: &Path) -> Option<SourceIdentity> {
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    if let Some(provider) = crate::core::documents::provider_cache_identity(path).ok()? {
        return Some(SourceIdentity::Provider(provider));
    }
    if let Some((archive, inner)) = crate::core::archive::page_origin(path) {
        return stamp(&archive).map(|stamp| SourceIdentity::Archive {
            archive,
            inner,
            stamp,
        });
    }
    stamp(path).map(SourceIdentity::File)
}
pub(crate) fn touch(path: &Path) -> io::Result<()> {
    File::options()
        .write(true)
        .open(path)?
        .set_modified(SystemTime::now())
}

fn read_image(path: &Path) -> io::Result<CachedImage> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    if length > MAX_FILE {
        return Err(invalid());
    }
    // One read: the QOI decoder issues many small reads, which a buffered
    // file serves far more slowly than a slice.
    let mut bytes = Vec::with_capacity(length as usize);
    file.read_to_end(&mut bytes)?;
    let mut input = io::Cursor::new(bytes);
    let mut header = [0; 24];
    input.read_exact(&mut header)?;
    if &header[..8] != MAGIC {
        return Err(invalid());
    }
    let number = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
    let original_size = [number(8), number(12)];
    let (width, height) = (number(16), number(20));
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || original_size.contains(&0)
        || original_size.iter().any(|&v| v > 262_144)
        || u64::from(original_size[0]) * u64::from(original_size[1]) > 1_000_000_000
    {
        return Err(invalid());
    }
    let mut decoder = image::codecs::qoi::QoiDecoder::new(input).map_err(|_| invalid())?;
    if decoder.dimensions() != (width, height)
        || decoder.total_bytes() != u64::from(width) * u64::from(height) * 4
        || decoder.color_type() != image::ColorType::Rgba8
    {
        return Err(invalid());
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 4);
    decoder.set_limits(limits).map_err(|_| invalid())?;
    let mut rgba = vec![0; width as usize * height as usize * 4];
    decoder.read_image(&mut rgba).map_err(|error| match error {
        image::ImageError::IoError(error) if error.kind() != io::ErrorKind::UnexpectedEof => error,
        _ => invalid(),
    })?;
    Ok(CachedImage {
        image: egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba),
        original_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Cache, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.png");
        fs::write(&source, b"source").unwrap();
        let cache = Cache::new(root.path().join("cache"), 1024 * 1024);
        (root, cache, source)
    }
    fn pixels() -> egui::ColorImage {
        egui::ColorImage::from_rgba_unmultiplied([2, 1], &[200, 90, 10, 128, 0, 0, 0, 0])
    }

    #[test]
    fn reads_hit_while_a_writer_or_pruner_holds_the_lock() {
        use std::{sync::mpsc, thread, time::Duration};

        let (_root, cache, source) = fixture();
        let key = cache.key(&source, 512).unwrap();
        cache.store(&key, &pixels(), [20, 10]).unwrap();
        let held = cache.lock().unwrap();
        let (send, receive) = mpsc::channel();
        let reader = thread::spawn(move || {
            send.send((
                cache.key(&source, 512).is_some(),
                cache.load(&key).map(|image| image.original_size),
            ))
            .unwrap();
        });
        // Release before joining even on failure, so a blocking regression
        // reports an assertion instead of stranding the test's reader thread.
        let result = receive.recv_timeout(Duration::from_secs(1));
        drop(held);
        reader.join().unwrap();
        assert_eq!(result, Ok((true, Some([20, 10]))));
    }

    #[test]
    fn foreground_cache_reads_preserve_hits_and_invalidation() {
        let (_root, cache, source) = fixture();
        let key = cache.key(&source, 512).unwrap();
        cache.store(&key, &pixels(), [20, 10]).unwrap();
        let loaded = cache.load(&key).unwrap();
        assert_eq!(loaded.original_size, [20, 10]);
        assert_eq!(loaded.image.pixels, pixels().pixels);

        let disabled = Cache::new(cache.root().to_owned(), 0);
        assert!(disabled.key(&source, 512).is_none());
        assert!(disabled.load(&key).is_none());

        cache.clear().unwrap();
        cache.store(&key, &pixels(), [20, 10]).unwrap();
        assert!(cache.load(&key).is_none());
        assert_eq!(cache.stats().unwrap().files, 0);

        let current = cache.key(&source, 512).unwrap();
        cache.store(&current, &pixels(), [20, 10]).unwrap();
        assert!(cache.load(&current).is_some());
        fs::write(&source, b"different source").unwrap();
        assert!(cache.load(&current).is_none());
    }

    #[test]
    fn round_trip_preserves_alpha_dimensions_and_invalidates_source_edits() {
        let (_root, cache, source) = fixture();
        let key = cache.key(&source, 512).unwrap();
        cache.store(&key, &pixels(), [20, 10]).unwrap();
        let loaded = cache.load(&key).unwrap();
        assert_eq!(loaded.original_size, [20, 10]);
        assert_eq!(loaded.image.pixels, pixels().pixels);
        assert!(cache.load(&cache.key(&source, 1024).unwrap()).is_none());
        fs::write(&source, b"different source").unwrap();
        assert!(cache.load(&key).is_none());
        assert!(cache.load(&cache.key(&source, 512).unwrap()).is_none());
    }
    #[test]
    fn clear_prevents_inflight_old_decode_from_repopulating_and_keeps_unowned_files() {
        let (_root, cache, source) = fixture();
        let key = cache.key(&source, 512).unwrap();
        cache.store(&key, &pixels(), [2, 1]).unwrap();
        fs::write(cache.root().join("display/keep.txt"), b"user").unwrap();
        cache.clear().unwrap();
        cache.store(&key, &pixels(), [2, 1]).unwrap();
        assert_eq!(cache.stats().unwrap().bytes, 0);
        assert!(cache.root().join("display/keep.txt").is_file());
        cache
            .store(&cache.key(&source, 512).unwrap(), &pixels(), [2, 1])
            .unwrap();
        assert_eq!(cache.stats().unwrap().files, 1);
    }
    #[test]
    fn thumbnail_publish_obeys_clear_generation_and_latest_limit() {
        let (_root, cache, _) = fixture();
        let key = "a".repeat(32);
        let before_clear = cache.thumbnail_generation().unwrap();
        cache.clear().unwrap();
        assert!(
            !cache
                .store_thumbnail(before_clear, &key, b"thumbnail", || 1024)
                .unwrap()
        );
        assert_eq!(cache.stats().unwrap().files, 0);

        let current = cache.thumbnail_generation().unwrap();
        assert!(
            !cache
                .store_thumbnail(current, &key, b"thumbnail", || 0)
                .unwrap()
        );
        assert!(
            !cache
                .store_thumbnail(current, &key, b"thumbnail", || 1)
                .unwrap()
        );
        assert_eq!(cache.stats().unwrap().files, 0);
        assert!(
            cache
                .store_thumbnail(current, &key, b"thumbnail", || 1024)
                .unwrap()
        );
        assert_eq!(cache.stats().unwrap().files, 1);
    }
    #[test]
    fn running_size_keeps_the_limit_without_listing_every_write() {
        let root = tempfile::tempdir().unwrap();
        let image = egui::ColorImage::filled([64, 64], egui::Color32::from_rgb(9, 99, 199));
        let probe = Cache::new(root.path().join("probe"), 1 << 30);
        let source = root.path().join("probe.png");
        fs::write(&source, b"probe").unwrap();
        probe
            .store(&probe.key(&source, 1).unwrap(), &image, [64, 64])
            .unwrap();
        let entry = probe.stats().unwrap().bytes;
        // Room for three entries: older ones are evicted as new ones arrive.
        let cache = Cache::new(root.path().join("cache"), entry * 3 + entry / 2);
        for index in 0..10 {
            let source = root.path().join(format!("{index}.png"));
            fs::write(&source, format!("source {index}")).unwrap();
            cache
                .store(&cache.key(&source, 1).unwrap(), &image, [64, 64])
                .unwrap();
            let stats = cache.stats().unwrap();
            assert!(
                stats.bytes <= cache.limit,
                "write {index}: {} bytes",
                stats.bytes
            );
        }
        assert_eq!(cache.stats().unwrap().files, 3);
        // Clear-all resets the running total as well as the files.
        cache.clear().unwrap();
        let source = root.path().join("after.png");
        fs::write(&source, b"after").unwrap();
        cache
            .store(&cache.key(&source, 1).unwrap(), &image, [64, 64])
            .unwrap();
        assert_eq!(cache.stats().unwrap().files, 1);
    }
    #[test]
    fn lru_includes_thumbnails_and_recovers_corrupt_entries() {
        let (_root, cache, source) = fixture();
        let key = cache.key(&source, 512).unwrap();
        cache.store(&key, &pixels(), [2, 1]).unwrap();
        fs::write(cache.path(&key), b"corrupt").unwrap();
        assert!(cache.load(&key).is_none());
        assert!(!cache.path(&key).exists());
        let directory = cache.root().join("thumbnails");
        fs::create_dir_all(&directory).unwrap();
        let old = directory.join(format!("{}.png", "a".repeat(32)));
        let recent = directory.join(format!("{}.jpg", "b".repeat(32)));
        fs::write(&old, vec![0; 600]).unwrap();
        fs::write(&recent, vec![0; 600]).unwrap();
        File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(UNIX_EPOCH)
            .unwrap();
        Cache::new(cache.root().to_owned(), 1000).prune().unwrap();
        assert!(!old.exists());
        assert!(recent.exists());
    }
}
