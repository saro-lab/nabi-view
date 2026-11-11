//! Private, bounded staging of documents selected through an OS picker.
//! Callers retain OS access for the duration of add(); the viewer only owns
//! these copies. Persistent source markers contain opaque host IDs, never
//! content URIs or security-scoped bookmarks; the native host owns those grants.
use crate::core::media::FileSortMetadata;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

const FILE_LIMIT: u64 = 256 * 1024 * 1024;
const BATCH_LIMIT: u64 = 512 * 1024 * 1024;
const COUNT_LIMIT: usize = 128;
// A picked folder is imported as a whole, so it may be larger than a selection.
const FOLDER_BATCH_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const FOLDER_COUNT_LIMIT: usize = 1000;
const SOURCE_MARKER: &str = ".nabi-document-source.json";
const SOURCE_LIMIT: u64 = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceRecord {
    pub id: String,
    #[serde(skip)]
    pub root: PathBuf,
    pub name: String,
    pub pages: Vec<PathBuf>,
    pub folder: bool,
}

fn active_sources() -> &'static Mutex<HashMap<PathBuf, PathBuf>> {
    static ACTIVE: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();
    ACTIVE.get_or_init(Default::default)
}

fn valid_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// A marker describes permission to reopen, not an image file or a write target.
/// Native hosts validate the saved grant on each reopen; a revoked grant stays
/// in the library so the user can select the source again.
pub(crate) fn source_record(path: &Path) -> Option<SourceRecord> {
    for root in path.ancestors() {
        if root.parent()?.file_name()? != "document-sources" {
            continue;
        }
        let id = root.file_name()?.to_str()?;
        if !valid_id(id) {
            return None;
        }
        let file = File::open(root.join(SOURCE_MARKER)).ok()?;
        if file.metadata().ok()?.len() > SOURCE_LIMIT {
            return None;
        }
        let mut record: SourceRecord = serde_json::from_reader(file.take(SOURCE_LIMIT)).ok()?;
        if record.id != id
            || record.pages.len() > FOLDER_COUNT_LIMIT
            || record
                .pages
                .iter()
                .any(|p| !safe_relative(p) || p.components().count() != 1)
        {
            return None;
        }
        record.root = root.to_owned();
        return Some(record);
    }
    None
}

pub(crate) fn source_is_container(path: &Path) -> bool {
    source_record(path).is_some_and(|record| record.root == path)
}

pub(crate) fn source_first_page(path: &Path) -> Option<PathBuf> {
    let record = source_record(path)?;
    record.pages.first().map(|page| record.root.join(page))
}

pub(crate) fn source_name(path: &Path) -> Option<String> {
    Some(source_record(path)?.name)
}

pub(crate) fn source_page_exists(path: &Path) -> bool {
    let Some(record) = source_record(path) else {
        return false;
    };
    let Ok(relative) = path.strip_prefix(&record.root) else {
        return false;
    };
    if !safe_relative(relative) {
        return false;
    }
    record.pages.iter().any(|page| {
        relative == page || (crate::core::archive::is_archive(page) && relative.starts_with(page))
    })
}

/// Resolves only a currently owned staging batch. Never falls back to an
/// original path or follows a document provider URI as a local filename.
pub(crate) fn source_local_path(path: &Path) -> Option<PathBuf> {
    let record = source_record(path)?;
    let relative = path.strip_prefix(&record.root).ok()?;
    if !safe_relative(relative) {
        return None;
    }
    let key = crate::core::paths::canonicalize(&record.root).ok()?;
    active_sources()
        .lock()
        .ok()?
        .get(&key)
        .map(|root| root.join(relative))
}

pub(crate) struct ImportBatch {
    directory: tempfile::TempDir,
    canonical_directory: PathBuf,
    files: Vec<PathBuf>,
    sort_metadata: HashMap<PathBuf, FileSortMetadata>,
    bytes: u64,
    folder: bool,
    skipped: usize,
    truncated: bool,
    source: Option<SourceRecord>,
    requested: Option<PathBuf>,
}

/// Images removed from the viewer's list are parked here, outside the listed
/// batch folder, so they can be restored. Abandoned batches start with
/// `selection-` and are cleaned on the next launch.
pub(crate) fn removed_directory(batch: &Path) -> Option<PathBuf> {
    let name = batch.file_name()?.to_string_lossy();
    if !name.starts_with("selection-") {
        return None;
    }
    Some(batch.with_file_name(format!("{name}.removed")))
}

