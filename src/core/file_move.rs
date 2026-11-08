//! No-clobber moves of regular files. Run on a worker, including volume copies.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(crate) struct MoveResult {
    pub moved: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub destination: PathBuf,
}

pub(crate) fn move_files(paths: Vec<PathBuf>, destination: &Path) -> MoveResult {
    let destination = fs::canonicalize(destination).and_then(|path| {
        if path.is_dir() {
            Ok(path)
        } else {
            Err(io::Error::other("Choose an existing destination folder."))
        }
    });
    let mut result = MoveResult::default();
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        if !seen.insert(path.clone()) {
            continue;
        }
        let operation = match &destination {
            Ok(folder) => {
                result.destination.clone_from(folder);
                move_one(&path, folder).map_err(|e| e.to_string())
            }
            Err(error) => Err(error.to_string()),
        };
        match operation {
            Ok(()) => result.moved.push(path),
            Err(error) => result.failed.push((path, error)),
        }
    }
    result
}

fn same_file(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return false;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if before.creation_time() != after.creation_time() {
            return false;
        }
    }
    after.is_file()
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
}

fn move_one(source: &Path, folder: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_file() {
        return Err(io::Error::other("Only regular image files can be moved."));
    }
    let target = folder.join(
        source
            .file_name()
            .ok_or_else(|| io::Error::other("Invalid image path."))?,
    );
    if fs::symlink_metadata(&target).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "A file already exists at the destination; it was not overwritten.",
        ));
    }
    // Hard linking is atomic, refuses any existing name, and preserves metadata.
    // Volumes/filesystems without hard links use a fully staged copy instead.
    match fs::hard_link(source, &target) {
        Ok(()) => {
            if !same_file(&metadata, &fs::symlink_metadata(&target)?) {
                return Err(io::Error::other(
                    "The source changed while moving; the source was retained.",
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(error),
        Err(_) => copy_noclobber(source, &target, &metadata)?,
    }
    if !same_file(&metadata, &fs::symlink_metadata(source)?) {
        return Err(io::Error::other(
            "The source changed while moving; both files were retained.",
        ));
    }
    fs::remove_file(source).map_err(|error| {
        io::Error::other(format!(
            "The destination was saved, but the source could not be removed: {error}"
        ))
    })
}

fn copy_noclobber(source: &Path, target: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    let mut input = fs::File::open(source)?;
    if !same_file(metadata, &input.metadata()?) {
        return Err(io::Error::other("The source changed before copying."));
    }
    let mut staged = tempfile::NamedTempFile::new_in(target.parent().unwrap())?;
    let copied = io::copy(&mut input, &mut staged)?;
    if copied != metadata.len() || !same_file(metadata, &input.metadata()?) {
        return Err(io::Error::other(
            "The source changed while copying; it was not moved.",
        ));
    }
    staged.as_file().set_permissions(metadata.permissions())?;
    if let Ok(modified) = metadata.modified() {
        staged
            .as_file()
            .set_times(fs::FileTimes::new().set_modified(modified))?;
    }
    staged.as_file().sync_all()?;
    staged
        .persist_noclobber(target)
        .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_regular_files_without_overwriting_and_reports_partial_failure() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("destination");
        fs::create_dir(&dest).unwrap();
        let first = root.path().join("first.png");
        let second = root.path().join("second.png");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        fs::write(dest.join("second.png"), b"existing").unwrap();
        let result = move_files(vec![first.clone(), second.clone(), first.clone()], &dest);
        assert_eq!(result.moved, [first.clone()]);
        assert_eq!(result.failed.len(), 1);
        assert!(!first.exists());
        assert_eq!(fs::read(dest.join("first.png")).unwrap(), b"first");
        assert_eq!(fs::read(&second).unwrap(), b"second");
        assert_eq!(fs::read(dest.join("second.png")).unwrap(), b"existing");
        assert_eq!(
            move_files(vec![second.clone()], root.path()).failed.len(),
            1
        );
        assert!(second.exists());
        assert_eq!(move_files(vec![dest.clone()], root.path()).failed.len(), 1);
        assert_eq!(
            move_files(vec![second.clone()], &root.path().join("missing"))
                .failed
                .len(),
            1
        );
        assert!(second.exists());
    }

    #[test]
    fn staged_copy_preserves_bytes_and_timestamp_and_never_clobbers() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        fs::write(&source, b"image data").unwrap();
        let metadata = fs::metadata(&source).unwrap();
        copy_noclobber(&source, &dest, &metadata).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"image data");
        assert_eq!(
            fs::metadata(&dest).unwrap().modified().unwrap(),
            metadata.modified().unwrap()
        );
        fs::write(&dest, b"existing").unwrap();
        assert!(copy_noclobber(&source, &dest, &metadata).is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"existing");
        assert!(source.exists());
    }

    #[cfg(unix)]
    #[test]
    fn source_and_destination_symlinks_are_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let source = root.path().join("image.png");
        fs::write(&source, b"image").unwrap();
        let link = root.path().join("link.png");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        assert_eq!(move_files(vec![link], &dest).failed.len(), 1);
        std::os::unix::fs::symlink(root.path().join("missing"), dest.join("image.png")).unwrap();
        assert_eq!(move_files(vec![source.clone()], &dest).failed.len(), 1);
        assert!(source.exists());
    }
}
