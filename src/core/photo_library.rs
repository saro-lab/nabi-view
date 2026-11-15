//! Built-in mobile photo destinations. These are native collections, not paths
//! into shared storage, and do not imply that photo access has been granted.
use std::path::{Path, PathBuf};

pub(crate) const ANDROID: &[(&str, &str)] = &[
    ("nabi-android-camera", "photo-camera"),
    ("nabi-android-pictures", "photo-pictures"),
    ("nabi-android-screenshots", "photo-screenshots"),
];
pub(crate) const IOS: &[(&str, &str)] = &[
    ("nabi-ios-library", "photo-library"),
    ("nabi-ios-screenshots", "photo-screenshots"),
    ("nabi-ios-favorites", "photo-favorites"),
];
// Keep older Favorites bookmarks readable without adding one to new installs.
#[cfg(any(target_os = "ios", test))]
pub(crate) const IOS_DEFAULTS: &[(&str, &str)] = &[IOS[0], IOS[1]];

#[cfg(any(target_os = "ios", test))]
pub(crate) fn is_ios(id: &str) -> bool {
    IOS.iter().any(|(key, _)| *key == id)
}

pub(crate) fn label(id: &str) -> Option<&'static str> {
    ANDROID
        .iter()
        .chain(IOS)
        .find(|(key, _)| *key == id)
        .map(|(_, label)| crate::i18n::tr(label))
}

pub(crate) fn title(path: &Path) -> Option<String> {
    #[cfg(any(target_os = "ios", test))]
    let (_, id) = super::documents::source_identity(path)?;
    #[cfg(not(any(target_os = "ios", test)))]
    let id = super::documents::source_record(path)?.id;
    label(&id).map(str::to_owned)
}

pub(crate) fn create_source(sources: &Path, id: &str) -> Result<PathBuf, String> {
    let name = label(id).ok_or("Unknown photo collection")?;
    super::documents::create_empty_source(sources, id, name)
}
