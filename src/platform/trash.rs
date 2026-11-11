//! OS trash operations. Call from a worker, never from the UI thread.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct TrashEntry {
    pub original_path: PathBuf,
    token: platform::Token,
}

#[derive(Default, Debug)]
pub struct TrashBatchResult {
    pub trashed: Vec<TrashEntry>,
    pub failed: Vec<(PathBuf, String)>,
}

#[derive(Default, Debug)]
pub struct RestoreBatchResult {
    pub restored: Vec<PathBuf>,
    pub remaining: Vec<TrashEntry>,
    pub failed: Vec<(PathBuf, String)>,
}

pub fn move_to_trash(paths: Vec<PathBuf>) -> TrashBatchResult {
    let mut result = TrashBatchResult::default();
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        if !seen.insert(path.clone()) {
            continue;
        }
        let operation = (|| {
            // Do not follow symlinks or accept directories through this image command.
            let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if !metadata.is_file() {
                return Err("Only regular image files can be moved to the trash.".into());
            }
            let absolute = std::path::absolute(&path).map_err(|e| e.to_string())?;
            platform::trash(&absolute)
        })();
        match operation {
            Ok(token) => result.trashed.push(TrashEntry {
                original_path: path,
                token,
            }),
            Err(error) => result.failed.push((path, error)),
        }
    }
    result
}

pub fn restore_batch(entries: Vec<TrashEntry>) -> RestoreBatchResult {
    let mut result = RestoreBatchResult::default();
    for entry in entries.into_iter().rev() {
        let operation = ensure_absent(&entry.original_path)
            .and_then(|()| platform::restore(&entry.token, &entry.original_path));
        match operation {
            Ok(path) => result.restored.push(path),
            Err(error) => {
                result.failed.push((entry.original_path.clone(), error));
                result.remaining.push(entry);
            }
        }
    }
    // Keep the original deletion order so another attempt still reverses it.
    result.remaining.reverse();
    result
}

