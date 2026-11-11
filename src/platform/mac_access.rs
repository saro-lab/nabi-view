//! Local app-scoped read grants. Bookmarks never enter portable settings/cache.
#![cfg_attr(test, allow(dead_code))]
use std::{
    ffi::{CStr, CString, OsStr, c_char, c_void},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

unsafe extern "C" {
    fn nabi_access_configure(directory: *const c_char, error: *mut *mut c_char) -> i32;
    fn nabi_access_remember(path: *const c_char, error: *mut *mut c_char) -> i32;
    fn nabi_access_select(
        initial: *const c_char,
        extensions: *const c_char,
        folder: bool,
        paths: *mut *mut *mut c_char,
        leases: *mut *mut *mut c_void,
        count: *mut usize,
        error: *mut *mut c_char,
    ) -> i32;
    fn nabi_access_resolve(
        path: *const c_char,
        result: *mut *mut c_char,
        handle: *mut *mut c_void,
        error: *mut *mut c_char,
    ) -> i32;
    fn nabi_access_pick(
        path: *const c_char,
        folder: bool,
        result: *mut *mut c_char,
        error: *mut *mut c_char,
    ) -> i32;
    fn nabi_access_has(path: *const c_char) -> i32;
    fn nabi_access_temporary(path: *const c_char) -> bool;
    fn nabi_access_write(
        path: *const c_char,
        handle: *mut *mut c_void,
        error: *mut *mut c_char,
    ) -> i32;
    fn nabi_access_forget(
        path: *const c_char,
        retained: *const *const c_char,
        count: usize,
        error: *mut *mut c_char,
    ) -> i32;
    fn nabi_access_is_sandboxed() -> bool;
    fn nabi_access_release(handle: *mut c_void);
    fn nabi_access_free(value: *mut c_void);
}
static CONFIGURED: AtomicBool = AtomicBool::new(false);
static DATABASE: Mutex<Option<PathBuf>> = Mutex::new(None);
static REFERENCES: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

// Called only while enforcing the native registry size limit. Do not call any
// Store/library helper that can re-enter native bookmark lookup here.
#[unsafe(no_mangle)]
extern "C" fn nabi_access_references_refresh() {
    let snapshot = DATABASE.lock().ok().and_then(|database| {
        database
            .as_ref()
            .and_then(|database| crate::storage::mac_grant_references(database))
    });
    if let Ok(mut references) = REFERENCES.lock() {
        *references = snapshot;
    }
}
#[unsafe(no_mangle)]
extern "C" fn nabi_access_referenced(original: *const c_char, current: *const c_char) -> i32 {
    if original.is_null() || current.is_null() {
        return 1;
    }
    let Ok(references) = REFERENCES.lock() else {
        return 1;
    };
    let Some(references) = references.as_ref() else {
        return 1;
    };
    let paths = [original, current]
        .map(|p| PathBuf::from(OsStr::from_bytes(unsafe { CStr::from_ptr(p) }.to_bytes())));
    i32::from(references.iter().any(|path| {
        paths
            .iter()
            .any(|root| path.starts_with(root) || root.starts_with(path))
    }))
}
pub(crate) struct Access {
    pub path: PathBuf,
    handle: *mut c_void,
}
// NSURL leases are immutable after acquisition. Foundation supports ending a
// security scope from worker threads; each retained object is released once.
unsafe impl Send for Access {}
impl Drop for Access {
    fn drop(&mut self) {
        unsafe { nabi_access_release(self.handle) };
    }
}
fn cpath(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| "Invalid document path.".into())
}
fn error(value: *mut c_char) -> String {
    if value.is_null() {
        return "Document access is unavailable. Choose the file or folder again.".into();
    }
    let message = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    unsafe { nabi_access_free(value.cast()) };
    message
}
fn take_path(value: *mut c_char) -> Option<PathBuf> {
    if value.is_null() {
        return None;
    }
    let path = PathBuf::from(OsStr::from_bytes(
        unsafe { CStr::from_ptr(value) }.to_bytes(),
    ));
    unsafe { nabi_access_free(value.cast()) };
    Some(path)
}
pub(crate) fn configure(state: &Path) -> Result<(), String> {
    *DATABASE
        .lock()
        .map_err(|_| "Document permission storage is unavailable.")? =
        Some(state.join("state.sqlite"));
    let path = cpath(state)?;
    let mut message = std::ptr::null_mut();
    if unsafe { nabi_access_configure(path.as_ptr(), &mut message) } != 0 {
        return Err(error(message));
    }
    CONFIGURED.store(true, Ordering::Release);
    Ok(())
}
pub(crate) fn remember(path: &Path) -> Result<(), String> {
    if !CONFIGURED.load(Ordering::Acquire) {
        return Ok(());
    }
    let path = cpath(path)?;
    let mut message = std::ptr::null_mut();
    if unsafe { nabi_access_remember(path.as_ptr(), &mut message) } == 0 {
        Ok(())
    } else {
        Err(error(message))
    }
}
pub(crate) fn has_grant(path: &Path) -> bool {
    CONFIGURED.load(Ordering::Acquire)
        && cpath(path).is_ok_and(|path| unsafe { nabi_access_has(path.as_ptr()) != 0 })
}
pub(crate) fn temporary(path: &Path) -> bool {
    cpath(path).is_ok_and(|path| unsafe { nabi_access_temporary(path.as_ptr()) })
}
pub(crate) struct Selection {
    pub access: Vec<Access>,
    pub warning: Option<String>,
}
pub(crate) fn select(
    folder: bool,
    initial: Option<&Path>,
    extensions: &[&str],
) -> Result<Option<Selection>, String> {
    let initial = initial.map(cpath).transpose()?;
    let extensions = CString::new(extensions.join(",")).map_err(|_| "Invalid file type.")?;
    let (mut paths, mut leases, mut count, mut message) = (
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        0,
        std::ptr::null_mut(),
    );
    let code = unsafe {
        nabi_access_select(
            initial.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
            extensions.as_ptr(),
            folder,
            &mut paths,
            &mut leases,
            &mut count,
            &mut message,
        )
    };
    if code == 2 {
        return Ok(None);
    }
    if code != 0 {
        return Err(error(message));
    }
    let mut result = Vec::with_capacity(count);
    if !paths.is_null() {
        for index in 0..count {
            let handle = unsafe { *leases.add(index) };
            if let Some(path) = take_path(unsafe { *paths.add(index) }) {
                result.push(Access { path, handle });
            } else {
                unsafe { nabi_access_release(handle) };
            }
        }
        unsafe { nabi_access_free(paths.cast()) };
        unsafe { nabi_access_free(leases.cast()) };
    }
    Ok(Some(Selection {
        access: result,
        warning: (!message.is_null()).then(|| error(message)),
    }))
}
pub(crate) fn sandboxed() -> bool {
    unsafe { nabi_access_is_sandboxed() }
}
pub(crate) fn resolve(path: &Path) -> Result<Access, String> {
    if !CONFIGURED.load(Ordering::Acquire) {
        return Ok(Access {
            path: path.to_owned(),
            handle: std::ptr::null_mut(),
        });
    }
    let input = cpath(path)?;
    let (mut result, mut handle, mut message) = (
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    );
    if unsafe { nabi_access_resolve(input.as_ptr(), &mut result, &mut handle, &mut message) } != 0 {
        unsafe { nabi_access_release(handle) };
        let _ = take_path(result);
        return Err(error(message));
    }
    let Some(path) = take_path(result) else {
        unsafe { nabi_access_release(handle) };
        return Err("Could not resolve document location.".into());
    };
    if !message.is_null() {
        unsafe { nabi_access_free(message.cast()) };
    }
    Ok(Access { path, handle })
}
pub(crate) fn pick_again(path: &Path, folder: bool) -> Result<Option<Access>, String> {
    let input = cpath(path)?;
    let (mut result, mut message) = (std::ptr::null_mut(), std::ptr::null_mut());
    match unsafe { nabi_access_pick(input.as_ptr(), folder, &mut result, &mut message) } {
        0 => resolve(&take_path(result).ok_or("No document was selected.")?).map(Some),
        2 => Ok(None),
        _ => Err(error(message)),
    }
}
pub(crate) fn write_scope(folder: &Path) -> Result<Option<Access>, String> {
    let path = cpath(folder)?;
    let (mut handle, mut message) = (std::ptr::null_mut(), std::ptr::null_mut());
    match unsafe { nabi_access_write(path.as_ptr(), &mut handle, &mut message) } {
        0 => Ok(Some(Access {
            path: folder.to_owned(),
            handle,
        })),
        2 => Ok(None),
        _ => Err(error(message)),
    }
}
pub(crate) fn forget(removed: &Path, retained: &[PathBuf]) -> Result<(), String> {
    if !CONFIGURED.load(Ordering::Acquire) {
        return Ok(());
    }
    let removed = cpath(removed)?;
    let kept = retained
        .iter()
        .map(|path| cpath(path))
        .collect::<Result<Vec<_>, _>>()?;
    let pointers = kept.iter().map(|path| path.as_ptr()).collect::<Vec<_>>();
    let mut message = std::ptr::null_mut();
    if unsafe {
        nabi_access_forget(
            removed.as_ptr(),
            pointers.as_ptr(),
            pointers.len(),
            &mut message,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(error(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_eviction_reads_only_raw_local_references_and_keeps_errors() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let database = root.path().join("state.sqlite");
        let folder = root.path().join("photos");
        assert!(!crate::storage::mac_grant_references(&database).is_none());
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute("CREATE TABLE library(path BLOB)", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO library(path) VALUES (?1)",
                [folder.as_os_str().as_bytes()],
            )
            .unwrap();
        assert!(
            crate::storage::mac_grant_references(&database)
                .unwrap()
                .contains(&folder)
        );
        assert!(!crate::storage::mac_grant_references(&database).is_none());
        connection.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        assert!(crate::storage::mac_grant_references(&database).is_none());
        connection
            .execute_batch("ROLLBACK; DROP TABLE library;")
            .unwrap();
        assert!(crate::storage::mac_grant_references(&database).is_none());
    }
    #[test]
    fn native_bookmark_registry_restores_moved_paths_and_forgets_only_unused_grants() {
        // Ordinary host test: exercises real Foundation bookmark bytes and
        // lifetimes, but does not claim signed App Sandbox permission validation.
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let state = root.path().join("state");
        let photos = root.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        let first = photos.join("first.png");
        std::fs::write(&first, b"read only original").unwrap();
        configure(&state).unwrap();
        remember(&photos).unwrap();
        assert!(has_grant(&first));
        let grant = resolve(&first).unwrap();
        assert_eq!(std::fs::read(&grant.path).unwrap(), b"read only original");
        drop(grant);
        let moved = root.path().join("moved");
        std::fs::rename(&photos, &moved).unwrap();
        let grant = resolve(&first).unwrap();
        assert_eq!(
            std::fs::canonicalize(&grant.path).unwrap(),
            std::fs::canonicalize(moved.join("first.png")).unwrap()
        );
        assert_eq!(std::fs::read(&grant.path).unwrap(), b"read only original");
        drop(grant);
        forget(&photos, &[photos.clone()]).unwrap();
        assert!(has_grant(&first));
        std::fs::remove_file(moved.join("first.png")).unwrap();
        std::fs::remove_dir(&moved).unwrap();
        assert!(resolve(&first).is_err());
        assert!(has_grant(&first)); // An unavailable root does not erase saved access.
        forget(&photos, &[]).unwrap();
        assert!(!has_grant(&first));
        let stored = std::fs::read(state.join("macos-bookmarks.plist")).unwrap();
        assert!(stored.starts_with(b"bplist"));
    }
}
