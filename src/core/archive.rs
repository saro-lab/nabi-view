//! Zip and 7z archives opened as read-only folders.
//!
//! The images of an archive are extracted to a private temporary folder, so
//! decoders, thumbnails and metadata keep reading ordinary files. Users and
//! saved state name a page as `archive/inner/path`: recent items and bookmarks
//! store that path, and opening it opens the archive at that page.
use std::{
    collections::HashMap,
    fs,
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub const EXTENSIONS: &[&str] = &["zip", "7z"];
const MAX_PAGES: usize = 10_000;
const MAX_PAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_NAME_CHARS: usize = 120;
const TEMP_PREFIX: &str = "nabi-archive-";
/// Folders left behind by a crash are removed once they are this old.
const STALE_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The archive is encrypted and no password was given.
    PasswordRequired,
    /// The given password does not decrypt the archive.
    WrongPassword,
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PasswordRequired => f.write_str("The archive needs a password."),
            Self::WrongPassword => f.write_str("The password is incorrect."),
            Self::Cancelled => f.write_str("Cancelled"),
            Self::Failed(error) => write!(f, "Could not open the archive. {error}"),
        }
    }
}

/// True for a path named like a supported archive, whether or not it exists.
pub fn is_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.iter().any(|a| ext.eq_ignore_ascii_case(a)))
}

/// Splits a page inside an archive (`book.zip/ch1/001.jpg`) into the archive
/// file and the page's path within it.
pub fn split(path: &Path) -> Option<(PathBuf, PathBuf)> {
    path.ancestors()
        .skip(1)
        .find(|ancestor| is_archive(ancestor) && ancestor.is_file())
        .and_then(|archive| {
            let inner = path.strip_prefix(archive).ok()?;
            (!inner.as_os_str().is_empty()).then(|| (archive.to_owned(), inner.to_owned()))
        })
}

/// One extracted image.
#[derive(Debug)]
pub struct Page {
    /// Path inside the archive, as the user knows it.
    pub inner: PathBuf,
    /// The extracted file the viewer reads.
    pub file: PathBuf,
    /// Entry metadata captured while reading, never the extraction file's time.
    pub(crate) sort: crate::core::media::FileSortMetadata,
}

/// Extracted images, in natural order of their path inside the archive. The
/// files are deleted when this is dropped.
pub struct Extracted {
    dir: Option<tempfile::TempDir>,
    pub pages: Vec<Page>,
    /// Images left out because of size or count limits.
    pub skipped: usize,
}

impl Extracted {
    #[cfg(test)]
    pub fn root(&self) -> &Path {
        self.dir.as_ref().map_or(Path::new(""), |dir| dir.path())
    }
}

impl Drop for Extracted {
    fn drop(&mut self) {
        let Some(dir) = self.dir.take() else { return };
        let path = dir.keep();
        if fs::remove_dir_all(&path).is_ok() {
            return;
        }
        // A decoder may still hold a file open; Windows refuses to delete it.
        let _ = std::thread::Builder::new()
            .name("archive-cleanup".into())
            .spawn(move || {
                for _ in 0..40 {
                    std::thread::sleep(Duration::from_millis(250));
                    if fs::remove_dir_all(&path).is_ok() || !path.exists() {
                        return;
                    }
                }
            });
    }
}

/// Extracts the archive's images into a new private folder.
pub fn extract(
    archive: &Path,
    password: Option<&str>,
    live: &dyn Fn() -> bool,
) -> Result<Extracted, Error> {
    remove_stale_folders();
    let dir = tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .tempdir()
        .map_err(|e| Error::Failed(e.to_string()))?;
    let mut writer = Writer::new(dir.path(), live);
    read_archive(archive, password, &mut writer, &|_| true)?;
    let Writer {
        mut pages, skipped, ..
    } = writer;
    pages.sort_by(|a, b| crate::core::media::compare_paths(&a.inner, &b.inner));
    Ok(Extracted {
        dir: Some(dir),
        pages,
        skipped,
    })
}