fn ensure_absent(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err("A file already exists at the original path; it was not overwritten.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::sync::{Arc, Mutex};
    use windows::Win32::Foundation::{E_ABORT, PROPERTYKEY};
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Shell::*;
    use windows::core::{Error, HRESULT, Interface, PCWSTR, Ref, implement};

    #[derive(Clone, Debug)]
    pub enum Token {
        Exact(OsString),
        Unresolved,
    }

    struct Com;
    impl Com {
        fn new() -> windows::core::Result<Self> {
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            }
            Ok(Self)
        }
    }
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    fn shell_path(path: &Path) -> PathBuf {
        // Shell parsing names cannot use std::fs::canonicalize's verbatim prefix.
        let units: Vec<_> = path
            .as_os_str()
            .encode_wide()
            .map(|unit| if unit == 47 { 92 } else { unit })
            .collect();
        if units.starts_with(&[92, 92, 63, 92, 85, 78, 67, 92]) {
            let mut normal = vec![92, 92];
            normal.extend_from_slice(&units[8..]);
            PathBuf::from(OsString::from_wide(&normal))
        } else if units.starts_with(&[92, 92, 63, 92]) && units.get(5) == Some(&58) {
            PathBuf::from(OsString::from_wide(&units[4..]))
        } else {
            PathBuf::from(OsString::from_wide(&units))
        }
    }

    fn item(name: &std::ffi::OsStr) -> windows::core::Result<IShellItem> {
        let name = wide(name);
        unsafe { SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None) }
    }

    fn parsing_name(item: &IShellItem) -> windows::core::Result<OsString> {
        unsafe {
            let name = item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)?;
            let result = OsString::from_wide(name.as_wide());
            CoTaskMemFree(Some(name.0.cast()));
            Ok(result)
        }
    }

    fn recycled_items() -> windows::core::Result<Vec<(OsString, PathBuf)>> {
        unsafe {
            let bin: IShellItem =
                SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)?;
            let items: IEnumShellItems = bin.BindToHandler(None, &BHID_EnumItems)?;
            let mut result = Vec::new();
            loop {
                let mut fetched = 0;
                let mut next = [None];
                items.Next(&mut next, Some(&mut fetched))?;
                if fetched == 0 {
                    break;
                }
                let item = next[0]
                    .as_ref()
                    .ok_or_else(|| Error::from_hresult(E_ABORT))?;
                let details: IShellItem2 = item.cast()?;
                let parent = details.GetString(&PROPERTYKEY {
                    fmtid: PSGUID_DISPLACED,
                    pid: PID_DISPLACED_FROM,
                })?;
                let parent_path = PathBuf::from(OsString::from_wide(parent.as_wide()));
                CoTaskMemFree(Some(parent.0.cast()));
                let name = item.GetDisplayName(SIGDN_PARENTRELATIVE)?;
                let path = parent_path.join(OsString::from_wide(name.as_wide()));
                CoTaskMemFree(Some(name.0.cast()));
                result.push((parsing_name(item)?, path));
            }
            Ok(result)
        }
    }

    fn find_pending(
        original: &Path,
        before: &std::collections::HashSet<OsString>,
    ) -> Result<OsString, String> {
        let mut matches = recycled_items()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|(id, path)| !before.contains(id) && path == original);
        let first = matches
            .next()
            .ok_or("The exact item is no longer in the Windows Recycle Bin.")?;
        if matches.next().is_some() {
            return Err("Several trash items match; nothing was restored.".into());
        }
        Ok(first.0)
    }

    #[derive(Default)]
    struct Outcome {
        token: Option<Token>,
        completed: bool,
        lookup_required: bool,
        restored_path: Option<PathBuf>,
        error: Option<String>,
    }
    #[implement(IFileOperationProgressSink)]
    struct Sink {
        outcome: Arc<Mutex<Outcome>>,
        destination: Option<PathBuf>,
    }
    #[allow(non_snake_case)]
    impl IFileOperationProgressSink_Impl for Sink_Impl {
        fn StartOperations(&self) -> windows::core::Result<()> {
            Ok(())
        }
        fn FinishOperations(&self, _: HRESULT) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreRenameItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            Err(Error::from_hresult(E_ABORT))
        }
        fn PostRenameItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreMoveItem(
            &self,
            flags: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            if flags & TSF_OVERWRITE_EXIST.0 as u32 != 0
                || self
                    .destination
                    .as_ref()
                    .is_none_or(|p| ensure_absent(p).is_err())
            {
                self.outcome.lock().unwrap().error = Some(format!(
                    "Restore refused an overwrite or a destination collision (flags {flags:#x})."
                ));
                return Err(Error::from_hresult(E_ABORT));
            }
            Ok(())
        }
        fn PostMoveItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            result: HRESULT,
            created: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            let mut outcome = self.outcome.lock().unwrap();
            if result.is_ok() && created.is_some() {
                let destination =
                    unsafe { created.as_ref().unwrap().GetDisplayName(SIGDN_FILESYSPATH) };
                match destination {
                    Ok(name) => {
                        let path = unsafe { PathBuf::from(OsString::from_wide(name.as_wide())) };
                        unsafe {
                            CoTaskMemFree(Some(name.0.cast()));
                        }
                        outcome.restored_path = Some(path);
                        outcome.completed = true;
                    }
                    Err(error) => {
                        outcome.error = Some(format!(
                            "Restore completed but its location could not be read: {error}"
                        ))
                    }
                }
            } else if outcome.error.is_none() {
                outcome.error = Some(format!("Restore did not complete: {result:?}"));
            }
            Ok(())
        }
        fn PreCopyItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            Err(Error::from_hresult(E_ABORT))
        }
        fn PostCopyItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreDeleteItem(
            &self,
            flags: u32,
            item: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            // The shell may fall back to permanent deletion when the volume or
            // execution environment cannot recycle. Never authorize that fallback.
            if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
                self.outcome.lock().unwrap().error = Some(
                    "The Windows Recycle Bin is unavailable for this file; the original was kept."
                        .into(),
                );
                return Err(Error::from_hresult(E_ABORT));
            }
            // A listed image may have become a directory since it was checked.
            unsafe {
                let item = item.as_ref().ok_or_else(|| Error::from_hresult(E_ABORT))?;
                let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
                let path = PathBuf::from(OsString::from_wide(name.as_wide()));
                CoTaskMemFree(Some(name.0.cast()));
                if !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
                    return Err(Error::from_hresult(E_ABORT));
                }
            }
            Ok(())
        }
        fn PostDeleteItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            result: HRESULT,
            created: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            let mut outcome = self.outcome.lock().unwrap();
            if result.is_ok() {
                if let Some(created) = created.as_ref() {
                    outcome.completed = true;
                    match parsing_name(created) {
                        Ok(name) => {
                            outcome.token = Some(Token::Exact(name));
                        }
                        Err(error) => {
                            outcome.error = Some(format!(
                                "Moved to trash but could not record its restore location: {error}"
                            ))
                        }
                    }
                } else {
                    // NULL can mean permanent deletion. Never call this success
                    // without independently locating the exact new recycle item.
                    outcome.lookup_required = true;
                }
            } else if outcome.error.is_none() {
                outcome.error = Some(format!("Move to trash failed: {result:?}"));
            }
            Ok(())
        }
        fn PreNewItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            Err(Error::from_hresult(E_ABORT))
        }
        fn PostNewItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: &PCWSTR,
            _: u32,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn UpdateProgress(&self, _: u32, _: u32) -> windows::core::Result<()> {
            Ok(())
        }
        fn ResetTimer(&self) -> windows::core::Result<()> {
            Ok(())
        }
        fn PauseTimer(&self) -> windows::core::Result<()> {
            Ok(())
        }
        fn ResumeTimer(&self) -> windows::core::Result<()> {
            Ok(())
        }
    }

    pub fn trash(path: &Path) -> Result<Token, String> {
        let _com = Com::new().map_err(|e| e.to_string())?;
        let before = recycled_items()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let outcome = Arc::new(Mutex::new(Outcome::default()));
        let sink: IFileOperationProgressSink = Sink {
            outcome: outcome.clone(),
            destination: None,
        }
        .into();
        let operation = (|| -> windows::core::Result<()> {
            unsafe {
                let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
                operation.SetOperationFlags(
                    FOF_NO_UI
                        | FOF_ALLOWUNDO
                        | FOF_WANTNUKEWARNING
                        | FOFX_RECYCLEONDELETE
                        | FOFX_EARLYFAILURE,
                )?;
                operation.DeleteItem(&item(shell_path(path).as_os_str())?, &sink)?;
                operation.PerformOperations()?;
                if operation.GetAnyOperationsAborted()?.as_bool() {
                    return Err(Error::from_hresult(E_ABORT));
                }
            }
            Ok(())
        })();
        let mut result = outcome.lock().unwrap();
        // A completed item is authoritative even if the aggregate operation aborted.
        if let Some(token) = result.token.take() {
            return Ok(token);
        }
        if result.completed || result.lookup_required {
            let original = shell_path(path);
            match find_pending(&original, &before) {
                Ok(id) => return Ok(Token::Exact(id)),
                Err(_) if result.completed => return Ok(Token::Unresolved),
                Err(error) => {
                    return Err(format!(
                        "The shell did not confirm a recoverable trash item: {error}"
                    ));
                }
            }
        }
        Err(result
            .error
            .take()
            .or_else(|| operation.err().map(|e| e.to_string()))
            .unwrap_or_else(|| "The shell did not move this file to the trash.".into()))
    }

    pub fn restore(token: &Token, original: &Path) -> Result<PathBuf, String> {
        let _com = Com::new().map_err(|e| e.to_string())?;
        let id = match token {
            Token::Exact(id) => id.clone(),
            Token::Unresolved => return Err("The exact trash identity was unavailable. Restore this item manually from the Recycle Bin.".into()),
        };
        let original = shell_path(original);
        let outcome = Arc::new(Mutex::new(Outcome::default()));
        let sink: IFileOperationProgressSink = Sink {
            outcome: outcome.clone(),
            destination: Some(original.clone()),
        }
        .into();
        let operation = (|| -> windows::core::Result<()> {
            let parent = original
                .parent()
                .ok_or_else(|| Error::from_hresult(E_ABORT))?;
            let name = wide(
                original
                    .file_name()
                    .ok_or_else(|| Error::from_hresult(E_ABORT))?,
            );
            unsafe {
                let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
                // No NOCONFIRMATION: collision must never be answered "Yes to all".
                operation.SetOperationFlags(
                    FOF_SILENT | FOF_NOERRORUI | FOFX_EARLYFAILURE | FOF_RENAMEONCOLLISION,
                )?;
                operation.MoveItem(
                    &item(&id)?,
                    &item(parent.as_os_str())?,
                    PCWSTR(name.as_ptr()),
                    &sink,
                )?;
                operation.PerformOperations()?;
                if operation.GetAnyOperationsAborted()?.as_bool() {
                    return Err(Error::from_hresult(E_ABORT));
                }
            }
            Ok(())
        })();
        let mut result = outcome.lock().unwrap();
        if let Some(path) = result.restored_path.take() {
            // RENAMEONCOLLISION guarantees no overwrite even if a file appears
            // after PreMoveItem. Report the actual restored path in that race.
            return Ok(path);
        }
        Err(result
            .error
            .take()
            .or_else(|| operation.err().map(|e| e.to_string()))
            .unwrap_or_else(|| "The shell did not restore this file.".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let directory = crate::test_support::root();
        std::fs::create_dir_all(&directory).unwrap();
        tempfile::Builder::new()
            .prefix("nabi-trash-test-")
            .tempdir_in(directory)
            .unwrap()
    }

    #[test]
    fn directories_and_missing_files_are_not_trashed() {
        let folder = fixture();
        let missing = folder.path().join("missing.png");
        let result = move_to_trash(vec![folder.path().to_owned(), missing]);
        assert!(result.trashed.is_empty());
        assert_eq!(result.failed.len(), 2);
        assert!(folder.path().is_dir());
    }

    #[test]
    #[ignore = "Uses OS trash with isolated test fixtures; run explicitly on a desktop"]
    fn native_trash_round_trip_keeps_failed_restore_for_retry() {
        let folder = fixture();
        let first = folder.path().join("01 한글.png");
        let second = folder.path().join("02 test.png");
        std::fs::write(&first, b"first fixture").unwrap();
        std::fs::write(&second, b"second fixture").unwrap();
        let result = move_to_trash(vec![first.clone(), second.clone()]);
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert_eq!(result.trashed.len(), 2);
        assert!(!first.exists() && !second.exists());

        std::fs::write(&first, b"new file must survive").unwrap();
        let restored = restore_batch(result.trashed);
        assert_eq!(
            restored.restored,
            vec![second.clone()],
            "{:?}",
            restored.failed
        );
        assert_eq!(restored.remaining.len(), 1);
        assert_eq!(std::fs::read(&first).unwrap(), b"new file must survive");
        assert_eq!(std::fs::read(&second).unwrap(), b"second fixture");
        // Remove only the collision fixture created above, then retry its saved token.
        std::fs::remove_file(&first).unwrap();
        let retry = restore_batch(restored.remaining);
        assert!(retry.failed.is_empty(), "{:?}", retry.failed);
        assert!(retry.remaining.is_empty());
        assert_eq!(std::fs::read(&first).unwrap(), b"first fixture");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct UnixFileIdentity {
    device: u64,
    inode: u64,
    created: Option<std::time::SystemTime>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unix_file_identity(path: &Path) -> Result<UnixFileIdentity, String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("The original regular file is no longer in the trash.".into());
    }
    Ok(UnixFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        created: metadata.created().ok(),
    })
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::collections::HashSet;
    use std::ffi::OsString;

    #[derive(Clone, Debug)]
    pub struct Token {
        original: PathBuf,
        item: Option<::trash::TrashItem>,
        identity: Option<UnixFileIdentity>,
    }

    pub fn trash(path: &Path) -> Result<Token, String> {
        let identity = unix_file_identity(path).ok();
        let before: HashSet<OsString> = ::trash::os_limited::list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|item| item.id)
            .collect();
        ::trash::delete(path).map_err(|e| e.to_string())?;
        let item = ::trash::os_limited::list().ok().and_then(|items| {
            let mut candidates = items
                .into_iter()
                .filter(|item| !before.contains(&item.id) && item.original_path() == path);
            let first = candidates.next()?;
            candidates.next().is_none().then_some(first)
        });
        // Never resolve missing identities later by path: a different file may
        // subsequently be trashed from the same path.
        Ok(Token {
            original: path.to_owned(),
            item,
            identity,
        })
    }

    pub fn restore(token: &Token, _: &Path) -> Result<PathBuf, String> {
        let expected = token.item.as_ref().ok_or(
            "The exact trash identity was unavailable. Restore this item manually from the Trash.",
        )?;
        let mut candidates: Vec<_> = ::trash::os_limited::list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|item| {
                item.original_path() == token.original
                    && item.id == expected.id
                    && item.time_deleted == expected.time_deleted
            })
            .collect();
        if candidates.len() != 1 {
            return Err(
                "The exact trash item is missing or ambiguous; nothing was restored.".into(),
            );
        }
        let item = candidates.pop().unwrap();
        let info = Path::new(&item.id);
        let info_dir = info.parent().ok_or("Invalid trash metadata location")?;
        if info_dir.file_name() != Some(std::ffi::OsStr::new("info"))
            || info.extension() != Some(std::ffi::OsStr::new("trashinfo"))
        {
            return Err("Invalid trash metadata location".into());
        }
        let source = info_dir
            .parent()
            .ok_or("Invalid trash directory")?
            .join("files")
            .join(info.file_stem().ok_or("Invalid trash item name")?);
        let expected_identity = token.identity.as_ref().ok_or(
            "The original file identity was unavailable. Restore this item manually from the Trash.",
        )?;
        if &unix_file_identity(&source)? != expected_identity {
            return Err(
                "The trash path now refers to a different file; nothing was restored.".into(),
            );
        }
        // Unlike a check followed by rename(), link creation fails atomically if
        // another process creates the destination in between. Trashed regular
        // files are on the same volume; unsupported filesystems fail safely.
        std::fs::hard_link(&source, &token.original).map_err(|e| e.to_string())?;
        // The original now owns the same data and metadata. Retaining a trash
        // copy after cleanup failure is safer than undoing the successful link.
        if std::fs::remove_file(&source).is_ok() {
            let _ = std::fs::remove_file(info);
        }
        Ok(token.original.clone())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::{CStr, CString, c_char};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    #[derive(Clone, Debug)]
    pub struct Token {
        path: PathBuf,
        identity: Option<UnixFileIdentity>,
    }
    unsafe extern "C" {
        fn nabi_trash_file(
            path: *const c_char,
            result: *mut *mut c_char,
            error: *mut *mut c_char,
        ) -> bool;
        fn nabi_restore_file(
            path: *const c_char,
            original: *const c_char,
            error: *mut *mut c_char,
        ) -> bool;
        fn nabi_trash_free(value: *mut c_char);
    }
    fn path_arg(path: &Path) -> Result<CString, String> {
        CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())
    }
    unsafe fn take_bytes(value: *mut c_char) -> Vec<u8> {
        if value.is_null() {
            return Vec::new();
        }
        let bytes = unsafe { CStr::from_ptr(value) }.to_bytes().to_vec();
        unsafe {
            nabi_trash_free(value);
        }
        bytes
    }
    pub fn trash(path: &Path) -> Result<Token, String> {
        let identity = unix_file_identity(path).ok();
        let path = path_arg(path)?;
        let mut result = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let ok = unsafe { nabi_trash_file(path.as_ptr(), &mut result, &mut error) };
        let message = unsafe { take_bytes(error) };
        let result = unsafe { take_bytes(result) };
        if ok && !result.is_empty() {
            Ok(Token {
                path: std::ffi::OsString::from_vec(result).into(),
                identity,
            })
        } else {
            Err(String::from_utf8_lossy(&message).into_owned())
        }
    }
    pub fn restore(token: &Token, original: &Path) -> Result<PathBuf, String> {
        let expected_identity = token.identity.as_ref().ok_or(
            "The original file identity was unavailable. Restore this item manually from the Trash.",
        )?;
        if &unix_file_identity(&token.path)? != expected_identity {
            return Err(
                "The trash path now refers to a different file; nothing was restored.".into(),
            );
        }
        let path = path_arg(&token.path)?;
        let original_arg = path_arg(original)?;
        let mut error = std::ptr::null_mut();
        let ok = unsafe { nabi_restore_file(path.as_ptr(), original_arg.as_ptr(), &mut error) };
        let message = unsafe { take_bytes(error) };
        if ok {
            Ok(original.to_owned())
        } else {
            Err(String::from_utf8_lossy(&message).into_owned())
        }
    }
}

