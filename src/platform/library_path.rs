//! Filesystem checks for a stored library container (directory or archive).
use std::{fs, io::ErrorKind, path::Path};

/// Only a positive observation of a missing container may discard a library row.
/// Callers handle provider-backed identities separately from filesystem paths.
pub(crate) fn is_missing(path: &Path) -> bool {
    #[cfg(all(target_os = "macos", not(test)))]
    let Ok(access) = crate::platform::mac_access::resolve_for_library_check(path) else {
        return false;
    };
    #[cfg(all(target_os = "macos", not(test)))]
    let path = access.path.as_path();
    filesystem_container_missing(path)
}

fn filesystem_container_missing(path: &Path) -> bool {
    let missing = match fs::metadata(path) {
        Ok(metadata) => metadata.is_file() && !crate::core::archive::is_archive(path),
        Err(e) => matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory),
    };
    missing && volume_online(path)
}

#[cfg(windows)]
fn volume_online(path: &Path) -> bool {
    // A detached drive or UNC share may also return ERROR_PATH_NOT_FOUND.
    path.is_absolute()
        && path
            .ancestors()
            .last()
            .is_some_and(|root| root != path && fs::read_dir(root).is_ok())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn volume_online(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    #[cfg(target_os = "macos")]
    let mount_directories = ["/Volumes"];
    #[cfg(target_os = "linux")]
    let mount_directories = ["/media", "/run/media", "/mnt"];

    let Some(base) = mount_directories
        .iter()
        .map(Path::new)
        .find(|base| path.starts_with(base))
    else {
        return true;
    };
    // After an unmount the mount directory can disappear or survive as an
    // empty directory on the host filesystem. Neither proves a file was deleted.
    let Ok(base_metadata) = fs::metadata(base) else {
        return false;
    };
    path.ancestors()
        .take_while(|ancestor| *ancestor != base)
        .find_map(|ancestor| fs::metadata(ancestor).ok())
        .is_some_and(|metadata| metadata.dev() != base_metadata.dev())
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn volume_online(_: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_absent_folders_and_archives_are_missing() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let container = root.path().join("album");
        let archive = root.path().join("book.ZIP");
        fs::create_dir(&container).unwrap();
        fs::write(&archive, []).unwrap();
        assert!(!filesystem_container_missing(&container));
        assert!(!filesystem_container_missing(&archive));
        fs::remove_dir(&container).unwrap();
        fs::remove_file(&archive).unwrap();
        assert!(filesystem_container_missing(&container));
        assert!(filesystem_container_missing(&archive));
        fs::write(&container, []).unwrap();
        assert!(filesystem_container_missing(&container));
        assert!(filesystem_container_missing(&container.join("subfolder")));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn absent_external_volume_is_unavailable() {
        #[cfg(target_os = "macos")]
        let base = Path::new("/Volumes");
        #[cfg(target_os = "linux")]
        let base = Path::new("/run/media");
        let volume = base.join("nabi-test-absent-disk-38cc45fd");
        assert!(!filesystem_container_missing(&volume.join("photos")));
    }
}