/// Extracts a single page, for a thumbnail of an archive that is not open.
pub fn extract_page(archive: &Path, inner: &Path) -> Result<Extracted, Error> {
    let dir = tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .tempdir()
        .map_err(|e| Error::Failed(e.to_string()))?;
    let mut writer = Writer::new(dir.path(), &|| true);
    writer.limit = 1;
    read_archive(archive, None, &mut writer, &|path| path == inner)?;
    let pages = std::mem::take(&mut writer.pages);
    if pages.is_empty() {
        return Err(Error::Failed("The page is not in the archive.".into()));
    }
    Ok(Extracted {
        dir: Some(dir),
        pages,
        skipped: 0,
    })
}

/// The first image in the archive, read from its file list without extracting.
pub fn first_page(archive: &Path) -> Option<PathBuf> {
    #[cfg(all(target_os = "macos", not(test)))]
    let access = crate::platform::mac_access::resolve(archive).ok()?;
    #[cfg(all(target_os = "macos", not(test)))]
    let archive = access.path.as_path();
    let names: Vec<PathBuf> = if is_zip(archive) {
        let file = fs::File::open(archive).ok()?;
        let zip = zip::ZipArchive::new(BufReader::new(file)).ok()?;
        (0..zip.len())
            .filter_map(|i| {
                let entry = zip.by_index_data(i).ok()?;
                entry.is_file().then(|| inner_path(&entry.name().ok()?))?
            })
            .collect()
    } else {
        let archive = sevenz_rust2::Archive::open(archive).ok()?;
        archive
            .files
            .iter()
            .filter(|entry| entry.has_stream && !entry.is_directory)
            .filter_map(|entry| inner_path(&entry.name))
            .collect()
    };
    names
        .into_iter()
        .filter(|name| crate::core::media::is_supported(name))
        .min_by(|a, b| crate::core::media::compare_paths(a, b))
}