#[cfg(any(target_os = "ios", target_os = "android"))]
mod platform {
    pub use super::parking::{Token, restore, trash};
}

/// Phones view private copies of picked documents. Removing an image parks
/// its copy beside the import batch, outside the listed folder, so it leaves
/// the list and can be restored. The batch removes parked copies when dropped.
#[cfg(any(target_os = "ios", target_os = "android", test))]
mod parking {
    use super::*;
    #[derive(Clone, Debug)]
    pub struct Token(PathBuf);

    pub fn trash(path: &Path) -> Result<Token, String> {
        let invalid = || "Invalid image path.".to_owned();
        let name = path.file_name().ok_or_else(invalid)?;
        let holding = path
            .parent()
            .and_then(crate::core::documents::removed_directory)
            .ok_or_else(invalid)?;
        std::fs::create_dir_all(&holding).map_err(|e| e.to_string())?;
        let slot = tempfile::Builder::new()
            .prefix("item-")
            .tempdir_in(&holding)
            .map_err(|e| e.to_string())?
            .keep();
        let parked = slot.join(name);
        if let Err(error) = std::fs::rename(path, &parked) {
            let _ = std::fs::remove_dir(&slot);
            return Err(error.to_string());
        }
        Ok(Token(parked))
    }

    pub fn restore(token: &Token, original: &Path) -> Result<PathBuf, String> {
        std::fs::rename(&token.0, original).map_err(|e| e.to_string())?;
        if let Some(slot) = token.0.parent() {
            let _ = std::fs::remove_dir(slot);
        }
        Ok(original.to_owned())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn parks_outside_the_batch_and_restores() {
            let cache = tempfile::tempdir().unwrap();
            let batch = cache.path().join("selection-test");
            std::fs::create_dir(&batch).unwrap();
            let image = batch.join("a.png");
            std::fs::write(&image, b"image").unwrap();
            let token = trash(&image).unwrap();
            assert!(!image.exists());
            assert!(
                token
                    .0
                    .starts_with(cache.path().join("selection-test.removed"))
            );
            assert_eq!(std::fs::read_dir(&batch).unwrap().count(), 0);
            assert_eq!(restore(&token, &image).unwrap(), image);
            assert_eq!(std::fs::read(&image).unwrap(), b"image");
            assert!(!token.0.exists());
        }
    }
}

#[cfg(not(any(
    windows,
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
)))]
mod platform {
    use super::*;
    #[derive(Clone, Debug)]
    pub struct Token;
    pub fn trash(_: &Path) -> Result<Token, String> {
        Err("Trash is unsupported on this platform.".into())
    }
    pub fn restore(_: &Token, _: &Path) -> Result<PathBuf, String> {
        Err("Trash is unsupported on this platform.".into())
    }
}
