//! Private, bounded staging of documents selected through an OS picker.
//! Callers retain OS access for the duration of add(); the viewer only owns
//! these copies. No source path, content URI or permission is persisted.
use std::{
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const FILE_LIMIT: u64 = 256 * 1024 * 1024;
const BATCH_LIMIT: u64 = 512 * 1024 * 1024;
const COUNT_LIMIT: usize = 128;

pub(crate) struct ImportBatch {
    directory: tempfile::TempDir,
    files: Vec<PathBuf>,
    bytes: u64,
}

impl ImportBatch {
    pub fn new(cache: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
        let directory = tempfile::Builder::new()
            .prefix("selection-")
            .tempdir_in(cache)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            directory,
            files: Vec::new(),
            bytes: 0,
        })
    }

    pub fn first(&self) -> Option<&Path> {
        self.files.first().map(PathBuf::as_path)
    }

    pub fn add(&mut self, name: &str, source: impl Read) -> Result<(), String> {
        self.add_limited(name, source, FILE_LIMIT, BATCH_LIMIT)
    }

    fn add_limited(
        &mut self,
        name: &str,
        source: impl Read,
        file_limit: u64,
        batch_limit: u64,
    ) -> Result<(), String> {
        if self.files.len() >= COUNT_LIMIT {
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
        if !crate::core::media::is_supported(Path::new(&name)) {
            return Err(format!("Unsupported image: {name}"));
        }
        let mut destination = self.directory.path().join(&name);
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
                .directory
                .path()
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

#[cfg(test)]
mod tests {
    use super::*;
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
                .all(|p| p.parent() == Some(batch.directory.path()))
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
                .add_limited("large.png", io::repeat(0), 8, 12)
                .is_err()
        );
        assert_eq!(
            std::fs::read_dir(batch.directory.path()).unwrap().count(),
            0
        );
        batch.add_limited("a.png", &b"12345678"[..], 8, 12).unwrap();
        assert!(batch.add_limited("b.png", &b"12345"[..], 8, 12).is_err());
        batch.add_limited("b.png", &b"1234"[..], 8, 12).unwrap();
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
}
