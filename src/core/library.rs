//! Recent folders and bookmarks: one history model and one thumbnail cache.
//!
//! Thumbnails are small RGBA PNG copies kept in the OS application cache. A file
//! is named after the image path, size and modification time, so an edited
//! image gets a new thumbnail. Files that no entry references are pruned.
//!
//! A zip or 7z archive is listed like a folder. Its pages are named by their
//! path inside it (`book.zip/ch1/001.jpg`).
use crate::core::archive;
use eframe::egui;
use image::{ImageEncoder, ImageReader, Limits};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Longest edge of a cached thumbnail.
pub(crate) const THUMBNAIL_EDGE: u32 = 256;
const THUMBNAIL_EXTENSION: &str = "png";
const LEGACY_THUMBNAIL_EXTENSION: &str = "jpg";
const THUMBNAIL_VERSION: &[u8] = b"nabi-thumbnail-v3-color-managed-rgba-png";
const MAX_NAME_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kind {
    #[default]
    Recent,
    Bookmark,
}

impl Kind {
    pub(crate) const ALL: [Self; 2] = [Self::Recent, Self::Bookmark];

    /// Stored in the state database; never renumber.
    pub(crate) fn code(self) -> i64 {
        match self {
            Self::Recent => 0,
            Self::Bookmark => 1,
        }
    }

    pub(crate) fn from_code(code: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub folder: PathBuf,
    /// The saved page while it exists, otherwise the folder's first image.
    /// Opening the entry starts here and the thumbnail shows it.
    pub image: Option<PathBuf>,
    /// Bookmark title; the folder name is shown without one.
    pub name: Option<String>,
    /// Cache key of the thumbnail last written for this entry.
    pub thumbnail: Option<String>,
    /// Bookmarks stay listed while their folder cannot be opened.
    pub available: bool,
}

impl Entry {
    pub fn title(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| folder_name(&self.folder))
    }
}

pub(crate) fn folder_name(folder: &Path) -> String {
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    if let Some(name) = crate::core::documents::source_name(folder) {
        return name;
    }
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string())
}

/// The folder's first image in natural order, if it can be read and has one.
pub(crate) fn first_image(folder: &Path) -> Option<PathBuf> {
    #[cfg(all(target_os = "macos", not(test)))]
    let access = crate::platform::mac_access::resolve(folder).ok()?;
    #[cfg(all(target_os = "macos", not(test)))]
    let folder = access.path.as_path();
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    if crate::core::documents::source_is_container(folder) {
        return crate::core::documents::source_first_page(folder);
    }
    if is_archive_file(folder) {
        return archive::first_page(folder).map(|page| folder.join(page));
    }
    crate::core::media::images_in_folder(folder)
        .ok()?
        .min_by(|a, b| crate::core::media::compare_file_names(a, b))
}

/// An archive that the library lists like a folder.
pub(crate) fn is_archive_file(path: &Path) -> bool {
    archive::is_archive(path) && path.is_file()
}

/// The folder or archive holding `image`, and the image's path within it.
pub(crate) fn container(image: &Path) -> Option<(PathBuf, PathBuf)> {
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    if let Some(source) = crate::core::documents::source_record(image) {
        let inner = image.strip_prefix(&source.root).ok()?.to_owned();
        return (!inner.as_os_str().is_empty()).then_some((source.root, inner));
    }
    archive::split(image).or_else(|| {
        Some((
            image.parent()?.to_owned(),
            PathBuf::from(image.file_name()?),
        ))
    })
}

/// True while `image` can be opened: a file, or a page of an existing archive.
/// Pages are not looked up inside the archive, so lists never read archives.
pub(crate) fn page_exists(image: &Path) -> bool {
    #[cfg(all(target_os = "macos", not(test)))]
    let access = match crate::platform::mac_access::resolve(image) {
        Ok(access) => access,
        Err(_) => return false,
    };
    #[cfg(all(target_os = "macos", not(test)))]
    let image = access.path.as_path();
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    if crate::core::documents::source_record(image).is_some() {
        return crate::core::documents::source_page_exists(image);
    }
    image.is_file() || archive::split(image).is_some()
}

