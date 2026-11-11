//! User-facing path spelling. Windows `fs::canonicalize` returns verbatim
//! paths (`\\?\C:\…`, `\\?\UNC\server\…`); show and store the ordinary form
//! whenever it names the same file.
use std::{
    io,
    path::{Path, PathBuf},
};

/// `fs::canonicalize` without Windows verbatim prefixes when they are not needed.
pub(crate) fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::fs::canonicalize(path).map(simplified)
}

/// Removes a Windows verbatim prefix when the ordinary path means the same.
/// Paths that need the prefix (reserved device names, trailing dots or
/// spaces, `.`/`..` components, volume GUIDs) are returned unchanged.
#[cfg(windows)]
pub(crate) fn simplified(path: PathBuf) -> PathBuf {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
    };
    const SLASH: u16 = b'\\' as u16;
    const VERBATIM: [u16; 4] = [SLASH, SLASH, b'?' as u16, SLASH];
    const UNC: [u16; 4] = [b'U' as u16, b'N' as u16, b'C' as u16, SLASH];

    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let Some(rest) = units.strip_prefix(&VERBATIM) else {
        return path;
    };
    let normal = if let Some(unc) = rest.strip_prefix(&UNC) {
        // `\\?\UNC\server\share\…` → `\\server\share\…`
        let mut parts = unc.splitn(3, |&u| u == SLASH);
        let (Some(server), Some(share)) = (parts.next(), parts.next()) else {
            return path;
        };
        if server.is_empty() || share.is_empty() || !plain_components(parts.next()) {
            return path;
        }
        [&[SLASH, SLASH][..], unc].concat()
    } else if let [drive, colon, SLASH, tail @ ..] = rest
        && *colon == b':' as u16
        && u8::try_from(*drive).is_ok_and(|d| d.is_ascii_alphabetic())
    {
        // `\\?\C:\…` → `C:\…`. A bare `\\?\C:` would become drive-relative.
        if !plain_components(Some(tail)) {
            return path;
        }
        rest.to_vec()
    } else {
        return path;
    };
    PathBuf::from(OsString::from_wide(&normal))
}

#[cfg(not(windows))]
pub(crate) fn simplified(path: PathBuf) -> PathBuf {
    path
}

/// True when every component keeps its meaning without the verbatim prefix.
#[cfg(windows)]
fn plain_components(tail: Option<&[u16]>) -> bool {
    let Some(tail) = tail else {
        return true;
    };
    let mut components: Vec<&[u16]> = tail.split(|&u| u == b'\\' as u16).collect();
    // A single trailing separator is harmless; empty inner components are not.
    if components.last().is_some_and(|c| c.is_empty()) {
        components.pop();
    }
    components.into_iter().all(plain_component)
}

#[cfg(windows)]
fn plain_component(name: &[u16]) -> bool {
    let Some(&last) = name.last() else {
        return false;
    };
    // Win32 path parsing strips trailing dots and spaces and rewrites `/`.
    if last == b'.' as u16 || last == b' ' as u16 || name.contains(&(b'/' as u16)) {
        return false;
    }
    !reserved_device(name)
}

/// DOS device names stay reserved with any extension: `CON`, `nul.txt`, `COM1 .png`.
#[cfg(windows)]
fn reserved_device(name: &[u16]) -> bool {
    let stem = name.split(|&u| u == b'.' as u16).next().unwrap_or(name);
    let stem = String::from_utf16_lossy(stem);
    let stem = stem.trim_end_matches(' ').to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        _ => {
            let mut chars = stem.chars();
            let prefix: String = chars.by_ref().take(3).collect();
            let digit = chars.next();
            (prefix == "COM" || prefix == "LPT")
                && chars.next().is_none()
                && digit.is_some_and(|d| d.is_ascii_digit() || matches!(d, '¹' | '²' | '³'))
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn check(input: &str, expected: &str) {
        assert_eq!(simplified(PathBuf::from(input)), PathBuf::from(expected));
    }

    #[test]
    fn removes_unneeded_verbatim_prefixes() {
        check(r"\\?\C:\", r"C:\");
        check(r"\\?\C:\사진\앨범", r"C:\사진\앨범");
        check(r"\\?\d:\a b\c.d", r"d:\a b\c.d");
        check(r"\\?\UNC\server\share\사진", r"\\server\share\사진");
        check(r"\\?\UNC\server\share", r"\\server\share");
        check(r"C:\already\plain", r"C:\already\plain");
        check(r"\\server\share\plain", r"\\server\share\plain");
    }

    #[test]
    fn keeps_prefix_when_the_plain_form_names_something_else() {
        for path in [
            r"\\?\C:",
            r"\\?\C:\trailing.",
            r"\\?\C:\trailing \x",
            r"\\?\C:\a\..\b",
            r"\\?\C:\a\.\b",
            r"\\?\C:\a\\b",
            r"\\?\C:\CON",
            r"\\?\C:\dir\nul.txt",
            r"\\?\C:\dir\Com1 .png",
            r"\\?\C:\LPT²",
            r"\\?\UNC\server",
            r"\\?\UNC\\share",
            r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\a",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\a",
        ] {
            check(path, path);
        }
        // Names that only resemble device names are ordinary.
        check(
            r"\\?\C:\CONSOLE\COM10\LPT\aux1",
            r"C:\CONSOLE\COM10\LPT\aux1",
        );
    }

    #[test]
    fn canonicalize_returns_plain_paths() {
        let dir = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let path = canonicalize(dir.path()).unwrap();
        assert!(!path.as_os_str().to_string_lossy().starts_with(r"\\?\"));
        assert_eq!(
            std::fs::canonicalize(&path).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
    }
}
