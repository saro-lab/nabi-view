//! Bounded, disposable image cache. All filesystem work belongs on a worker.
//! Cache files never contain source paths and never replace user documents.
use eframe::egui;
use image::{ImageDecoder, ImageEncoder};
use std::{
    fs::{self, File},
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAGIC: &[u8; 8] = b"NABIC001";
const MAX_PIXELS: u64 = 6 * 1024 * 1024;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const KEY_VERSION: &[u8] = b"display-rgba-v2-color-managed";

#[derive(Clone)]
pub(crate) struct Cache {
    root: PathBuf,
    limit: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: SystemTime,
}

/// Source identity captured before decoding, invalidated by edits and clear-all.
pub(crate) struct Key {
    name: String,
    source: PathBuf,
    stamp: Stamp,
    generation: u64,
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

    pub fn key(&self, source: &Path, max_side: u32) -> Option<Key> {
        if self.limit == 0 {
            return None;
        }
        let source = crate::core::paths::canonicalize(source).ok()?;
        let stamp = stamp(&source)?;
        let mut bytes = source.as_os_str().as_encoded_bytes().to_vec();
        bytes.extend(stamp.size.to_le_bytes());
        bytes.extend(
            stamp
                .modified
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_nanos()
                .to_le_bytes(),
        );
        bytes.extend(max_side.to_le_bytes());
        bytes.extend(KEY_VERSION);
        let hash = |basis: u64| {
            bytes.iter().fold(basis, |hash, &byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        };
        let _lock = self.lock().ok()?;
        Some(Key {
            name: format!(
                "{:016x}{:016x}",
                hash(0xcbf2_9ce4_8422_2325),
                hash(0x6c62_272e_07bb_0142)
            ),
            source,
            stamp,
            generation: self.generation(),
        })
    }

    /// Corrupt, stale and unsupported entries are ordinary misses.
    pub fn load(&self, key: &Key) -> Option<CachedImage> {
        let _lock = self.lock().ok()?;
        if key.generation != self.generation() || stamp(&key.source).as_ref() != Some(&key.stamp) {
            return None;
        }
        let path = self.path(key);
        let image = read_image(&path);
        match image {
            Ok(image) => {
                let _ = touch(&path);
                Some(image)
            }
            Err(_) => {
                let _ = fs::remove_file(path);
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
        image::codecs::png::PngEncoder::new(&mut encoded)
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
        if key.generation != self.generation() || stamp(&key.source).as_ref() != Some(&key.stamp) {
            return Ok(());
        }
        let directory = self.root.join("display");
        fs::create_dir_all(&directory)?;
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
        temporary
            .persist(self.path(key))
            .map_err(|error| error.error)?;
        self.prune_locked()
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
        for entry in self.entries()? {
            fs::remove_file(entry.path)?;
        }
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

    fn lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.root)?;
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("cache.lock"))?;
        file.lock()?;
        Ok(file)
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
                Err(error) => return Err(error),
            }
        }
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
pub(crate) fn touch(path: &Path) -> io::Result<()> {
    File::options()
        .write(true)
        .open(path)?
        .set_modified(SystemTime::now())
}

fn read_image(path: &Path) -> io::Result<CachedImage> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_FILE {
        return Err(invalid());
    }
    let mut input = BufReader::new(file);
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
    let mut decoder = image::codecs::png::PngDecoder::new(input).map_err(io::Error::other)?;
    if decoder.dimensions() != (width, height)
        || decoder.total_bytes() != u64::from(width) * u64::from(height) * 4
        || decoder.color_type() != image::ColorType::Rgba8
    {
        return Err(invalid());
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 4);
    decoder.set_limits(limits).map_err(io::Error::other)?;
    let mut rgba = vec![0; width as usize * height as usize * 4];
    decoder.read_image(&mut rgba).map_err(io::Error::other)?;
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