fn is_zip(archive: &Path) -> bool {
    archive
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

fn read_archive(
    archive: &Path,
    password: Option<&str>,
    writer: &mut Writer,
    wanted: &dyn Fn(&Path) -> bool,
) -> Result<(), Error> {
    #[cfg(all(target_os = "macos", not(test)))]
    let access = crate::platform::mac_access::resolve(archive).map_err(Error::Failed)?;
    #[cfg(all(target_os = "macos", not(test)))]
    let archive = access.path.as_path();
    let file = fs::File::open(archive).map_err(|e| Error::Failed(e.to_string()))?;
    if is_zip(archive) {
        read_zip(file, password, writer, wanted)
    } else {
        read_7z(file, password, writer, wanted)
    }
}

fn read_zip(
    file: fs::File,
    password: Option<&str>,
    writer: &mut Writer,
    wanted: &dyn Fn(&Path) -> bool,
) -> Result<(), Error> {
    let mut zip =
        zip::ZipArchive::new(BufReader::new(file)).map_err(|e| Error::Failed(e.to_string()))?;
    let mut images = Vec::new();
    for index in 0..zip.len() {
        let entry = zip
            .by_index_data(index)
            .map_err(|e| Error::Failed(e.to_string()))?;
        if !entry.is_file() {
            continue;
        }
        let Some(inner) = entry.name().ok().and_then(|name| inner_path(&name)) else {
            continue;
        };
        if crate::core::media::is_supported(&inner) && wanted(&inner) {
            images.push((index, inner, entry.encrypted(), entry.size()));
        }
    }
    // Ask once, before anything is written.
    if password.is_none() && images.iter().any(|image| image.2) {
        return Err(Error::PasswordRequired);
    }
    for (index, inner, encrypted, size) in images {
        if writer.full() {
            break;
        }
        let entry = match (encrypted, password) {
            (true, Some(password)) => zip.by_index_decrypt(index, password.as_bytes()),
            _ => zip.by_index(index),
        };
        let mut entry = match entry {
            Ok(entry) => entry,
            Err(zip::result::ZipError::InvalidPassword) => return Err(Error::WrongPassword),
            Err(_) => {
                writer.skipped += 1;
                continue;
            }
        };
        // ZIP DOS date/time has no timezone. Preserve its chronological key
        // within this archive instead of inventing a UTC timestamp.
        let modified = entry
            .last_modified()
            .map(|time| (u128::from(time.datepart()) << 16) | u128::from(time.timepart()));
        match writer.add(inner, size, modified, &mut entry) {
            Ok(()) => {}
            Err(Error::Failed(_)) if encrypted => return Err(Error::WrongPassword),
            Err(Error::Failed(_)) => writer.skipped += 1,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn read_7z(
    file: fs::File,
    password: Option<&str>,
    writer: &mut Writer,
    wanted: &dyn Fn(&Path) -> bool,
) -> Result<(), Error> {
    use sevenz_rust2::{ArchiveReader, EntryDecision, Error as SevenError, Password};
    let key = password.map_or_else(Password::empty, Password::new);
    let failed = |error: SevenError| match error {
        SevenError::PasswordRequired if password.is_none() => Error::PasswordRequired,
        SevenError::PasswordRequired | SevenError::MaybeBadPassword(_) => Error::WrongPassword,
        // A wrong key decrypts to garbage, which fails checks or parsing.
        SevenError::FileOpen(..) | SevenError::Unsupported(_) => Error::Failed(error.to_string()),
        SevenError::UnsupportedCompressionMethod(_) => Error::Failed(error.to_string()),
        _ if password.is_some() => Error::WrongPassword,
        _ => Error::Failed(error.to_string()),
    };
    let mut reader = ArchiveReader::new(BufReader::new(file), key).map_err(&failed)?;
    let archive = reader.archive();
    if password.is_none()
        && (0..archive.files.len()).any(|index| {
            let entry = &archive.files[index];
            archive.is_file_encrypted(index)
                && entry.has_stream
                && inner_path(&entry.name)
                    .is_some_and(|inner| crate::core::media::is_supported(&inner) && wanted(&inner))
        })
    {
        return Err(Error::PasswordRequired);
    }
    let mut stopped: Option<Error> = None;
    let result = reader.for_each_entries_with_decision(|entry, data| {
        if writer.full() {
            return Ok(EntryDecision::Stop);
        }
        if entry.is_directory || !entry.has_stream {
            return Ok(EntryDecision::Continue);
        }
        let Some(inner) = inner_path(&entry.name) else {
            return Ok(EntryDecision::Continue);
        };
        if !crate::core::media::is_supported(&inner) || !wanted(&inner) {
            return Ok(EntryDecision::Continue);
        }
        let modified = entry
            .has_last_modified_date
            .then_some(u128::from(u64::from(entry.last_modified_date)));
        match writer.add(inner, entry.size, modified, data) {
            Ok(()) => Ok(EntryDecision::Continue),
            Err(Error::Failed(error)) => {
                // The decoder's own error says whether the key was wrong.
                Err(SevenError::Other(error.into()))
            }
            Err(error) => {
                stopped = Some(error);
                Ok(EntryDecision::Stop)
            }
        }
    });
    if let Some(error) = stopped {
        return Err(error);
    }
    result.map_err(&failed)
}

/// A safe relative path from an archive entry name, or None for names that
/// leave the archive (`..`) or name nothing.
fn inner_path(name: &str) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            part if part.contains('\0') => return None,
            part => path.push(part),
        }
    }
    (!path.as_os_str().is_empty()).then_some(path)
}

/// A file name every supported file system accepts, keeping the extension.
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let path = Path::new(&cleaned);
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem: String = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    let stem = stem.trim_end_matches([' ', '.']);
    let stem = if stem.is_empty() || reserved_device(stem) {
        format!("_{stem}")
    } else {
        stem.to_owned()
    };
    if extension.is_empty() {
        stem
    } else {
        format!("{stem}.{extension}")
    }
}

