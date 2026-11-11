//! Local image viewer. Platform hosts share the same application and image pipeline.
mod app;
mod core;
mod i18n;
mod platform;
pub mod preview;
mod storage;
#[cfg(test)]
mod test_support;
mod ui;

#[cfg(target_os = "android")]
pub use android_activity;
#[cfg(target_os = "android")]
pub use app::run_android;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub use app::run_desktop;
#[cfg(target_os = "ios")]
pub use app::run_ios;