/// A single-line title, or None to fall back to the folder name.
pub(crate) fn clean_name(name: &str) -> Option<String> {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Cache key for the current contents of `image`, or None when it is not a file.
/// A page inside an archive follows the archive file.
pub(crate) fn thumbnail_key(image: &Path) -> Option<String> {
    #[cfg(all(target_os = "macos", not(test)))]
    let access = crate::platform::mac_access::resolve(image).ok()?;
    #[cfg(all(target_os = "macos", not(test)))]
    let image = access.path.as_path();
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    let local =
        crate::core::documents::source_local_path(image).unwrap_or_else(|| image.to_owned());
    #[cfg(not(any(target_os = "ios", target_os = "android", test)))]
    let local = image.to_owned();
    let file = archive::split(&local).map_or(local, |(archive, _)| archive);
    let metadata = fs::metadata(file).ok().filter(|m| m.is_file())?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |time| time.as_nanos());
    let mut bytes = image.as_os_str().as_encoded_bytes().to_vec();
    bytes.extend(metadata.len().to_le_bytes());
    bytes.extend(modified.to_le_bytes());
    // Never reuse the old JPEGs that flattened transparency over a dark matte.
    bytes.extend(THUMBNAIL_VERSION);
    bytes.extend(THUMBNAIL_EDGE.to_le_bytes());
    // A fixed hash keeps names stable across releases and toolchains.
    let fnv = |basis: u64| {
        bytes.iter().fold(basis, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    };
    Some(format!(
        "{:016x}{:016x}",
        fnv(0xcbf2_9ce4_8422_2325),
        fnv(0x6c62_272e_07bb_0142)
    ))
}

pub(crate) fn is_thumbnail_key(key: &str) -> bool {
    key.len() == 32 && key.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The cache file name for `key`, if `name` is one.
pub(crate) fn thumbnail_key_of_file(name: &str) -> Option<&str> {
    name.strip_suffix(THUMBNAIL_EXTENSION)
        .or_else(|| name.strip_suffix(LEGACY_THUMBNAIL_EXTENSION))
        .and_then(|stem| stem.strip_suffix('.'))
        .filter(|key| is_thumbnail_key(key))
}

pub(crate) fn thumbnail_file(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.{THUMBNAIL_EXTENSION}"))
}

/// Reads one thumbnail without writing a cache file. Logical mobile sources
/// and archive pages use the same bounded decoder whether disk caching is on
/// or off.
pub(crate) fn decode_thumbnail(image: &Path) -> Result<egui::ColorImage, String> {
    #[cfg(all(target_os = "macos", not(test)))]
    let access = crate::platform::mac_access::resolve(image)?;
    #[cfg(all(target_os = "macos", not(test)))]
    let image = access.path.as_path();
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    let local =
        crate::core::documents::source_local_path(image).unwrap_or_else(|| image.to_owned());
    #[cfg(not(any(target_os = "ios", target_os = "android", test)))]
    let local = image.to_owned();
    let image = local.as_path();
    let frame = match archive::split(image) {
        // Only this page is extracted, and only while it is decoded.
        Some((file, inner)) => {
            let page = archive::extract_page(&file, &inner).map_err(|e| e.to_string())?;
            crate::core::media::decode_still(&page.pages[0].file, THUMBNAIL_EDGE)?
        }
        None => crate::core::media::decode_still(image, THUMBNAIL_EDGE)?,
    };
    Ok(frame.image)
}

/// Decodes `image`, stores its thumbnail as `key` and returns the pixels to show.
pub(crate) fn create_thumbnail(
    image: &Path,
    dir: &Path,
    key: &str,
) -> Result<egui::ColorImage, String> {
    if !is_thumbnail_key(key) {
        return Err("Invalid thumbnail name.".into());
    }
    let image = decode_thumbnail(image)?;
    let [width, height] = image.size;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in &image.pixels {
        // PNG stores straight alpha; Color32 stores premultiplied colors.
        rgba.extend_from_slice(&pixel.to_srgba_unmultiplied());
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &rgba,
            width as u32,
            height as u32,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Write beside the target and rename, so readers never see a partial file.
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    temp.write_all(&png).map_err(|e| e.to_string())?;
    temp.persist(thumbnail_file(dir, key))
        .map_err(|e| e.error.to_string())?;
    Ok(image)
}

/// Reads a cached thumbnail written by `create_thumbnail`.
pub(crate) fn load_thumbnail(dir: &Path, key: &str) -> Result<egui::ColorImage, String> {
    if !is_thumbnail_key(key) {
        return Err("Invalid thumbnail name.".into());
    }
    let png_path = thumbnail_file(dir, key);
    let (mut reader, format, used_path) = match ImageReader::open(&png_path) {
        Ok(reader) => (reader, image::ImageFormat::Png, png_path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Keep offline bookmarks useful until their source can be read again.
            let legacy = dir.join(format!("{key}.{LEGACY_THUMBNAIL_EXTENSION}"));
            let reader = ImageReader::open(&legacy).map_err(|e| e.to_string())?;
            (reader, image::ImageFormat::Jpeg, legacy)
        }
        Err(error) => return Err(error.to_string()),
    };
    reader.set_format(format);
    // The cache is ours, but it is still a file on disk: bound what it can cost.
    let mut limits = Limits::default();
    limits.max_image_width = Some(4 * THUMBNAIL_EDGE);
    limits.max_image_height = Some(4 * THUMBNAIL_EDGE);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
    let _ = crate::core::disk_cache::touch(&used_path);
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_single_line_and_bounded() {
        assert_eq!(clean_name("  Comics \n "), Some("Comics".into()));
        assert_eq!(clean_name(" \t "), None);
        assert_eq!(clean_name("a\u{7}b"), Some("ab".into()));
        assert_eq!(clean_name(&"x".repeat(500)).unwrap().chars().count(), 120);
    }

    #[test]
    fn thumbnails_follow_image_contents_and_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("page.png");
        let cache = root.path().join("thumbnails");
        assert_eq!(thumbnail_key(&image), None);
        image::RgbaImage::from_pixel(600, 300, image::Rgba([200, 40, 40, 255]))
            .save(&image)
            .unwrap();
        let key = thumbnail_key(&image).unwrap();
        assert!(is_thumbnail_key(&key));
        assert_eq!(thumbnail_key(&image), Some(key.clone()));

        let shown = create_thumbnail(&image, &cache, &key).unwrap();
        assert_eq!(shown.size, [256, 128]);
        let file = thumbnail_file(&cache, &key);
        let name = file.file_name().unwrap().to_str().unwrap();
        assert_eq!(thumbnail_key_of_file(name), Some(key.as_str()));
        let loaded = load_thumbnail(&cache, &key).unwrap();
        assert_eq!(loaded.size, [256, 128]);
        let pixel = loaded.pixels[128 * 64 + 128];
        assert!(pixel.r() > 180 && pixel.g() < 70, "{pixel:?}");
        // Only the finished file remains.
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);

        // A changed image gets a different cache file.
        image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 0]))
            .save(&image)
            .unwrap();
        let changed = thumbnail_key(&image).unwrap();
        assert_ne!(changed, key);
        // Fully transparent pixels stay transparent after reopening the cache.
        let transparent = create_thumbnail(&image, &cache, &changed).unwrap();
        assert_eq!(transparent.size, [10, 10]);
        let pixel = load_thumbnail(&cache, &changed).unwrap().pixels[0];
        assert_eq!(pixel.a(), 0, "{pixel:?}");

        assert!(load_thumbnail(&cache, "../settings").is_err());
        assert!(create_thumbnail(&image, &cache, "not-a-key").is_err());
        assert_eq!(thumbnail_key_of_file("notes.jpg"), None);
        assert_eq!(
            thumbnail_key_of_file(&format!("{key}.jpg")),
            Some(key.as_str())
        );
        assert_eq!(thumbnail_key_of_file(&format!("{key}.webp")), None);
    }

    #[test]
    fn cached_thumbnails_preserve_all_alpha_levels_and_color() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("alpha.png");
        let cache = root.path().join("thumbnails");
        image::RgbaImage::from_fn(256, 1, |x, _| image::Rgba([220, 80, 40, x as u8]))
            .save(&image)
            .unwrap();
        let original = fs::read(&image).unwrap();
        let key = thumbnail_key(&image).unwrap();
        let shown = create_thumbnail(&image, &cache, &key).unwrap();
        let loaded = load_thumbnail(&cache, &key).unwrap();
        assert_eq!(loaded.size, shown.size);
        for (alpha, (before, after)) in shown.pixels.iter().zip(&loaded.pixels).enumerate() {
            assert_eq!(before.a(), alpha as u8);
            assert_eq!(after.a(), alpha as u8);
            for (before, after) in before.to_array().into_iter().zip(after.to_array()) {
                assert!(
                    before.abs_diff(after) <= 1,
                    "Alpha {alpha}: {before} -> {after}"
                );
            }
        }
        // The PNG stores straight-alpha color, not the darker premultiplied bytes.
        let png = image::open(thumbnail_file(&cache, &key))
            .unwrap()
            .into_rgba8();
        for (actual, expected) in png
            .get_pixel(128, 0)
            .0
            .into_iter()
            .zip([220_u8, 80, 40, 128])
        {
            assert!(actual.abs_diff(expected) <= 2, "{actual} != {expected}");
        }
        assert_eq!(fs::read(&image).unwrap(), original);
    }

    #[test]
    fn legacy_thumbnails_remain_readable_until_sources_are_available() {
        let root = tempfile::tempdir().unwrap();
        let key = "a".repeat(32);
        let legacy = root.path().join(format!("{key}.jpg"));
        image::RgbImage::from_pixel(8, 8, image::Rgb([28, 30, 31]))
            .save(&legacy)
            .unwrap();
        assert_eq!(load_thumbnail(root.path(), &key).unwrap().size, [8, 8]);

        let source = root.path().join("source.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([220, 80, 40, 128]))
            .save(&source)
            .unwrap();
        create_thumbnail(&source, root.path(), &key).unwrap();
        // Prefer RGBA PNG even if a legacy file with this key is also present.
        assert_eq!(
            load_thumbnail(root.path(), &key).unwrap().pixels[0].a(),
            128
        );
        assert!(legacy.is_file());
    }
}