fn reserved_device(stem: &str) -> bool {
    let base = stem
        .split('.')
        .next()
        .unwrap_or(stem)
        .trim_end()
        .to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit())
}

/// Deletes extraction folders that an earlier crash left behind.
fn remove_stale_folders() {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > STALE_AGE);
        if old {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

struct Writer<'a> {
    root: &'a Path,
    live: &'a dyn Fn() -> bool,
    pages: Vec<Page>,
    /// Extraction folder for each folder inside the archive.
    folders: HashMap<PathBuf, PathBuf>,
    bytes: u64,
    skipped: usize,
    limit: usize,
}

impl<'a> Writer<'a> {
    fn new(root: &'a Path, live: &'a dyn Fn() -> bool) -> Self {
        Self {
            root,
            live,
            pages: Vec::new(),
            folders: HashMap::new(),
            bytes: 0,
            skipped: 0,
            limit: MAX_PAGES,
        }
    }

    fn full(&self) -> bool {
        self.pages.len() >= self.limit || self.bytes >= MAX_TOTAL_BYTES
    }

    /// Copies one image. Size limits skip the image; read errors are returned.
    fn add(
        &mut self,
        inner: PathBuf,
        size: u64,
        modified: Option<u128>,
        data: &mut dyn Read,
    ) -> Result<(), Error> {
        if !(self.live)() {
            return Err(Error::Cancelled);
        }
        let limit = MAX_PAGE_BYTES.min(MAX_TOTAL_BYTES.saturating_sub(self.bytes));
        if self.full() || size > limit {
            self.skipped += 1;
            return Ok(());
        }
        let parent = inner.parent().unwrap_or(Path::new("")).to_owned();
        let count = self.folders.len();
        let folder = self
            .folders
            .entry(parent)
            .or_insert_with(|| self.root.join(count.to_string()))
            .clone();
        fs::create_dir_all(&folder).map_err(|e| Error::Failed(e.to_string()))?;
        let name = safe_name(&inner.file_name().unwrap_or_default().to_string_lossy());
        let (mut output, file) = create_unique(&folder, &name)?;
        let mut input = LiveReader {
            inner: data,
            live: self.live,
        }
        .take(limit + 1);
        let copied = io::copy(&mut input, &mut output).and_then(|n| output.flush().map(|()| n));
        drop(output);
        let copied = match copied {
            Ok(copied) => copied,
            Err(error) => {
                let _ = fs::remove_file(&file);
                return Err(if (self.live)() {
                    Error::Failed(error.to_string())
                } else {
                    Error::Cancelled
                });
            }
        };
        if copied > limit || copied == 0 {
            let _ = fs::remove_file(&file);
            self.skipped += 1;
            return Ok(());
        }
        self.bytes += copied;
        self.pages.push(Page {
            inner,
            file,
            sort: crate::core::media::FileSortMetadata {
                bytes: copied,
                modified,
            },
        });
        Ok(())
    }
}

/// Archives may hold names that only differ by case or by cleaned characters.
fn create_unique(folder: &Path, name: &str) -> Result<(fs::File, PathBuf), Error> {
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = path.extension().map(|e| e.to_string_lossy());
    for suffix in 0..10_000 {
        let candidate = match (suffix, &extension) {
            (0, _) => name.to_owned(),
            (n, Some(ext)) => format!("{stem} ({n}).{ext}"),
            (n, None) => format!("{stem} ({n})"),
        };
        let file = folder.join(candidate);
        match fs::File::options().write(true).create_new(true).open(&file) {
            Ok(output) => return Ok((output, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::Failed(e.to_string())),
        }
    }
    Err(Error::Failed("Too many files with the same name.".into()))
}

struct LiveReader<'a> {
    inner: &'a mut dyn Read,
    live: &'a dyn Fn() -> bool,
}

impl Read for LiveReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !(self.live)() {
            return Err(io::Error::other("Cancelled"));
        }
        self.inner.read(buf)
    }
}

#[cfg(test)]
mod tests;
