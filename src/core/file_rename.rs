//! Batch renaming in one folder, with preflight validation and no-clobber moves.
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub source: PathBuf,
    pub target: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug)]
pub(crate) struct Error {
    pub path: PathBuf,
    pub message: &'static str,
    pub detail: String,
}
impl Error {
    fn new(path: &Path, message: &'static str) -> Self {
        Self {
            path: path.into(),
            message,
            detail: String::new(),
        }
    }
    fn io(path: &Path, error: impl std::fmt::Display) -> Self {
        Self {
            path: path.into(),
            message: "rename-file-error",
            detail: error.to_string(),
        }
    }
    pub fn display(&self) -> String {
        let message = crate::i18n::tr(self.message);
        if self.detail.is_empty() {
            format!("{message}\n{}", self.path.display())
        } else {
            format!("{message}\n{}\n{}", self.path.display(), self.detail)
        }
    }
}

impl Plan {
    pub fn new(paths: &[PathBuf]) -> Self {
        Self {
            entries: paths
                .iter()
                .map(|path| Entry {
                    source: path.clone(),
                    target: path.clone(),
                })
                .collect(),
        }
    }
    pub fn digits(&self) -> usize {
        self.entries.len().max(1).to_string().len().max(4)
    }
    pub fn changes(&self) -> usize {
        self.entries.iter().filter(|e| e.source != e.target).count()
    }
    pub fn folder_names(&self, digits: usize) -> Result<Self, Error> {
        let mut next = self.clone();
        for (index, entry) in next.entries.iter_mut().enumerate() {
            let folder = entry
                .source
                .parent()
                .and_then(Path::file_name)
                .and_then(|v| v.to_str())
                .ok_or_else(|| Error::new(&entry.source, "rename-invalid-name"))?;
            let stem = format!(
                "{folder}_{:0width$}",
                index + 1,
                width = digits.clamp(1, 20)
            );
            entry.target = with_stem(&entry.source, &stem)?;
        }
        Ok(next)
    }
    pub fn replace(&self, from: &str, to: &str) -> Result<Self, Error> {
        if from.is_empty() {
            return Ok(self.clone());
        }
        let mut next = self.clone();
        for entry in &mut next.entries {
            let stem = entry
                .target
                .file_stem()
                .and_then(|v| v.to_str())
                .ok_or_else(|| Error::new(&entry.source, "rename-invalid-name"))?;
            entry.target = with_stem(&entry.source, &stem.replace(from, to))?;
        }
        Ok(next)
    }
}
fn with_stem(source: &Path, stem: &str) -> Result<PathBuf, Error> {
    if stem.is_empty() {
        return Err(Error::new(source, "rename-invalid-name"));
    }
    let mut name = std::ffi::OsString::from(stem);
    if let Some(extension) = source.extension() {
        name.push(".");
        name.push(extension);
    }
    Ok(source.with_file_name(name))
}

fn valid_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    !name.is_empty()
        && name.len() <= 255
        && name.encode_utf16().count() <= 255
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        && !matches!(
            base.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        && !["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if a.dev() != b.dev() || a.ino() != b.ino() {
            return false;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if a.creation_time() != b.creation_time() {
            return false;
        }
    }
    b.is_file() && a.len() == b.len() && a.modified().ok() == b.modified().ok()
}

/// A checked preview keeps file identities so applying cannot rename a replacement file.
#[derive(Clone)]
pub(crate) struct CheckedPlan {
    pub plan: Plan,
    metadata: Vec<fs::Metadata>,
}