impl ImportBatch {
    pub fn new(cache: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
        let directory = tempfile::Builder::new()
            .prefix("selection-")
            .tempdir_in(cache)
            .map_err(|e| e.to_string())?;
        let canonical_directory =
            crate::core::paths::canonicalize(directory.path()).map_err(|e| e.to_string())?;
        Ok(Self {
            directory,
            canonical_directory,
            files: Vec::new(),
            sort_metadata: HashMap::new(),
            bytes: 0,
            folder: false,
            skipped: 0,
            truncated: false,
            source: None,
            requested: None,
        })
    }

    /// A whole picked folder: other files are skipped instead of failing the batch.
    pub fn new_folder(cache: &Path) -> Result<Self, String> {
        let mut batch = Self::new(cache)?;
        batch.folder = true;
        Ok(batch)
    }

    pub fn is_folder(&self) -> bool {
        self.folder
    }

    pub fn first(&self) -> Option<&Path> {
        self.files.first().map(PathBuf::as_path)
    }

    pub fn directory(&self) -> &Path {
        &self.canonical_directory
    }
    /// Provider metadata is kept only for the owned copies. In particular, a
    /// missing provider time must replace the staging file's creation-time mtime.
    /// Archive members retain the archive index's own ordering metadata.
    pub fn apply_sort_metadata(&self, stamps: &mut HashMap<PathBuf, FileSortMetadata>) {
        for (path, stamp) in stamps {
            if let Some(original) = self.sort_metadata.get(path) {
                *stamp = *original;
            }
        }
    }
    pub fn source(&self) -> Option<&SourceRecord> {
        self.source.as_ref()
    }
    pub fn logical_path(&self, local: &Path) -> Option<PathBuf> {
        let relative = local.strip_prefix(&self.canonical_directory).ok()?;
        safe_relative(relative).then(|| self.source.as_ref().map(|s| s.root.join(relative)))?
    }
    pub fn local_path(&self, logical: &Path) -> Option<PathBuf> {
        let relative = logical.strip_prefix(&self.source.as_ref()?.root).ok()?;
        safe_relative(relative).then(|| self.canonical_directory.join(relative))
    }
    pub fn initial_path(&self) -> Option<PathBuf> {
        self.requested
            .as_deref()
            .and_then(|p| self.local_path(p))
            .filter(|p| p.is_file() || crate::core::archive::split(p).is_some())
            .or_else(|| self.first().map(Path::to_owned))
    }
    pub fn request_page(&mut self, path: Option<PathBuf>) {
        self.requested = path;
    }

    pub fn set_source(&mut self, sources: &Path, id: &str, name: &str) -> Result<(), String> {
        if !valid_id(id) || !sources.is_absolute() {
            return Err("Invalid saved document source.".into());
        }
        std::fs::create_dir_all(sources).map_err(|e| e.to_string())?;
        let sources = crate::core::paths::canonicalize(sources).map_err(|e| e.to_string())?;
        let root = sources.join(id);
        if std::fs::symlink_metadata(&root).is_ok_and(|m| !m.is_dir() || m.file_type().is_symlink())
        {
            return Err("Invalid saved document directory.".into());
        }
        self.source = Some(SourceRecord {
            id: id.into(),
            root,
            name: name.chars().filter(|c| !c.is_control()).take(200).collect(),
            pages: Vec::new(),
            folder: self.folder,
        });
        Ok(())
    }

