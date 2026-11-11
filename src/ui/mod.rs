pub(crate) mod cursor_idle;
pub(crate) mod direction;
pub(crate) mod hdr;
pub(crate) mod mobile;
#[cfg(test)]
mod tests;
pub(crate) mod transparency;
pub(crate) mod viewer;

#[cfg(any(target_os = "linux", all(unix, test)))]
mod hdr_wayland;
#[cfg(any(target_os = "windows", test))]
mod hdr_windows;
