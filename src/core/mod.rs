pub(crate) mod archive;
pub(crate) mod cache;
pub(crate) mod commands;
pub(crate) mod disk_cache;
#[cfg(any(target_os = "ios", target_os = "android", test))]
pub(crate) mod documents;
pub(crate) mod file_move;
pub(crate) mod file_rename;
#[cfg(any(target_os = "ios", test))]
pub(crate) mod ios_access;
pub(crate) mod library;
pub(crate) mod media;
pub(crate) mod metadata;
pub(crate) mod paths;
#[cfg(any(target_os = "ios", target_os = "android", test))]
pub(crate) mod photo_library;
pub(crate) mod settings;

/// Built for a phone. The desktop phone preview is a runtime mode instead;
/// see `crate::app::is_mobile`.
pub(crate) const NATIVE_MOBILE: bool = cfg!(any(target_os = "ios", target_os = "android"));