pub(crate) fn check(plan: Plan, baseline: Option<&CheckedPlan>) -> Result<CheckedPlan, Error> {
    let folder = plan.entries.first().and_then(|e| e.source.parent());
    let mut sources = HashMap::new();
    let mut names: HashMap<String, usize> = HashMap::new();
    let mut metadata = Vec::with_capacity(plan.entries.len());
    for (index, entry) in plan.entries.iter().enumerate() {
        if entry.source.parent() != folder
            || entry.target.parent() != folder
            || sources.insert(entry.source.clone(), index).is_some()
            || (entry.source != entry.target
                && (!valid_name(&entry.target)
                    || entry.source.extension() != entry.target.extension()))
        {
            return Err(Error::new(&entry.target, "rename-invalid-name"));
        }
        let name = entry
            .target
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if let Some(previous) = names.insert(name, index)
            && (entry.source != entry.target
                || plan.entries[previous].source != plan.entries[previous].target)
        {
            return Err(Error::new(&entry.target, "rename-conflict"));
        }
        let stamp = fs::symlink_metadata(&entry.source).map_err(|e| Error::io(&entry.source, e))?;
        if !stamp.is_file() {
            return Err(Error::new(&entry.source, "rename-invalid-source"));
        }
        if let Some(before) = baseline
            && (before
                .plan
                .entries
                .get(index)
                .is_none_or(|e| e.source != entry.source)
                || before
                    .metadata
                    .get(index)
                    .is_none_or(|m| !same_file(m, &stamp)))
        {
            return Err(Error::new(&entry.source, "rename-source-changed"));
        }
        metadata.push(stamp);
    }
    for (index, entry) in plan
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.source != e.target)
    {
        match fs::symlink_metadata(&entry.target) {
            Ok(stamp) => {
                // A source in this batch will vacate its name. Case-only changes
                // on a case-insensitive volume are also safe through staging.
                if !sources.contains_key(&entry.target)
                    && !(entry
                        .source
                        .file_name()
                        .zip(entry.target.file_name())
                        .is_some_and(|(a, b)| {
                            a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
                        })
                        && same_file(&metadata[index], &stamp))
                {
                    return Err(Error::new(&entry.target, "rename-conflict"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(&entry.target, e)),
        }
    }
    Ok(CheckedPlan { plan, metadata })
}

#[derive(Default)]
pub(crate) struct RenameResult {
    /// Actual final locations, including retained recovery files after a failed rollback.
    pub renamed: Vec<(PathBuf, PathBuf)>,
    pub errors: Vec<Error>,
}

pub(crate) fn rename(checked: CheckedPlan) -> RenameResult {
    rename_with(checked, rename_noclobber)
}

fn rename_with(
    checked: CheckedPlan,
    mut move_file: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> RenameResult {
    let mut result = RenameResult::default();
    let checked = match check(checked.plan.clone(), Some(&checked)) {
        Ok(value) => value,
        Err(error) => {
            result.errors.push(error);
            return result;
        }
    };
    let changes: Vec<_> = checked
        .plan
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.source != e.target)
        .collect();
    let Some((_, first)) = changes.first() else {
        return result;
    };
    // Never let TempDir's recursive cleanup delete user files on an error/panic.
    let staging = match tempfile::Builder::new()
        .prefix(".nabi-rename-")
        .tempdir_in(first.source.parent().unwrap())
    {
        Ok(dir) => dir.keep(),
        Err(error) => {
            result.errors.push(Error::io(&first.source, error));
            return result;
        }
    };
    let parked: Vec<_> = changes
        .iter()
        .map(|(_, e)| staging.join(e.source.file_name().unwrap()))
        .collect();
    let mut current: Vec<_> = changes.iter().map(|(_, e)| e.source.clone()).collect();
    let operation = (|| -> Result<(), Error> {
        for (i, (index, entry)) in changes.iter().enumerate() {
            let stamp =
                fs::symlink_metadata(&entry.source).map_err(|e| Error::io(&entry.source, e))?;
            if !same_file(&checked.metadata[*index], &stamp) {
                return Err(Error::new(&entry.source, "rename-source-changed"));
            }
            move_file(&entry.source, &parked[i]).map_err(|e| Error::io(&entry.source, e))?;
            current[i].clone_from(&parked[i]);
            let staged = fs::symlink_metadata(&parked[i]).map_err(|e| Error::io(&parked[i], e))?;
            if !same_file(&checked.metadata[*index], &staged) {
                return Err(Error::new(&entry.source, "rename-source-changed"));
            }
        }
        for (i, (_, entry)) in changes.iter().enumerate() {
            move_file(&parked[i], &entry.target).map_err(|e| Error::io(&entry.target, e))?;
            current[i].clone_from(&entry.target);
        }
        Ok(())
    })();
    if let Err(error) = operation {
        result.errors.push(error);
        // Vacate published names first, otherwise exchanges cannot roll back.
        for (i, (_, entry)) in changes.iter().enumerate() {
            if current[i] == entry.target {
                match move_file(&current[i], &parked[i]) {
                    Ok(()) => current[i].clone_from(&parked[i]),
                    Err(error) => result.errors.push(Error::io(&current[i], error)),
                }
            }
        }
        for (i, (_, entry)) in changes.iter().enumerate() {
            if current[i] == parked[i] {
                match move_file(&parked[i], &entry.source) {
                    Ok(()) => current[i].clone_from(&entry.source),
                    Err(error) => result.errors.push(Error::io(&parked[i], error)),
                }
            }
        }
    }
    for ((_, entry), path) in changes.into_iter().zip(current) {
        if path != entry.source {
            result.renamed.push((entry.source.clone(), path));
        }
    }
    // remove_dir only removes an empty directory, preserving any recovery files.
    if let Err(error) = fs::remove_dir(&staging) {
        result.errors.push(Error::io(&staging, error));
    }
    result
}

#[cfg(target_os = "macos")]
fn rename_noclobber(source: &Path, target: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn renamex_np(
            from: *const std::ffi::c_char,
            to: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // Darwin RENAME_EXCL. Both C strings live for the entire call.
    if unsafe { renamex_np(source.as_ptr(), target.as_ptr(), 0x00000004) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
#[cfg(target_os = "linux")]
fn rename_noclobber(source: &Path, target: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // No fallback to overwriting rename when the filesystem lacks this flag.
    if unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
#[cfg(windows)]
fn rename_noclobber(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW},
        core::PCWSTR,
    };
    let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<_> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // Omitting REPLACE_EXISTING makes destination creation fail on collision.
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|error| io::Error::from_raw_os_error((error.code().0 & 0xffff) as i32))
}
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn rename_noclobber(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Renaming original files is unavailable on this platform.",
    ))
}

#[cfg(test)]
mod tests;