    /// Atomically publishes metadata after a successful bounded import. Source
    /// pixels remain in the cache-owned TempDir and disappear with the batch.
    pub fn commit_source(&mut self) -> Result<(), String> {
        let Some(source) = &mut self.source else {
            return Ok(());
        };
        source.pages = self
            .files
            .iter()
            .filter_map(|p| p.file_name().map(PathBuf::from))
            .collect();
        if source.folder {
            source
                .pages
                .sort_by(|a, b| crate::core::media::compare_file_names(a, b));
        }
        std::fs::create_dir_all(&source.root).map_err(|e| e.to_string())?;
        let mut temp = tempfile::NamedTempFile::new_in(&source.root).map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut temp, source).map_err(|e| e.to_string())?;
        temp.as_file_mut().sync_all().map_err(|e| e.to_string())?;
        temp.persist(source.root.join(SOURCE_MARKER))
            .map_err(|e| e.to_string())?;
        active_sources()
            .lock()
            .unwrap()
            .insert(source.root.clone(), self.canonical_directory.clone());
        Ok(())
    }

    /// Message for images left out of a folder import, if any.
    pub fn notice(&self) -> Option<String> {
        if self.truncated {
            Some(format!(
                "Only the first {} images in the folder were imported.",
                self.files.len()
            ))
        } else if self.skipped > 0 {
            Some(format!(
                "{} images in the folder could not be imported.",
                self.skipped
            ))
        } else {
            None
        }
    }

    /// Copies one document. Returns `Ok(false)` once a folder import is full
    /// and the caller should stop offering documents.
    #[cfg(test)]
    pub fn add(&mut self, name: &str, source: impl Read) -> Result<bool, String> {
        self.add_with_modified(name, source, None)
    }

    pub fn add_with_modified(
        &mut self,
        name: &str,
        source: impl Read,
        modified: Option<SystemTime>,
    ) -> Result<bool, String> {
        if !self.folder {
            return self
                .add_limited(name, source, FILE_LIMIT, BATCH_LIMIT, modified)
                .map(|()| true);
        }
        if !crate::core::media::is_supported(Path::new(name)) {
            return Ok(true);
        }
        if self.files.len() >= FOLDER_COUNT_LIMIT || self.bytes >= FOLDER_BATCH_LIMIT {
            self.truncated = true;
            return Ok(false);
        }
        if self
            .add_limited(name, source, FILE_LIMIT, FOLDER_BATCH_LIMIT, modified)
            .is_err()
        {
            self.skipped += 1;
        }
        Ok(true)
    }

    fn add_limited(
        &mut self,
        name: &str,
        source: impl Read,
        file_limit: u64,
        batch_limit: u64,
        modified: Option<SystemTime>,
    ) -> Result<(), String> {
        if !self.folder && self.files.len() >= COUNT_LIMIT {
            return Err("Select at most 128 images at a time.".into());
        }
        // A provider's display name is untrusted metadata, not a filesystem path.
        let name: String = name
            .chars()
            .map(|c| {
                if c == '/' || c == '\\' || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        if name.is_empty() || name == "." || name == ".." || name.len() > 200 {
            return Err("Invalid document name.".into());
        }
        if !crate::core::media::is_supported(Path::new(&name))
            && !crate::core::archive::is_archive(Path::new(&name))
        {
            return Err(format!("Unsupported image or archive: {name}"));
        }
        let mut destination = self.canonical_directory.join(&name);
        let mut suffix = 1;
        while destination.exists() {
            let stem = Path::new(&name)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy();
            let extension = Path::new(&name)
                .extension()
                .unwrap_or_default()
                .to_string_lossy();
            destination = self
                .canonical_directory
                .join(format!("{stem} ({suffix}).{extension}"));
            suffix += 1;
        }
        let remaining = file_limit.min(batch_limit.saturating_sub(self.bytes));
        let result = (|| {
            let mut output = File::options()
                .write(true)
                .create_new(true)
                .open(&destination)
                .map_err(|e| e.to_string())?;
            let mut input = source.take(remaining.saturating_add(1));
            let size = io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
            if size > remaining {
                return Err("Selected images exceed the import size limit.".into());
            }
            if size == 0 {
                return Err("The selected file is empty.".into());
            }
            output.flush().map_err(|e| e.to_string())?;
            Ok(size)
        })();
        match result {
            Ok(size) => {
                self.bytes += size;
                // Some providers use epoch zero (or an unavailable pre-epoch
                // value) for an unknown timestamp. Never substitute copy time.
                let modified = modified
                    .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .filter(|elapsed| !elapsed.is_zero())
                    .map(|elapsed| (1u128 << 127) + elapsed.as_nanos());
                self.sort_metadata.insert(
                    destination.clone(),
                    FileSortMetadata {
                        bytes: size,
                        modified,
                    },
                );
                self.files.push(destination);
                Ok(())
            }
            Err(error) => {
                let _ = std::fs::remove_file(destination);
                Err(error)
            }
        }
    }
}

impl Drop for ImportBatch {
    fn drop(&mut self) {
        if let Some(source) = &self.source {
            let mut active = active_sources().lock().unwrap();
            if active
                .get(&source.root)
                .is_some_and(|path| path == &self.canonical_directory)
            {
                active.remove(&source.root);
            }
        }
        if let Some(removed) = removed_directory(self.directory.path()) {
            let _ = std::fs::remove_dir_all(removed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn import_metadata_preserves_provider_times_and_duplicate_sizes() {
        use std::time::Duration;
        let cache = tempfile::tempdir().unwrap();
        let mut batch = ImportBatch::new(cache.path()).unwrap();
        let epoch = SystemTime::UNIX_EPOCH;
        for (modified, bytes) in [
            (Some(epoch + Duration::from_secs(42)), 3),
            (Some(epoch), 4),
            (Some(epoch - Duration::from_secs(1)), 5),
            (None, 6),
        ] {
            batch
                .add_with_modified("same.png", vec![1; bytes].as_slice(), modified)
                .unwrap();
        }
        let fake_copy_stamp = FileSortMetadata {
            bytes: 999,
            modified: Some(u128::MAX),
        };
        let mut stamps: HashMap<_, _> = batch
            .files
            .iter()
            .cloned()
            .map(|path| (path, fake_copy_stamp))
            .collect();
        let unrelated = cache.path().join("unrelated.png");
        stamps.insert(unrelated.clone(), fake_copy_stamp);
        batch.apply_sort_metadata(&mut stamps);
        for (index, path) in batch.files.iter().enumerate() {
            assert_eq!(stamps[path].bytes, index as u64 + 3);
            assert_eq!(
                stamps[path].modified,
                (index == 0).then_some((1u128 << 127) + 42_000_000_000)
            );
        }
        assert_eq!(stamps[&unrelated], fake_copy_stamp);
        assert_eq!(batch.sort_metadata.len(), 4);
        assert!(
            batch
                .add_with_modified("empty.png", io::empty(), Some(epoch))
                .is_err()
        );
        assert_eq!(batch.sort_metadata.len(), 4);
    }

    #[test]
    fn confines_names_preserves_duplicates_and_cleans_up() {
        let cache = tempfile::tempdir().unwrap();
        let mut batch = ImportBatch::new(cache.path()).unwrap();
        batch.add("../image.png", &b"first"[..]).unwrap();
        batch.add("../image.png", &b"second"[..]).unwrap();
        assert_eq!(batch.files.len(), 2);
        assert_ne!(batch.files[0], batch.files[1]);
        assert!(
            batch
                .files
                .iter()
                .all(|p| p.parent() == Some(batch.directory()))
        );
        assert_eq!(std::fs::read(batch.first().unwrap()).unwrap(), b"first");
        let path = batch.first().unwrap().to_owned();
        drop(batch);
        assert!(!path.exists());
    }
    #[test]
    fn caps_individual_and_combined_streams_and_removes_partial_files() {
        let cache = tempfile::tempdir().unwrap();
        let mut batch = ImportBatch::new(cache.path()).unwrap();
        assert!(
            batch
                .add_limited("large.png", io::repeat(0), 8, 12, None)
                .is_err()
        );
        assert_eq!(
            std::fs::read_dir(batch.directory.path()).unwrap().count(),
            0
        );
        batch
            .add_limited("a.png", &b"12345678"[..], 8, 12, None)
            .unwrap();
        assert!(
            batch
                .add_limited("b.png", &b"12345"[..], 8, 12, None)
                .is_err()
        );
        batch
            .add_limited("b.png", &b"1234"[..], 8, 12, None)
            .unwrap();
        assert_eq!(batch.bytes, 12);
        assert_eq!(batch.files.len(), 2);
    }
    #[test]
    fn rejects_empty_and_unsupported_documents() {
        let cache = tempfile::tempdir().unwrap();
        let mut batch = ImportBatch::new(cache.path()).unwrap();
        assert!(batch.add("a.png", io::empty()).is_err());
        assert!(batch.add("a.sh", &b"text"[..]).is_err());
        assert!(batch.first().is_none());
    }
    #[test]
    fn folder_imports_skip_other_files_and_report_failures() {
        let cache = tempfile::tempdir().unwrap();
        let mut batch = ImportBatch::new_folder(cache.path()).unwrap();
        assert!(batch.is_folder());
        assert!(batch.add("notes.txt", &b"text"[..]).unwrap());
        assert!(batch.add("a.png", &b"image"[..]).unwrap());
        assert!(batch.add("empty.png", io::empty()).unwrap());
        assert_eq!(batch.files.len(), 1);
        assert_eq!(batch.first().unwrap().file_name().unwrap(), "a.png");
        assert!(batch.notice().unwrap().contains('1'));
        for index in 0..FOLDER_COUNT_LIMIT + 5 {
            if !batch.add(&format!("{index}.png"), &b"x"[..]).unwrap() {
                break;
            }
        }
        assert_eq!(batch.files.len(), FOLDER_COUNT_LIMIT);
        assert!(batch.notice().unwrap().contains("1000"));
    }
    #[test]
    fn dropping_a_batch_removes_its_parked_copies() {
        let cache = tempfile::tempdir().unwrap();
        let batch = ImportBatch::new(cache.path()).unwrap();
        let removed = removed_directory(batch.directory.path()).unwrap();
        assert!(
            removed
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("selection-")
        );
        std::fs::create_dir_all(removed.join("item")).unwrap();
        drop(batch);
        assert!(!removed.exists());
    }

    #[test]
    fn saved_sources_keep_logical_pages_without_retaining_pixels() {
        let root = tempfile::tempdir().unwrap();
        let sources = root.path().join("document-sources");
        let mut batch = ImportBatch::new(root.path()).unwrap();
        batch
            .add("book.zip", &b"bounded archive input"[..])
            .unwrap();
        batch
            .set_source(&sources, "12345678-source", "Picked book")
            .unwrap();
        batch.commit_source().unwrap();
        let copy = batch.first().unwrap().to_owned();
        let logical = batch.logical_path(&copy).unwrap();
        assert!(source_page_exists(&logical));
        assert!(source_page_exists(&logical.join("chapter/01.png")));
        assert!(!source_page_exists(&logical.join("../outside.png")));
        assert_eq!(source_local_path(&logical), Some(copy.clone()));
        assert_eq!(source_name(&logical).as_deref(), Some("Picked book"));
        let container = logical.parent().unwrap().to_owned();
        assert!(source_is_container(&container));
        assert_eq!(source_first_page(&container), Some(logical.clone()));
        assert!(!logical.is_file());
        drop(batch);
        assert!(!copy.exists());
        assert!(source_page_exists(&logical));
        assert!(source_local_path(&logical).is_none());
        let mut next = ImportBatch::new(root.path()).unwrap();
        next.add("book.zip", &b"updated source"[..]).unwrap();
        next.set_source(&sources, "12345678-source", "Picked book")
            .unwrap();
        next.request_page(Some(logical.clone()));
        next.commit_source().unwrap();
        assert_eq!(next.initial_path().as_deref(), next.first());
        assert_eq!(
            std::fs::read(source_local_path(&logical).unwrap()).unwrap(),
            b"updated source"
        );
    }

    #[test]
    fn source_markers_reject_traversal_and_partial_imports() {
        let root = tempfile::tempdir().unwrap();
        let sources = root.path().join("document-sources");
        let mut batch = ImportBatch::new(root.path()).unwrap();
        assert!(batch.set_source(&sources, "../../outside", "Bad").is_err());
        batch
            .set_source(&sources, "12345678-source", "Good")
            .unwrap();
        assert!(source_record(&sources.join("12345678-source")).is_none());
        batch.add("01.png", &b"first"[..]).unwrap();
        batch.commit_source().unwrap();
        let marker = sources.join("12345678-source").join(SOURCE_MARKER);
        let mut record: SourceRecord =
            serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
        record.pages = vec![PathBuf::from("../outside.png")];
        std::fs::write(&marker, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(source_record(marker.parent().unwrap()).is_none());
    }

    #[test]
    fn failed_reopen_keeps_the_source_and_replacement_outlives_the_previous_batch() {
        let root = tempfile::tempdir().unwrap();
        let sources = root.path().join("document-sources");
        let mut first = ImportBatch::new(root.path()).unwrap();
        first.add("01.png", &b"first"[..]).unwrap();
        first
            .set_source(&sources, "12345678-reopen", "Photos")
            .unwrap();
        first.commit_source().unwrap();
        let logical = first.logical_path(first.first().unwrap()).unwrap();
        let marker = first.source().unwrap().root.join(SOURCE_MARKER);
        let metadata = std::fs::read(&marker).unwrap();

        // A revoked/unreadable provider fails before commit_source. The saved
        // page and the currently displayed copy must remain usable.
        let mut failed = ImportBatch::new(root.path()).unwrap();
        failed
            .set_source(&sources, "12345678-reopen", "Photos")
            .unwrap();
        assert!(failed.add("01.png", io::empty()).is_err());
        drop(failed);
        assert_eq!(std::fs::read(&marker).unwrap(), metadata);
        assert_eq!(source_local_path(&logical).as_deref(), first.first());

        let mut replacement = ImportBatch::new(root.path()).unwrap();
        replacement.add("01.png", &b"replacement"[..]).unwrap();
        replacement
            .set_source(&sources, "12345678-reopen", "Photos")
            .unwrap();
        replacement.commit_source().unwrap();
        drop(first);
        let copy = source_local_path(&logical).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"replacement");

        // Clearing disposable display/thumbnail files cannot revoke a grant
        // or delete an open import. A restart has no active copy but retains
        // the same logical page for the native host to reopen.
        crate::core::disk_cache::Cache::new(root.path().join("cache/v1"), 1024)
            .clear()
            .unwrap();
        assert!(marker.is_file());
        assert!(copy.is_file());
        drop(replacement);
        assert!(!copy.exists());
        assert!(source_page_exists(&logical));
        assert!(source_local_path(&logical).is_none());
    }
}
