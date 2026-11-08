//! Select a file in the OS file manager. Call from a worker, never the UI thread.
use std::path::{Path, PathBuf};

pub fn supported() -> bool {
    cfg!(any(windows, target_os = "macos", target_os = "linux"))
}

pub fn reveal(path: &Path) -> Result<(), String> {
    if !supported() {
        return Err("Showing a file in the file manager is unavailable on this platform.".into());
    }
    platform::reveal(&file_path(path)?)
}

fn file_path(path: &Path) -> Result<PathBuf, String> {
    // Keep the selected directory entry, including a symlink, instead of its target.
    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_file() && !metadata.is_symlink() {
        return Err("Only a file can be selected with this command.".into());
    }
    Ok(path)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        Common::ITEMIDLIST, SHOpenFolderAndSelectItems, SHParseDisplayName,
    };
    use windows::core::PCWSTR;

    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    struct ItemId(*mut ITEMIDLIST);
    impl Drop for ItemId {
        fn drop(&mut self) {
            unsafe { CoTaskMemFree(Some(self.0.cast())) };
        }
    }

    fn shell_path(path: &Path) -> Result<Vec<u16>, String> {
        // Shell parsing names use ordinary drive/UNC paths. Preserve UTF-16 even
        // for names that cannot be represented as a Rust String.
        let units: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .map(|unit| if unit == 47 { 92 } else { unit })
            .collect();
        if units.contains(&0) {
            return Err("A file path cannot contain a null character.".into());
        }
        let mut normal = if units.starts_with(&[92, 92, 63, 92, 85, 78, 67, 92]) {
            let mut normal = vec![92, 92];
            normal.extend_from_slice(&units[8..]);
            normal
        } else if units.starts_with(&[92, 92, 63, 92]) && units.get(5) == Some(&58) {
            units[4..].to_vec()
        } else {
            units
        };
        normal.push(0);
        Ok(normal)
    }

    pub fn reveal(path: &Path) -> Result<(), String> {
        let path = shell_path(path)?;
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|error| error.to_string())?;
            let _com = Com;
            let mut item = ItemId(std::ptr::null_mut());
            SHParseDisplayName(PCWSTR(path.as_ptr()), None, &mut item.0, 0, None)
                .map_err(|error| error.to_string())?;
            // With zero child IDs, the absolute item ID identifies the file to
            // select and the Shell opens its parent directory.
            SHOpenFolderAndSelectItems(item.0, None, 0).map_err(|error| error.to_string())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        #[test]
        fn shell_path_keeps_utf16_and_removes_only_verbatim_prefixes() {
            for (input, expected) in [
                (r"\\?\C:\사진, $(file).png", r"C:\사진, $(file).png"),
                (r"\\?\UNC\server\사진\a.png", r"\\server\사진\a.png"),
            ] {
                let actual = shell_path(Path::new(input)).unwrap();
                let expected: Vec<_> = expected.encode_utf16().chain(Some(0)).collect();
                assert_eq!(actual, expected);
            }
            let units = [67, 58, 92, 0xd800, 46, 112, 110, 103];
            let path = PathBuf::from(OsString::from_wide(&units));
            assert_eq!(
                shell_path(&path).unwrap(),
                [units.as_slice(), &[0]].concat()
            );
            let path = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0, 97]));
            assert!(shell_path(&path).is_err());
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    pub fn reveal(path: &Path) -> Result<(), String> {
        // The absolute path is one argv item and cannot become an option even
        // when the file name begins with a dash. No shell interprets the name.
        let mut command = std::process::Command::new("/usr/bin/open");
        command.arg("-R").arg(path);
        run_command(&mut command)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    pub fn reveal(path: &Path) -> Result<(), String> {
        let mut command = file_manager_command(path.as_os_str().as_bytes())?;
        run_command(&mut command)
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod platform {
    use super::*;

    pub fn reveal(_: &Path) -> Result<(), String> {
        Err("Showing a file in the file manager is unavailable on this platform.".into())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_command(command: &mut std::process::Command) -> Result<(), String> {
    let output = command
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let error = String::from_utf8_lossy(&output.stderr);
    let error = error.trim();
    if error.is_empty() {
        Err(format!("File manager request failed: {}", output.status))
    } else {
        Err(error.to_owned())
    }
}

#[cfg(any(target_os = "linux", test))]
fn file_manager_command(path: &[u8]) -> Result<std::process::Command, String> {
    let uri = file_uri(path)?;
    let mut command = std::process::Command::new("dbus-send");
    command.args([
        "--session",
        "--print-reply",
        "--reply-timeout=5000",
        "--type=method_call",
        "--dest=org.freedesktop.FileManager1",
        "/org/freedesktop/FileManager1",
        "org.freedesktop.FileManager1.ShowItems",
    ]);
    // URI escaping also prevents dbus-send's comma-separated array syntax from
    // splitting a single file name into multiple item parameters.
    command.arg(format!("array:string:{uri}")).arg("string:");
    Ok(command)
}

#[cfg(any(target_os = "linux", test))]
fn file_uri(path: &[u8]) -> Result<String, String> {
    if !path.starts_with(b"/") || path.contains(&0) {
        return Err("A file URI needs an absolute path without null characters.".into());
    }
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut uri = String::from("file://");
    for &byte in path {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push('%');
            uri.push(char::from(HEX[usize::from(byte >> 4)]));
            uri.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_file_uri_preserves_special_characters_as_one_item() {
        let path = "/사진/a,b $(x);\"'#%\n.png";
        let expected = "file:///%EC%82%AC%EC%A7%84/a%2Cb%20%24%28x%29%3B%22%27%23%25%0A.png";
        assert_eq!(file_uri(path.as_bytes()).unwrap(), expected);
        assert_eq!(file_uri(b"/tmp/\xff.png").unwrap(), "file:///tmp/%FF.png");
        assert!(file_uri(b"relative.png").is_err());
        assert!(file_uri(b"/tmp/\0.png").is_err());
        let command = file_manager_command(path.as_bytes()).unwrap();
        let arguments: Vec<_> = command.get_args().collect();
        assert_eq!(arguments.len(), 9);
        assert_eq!(arguments[6], "org.freedesktop.FileManager1.ShowItems");
        assert_eq!(arguments[7], format!("array:string:{expected}").as_str());
        assert_eq!(arguments[8], "string:");
    }

    #[test]
    fn target_validation_rejects_missing_files_and_directories_without_launching() {
        let root = crate::test_support::root();
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        assert!(file_path(directory.path()).is_err());
        assert!(file_path(&directory.path().join("missing.png")).is_err());
        let path = directory.path().join("사진, $(file).png");
        std::fs::write(&path, b"unchanged").unwrap();
        assert_eq!(file_path(&path).unwrap(), path);
        assert_eq!(std::fs::read(path).unwrap(), b"unchanged");
    }

    #[cfg(unix)]
    #[test]
    fn target_validation_preserves_the_selected_symlink() {
        let root = crate::test_support::root();
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let target = directory.path().join("image.png");
        let link = directory.path().join("selected.png");
        std::fs::write(&target, b"unchanged").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(file_path(&link).unwrap(), link);
    }
}
