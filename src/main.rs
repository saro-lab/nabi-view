#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn main() -> eframe::Result {
    nabi_view::run_desktop()
}

#[cfg(target_os = "ios")]
fn main() -> eframe::Result {
    nabi_view::run_ios()
}

#[cfg(target_os = "android")]
fn main() {
    // Android starts the cdylib through NativeActivity, not this executable.
}
