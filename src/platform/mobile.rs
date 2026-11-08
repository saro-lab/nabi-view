//! Small native boundary shared by the iOS and Android hosts.
use crate::{core::documents::ImportBatch, storage::Store};
use eframe::egui;
use std::{
    ffi::{CStr, c_char},
    fs::File,
    os::fd::FromRawFd,
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

unsafe extern "C" {
    fn nabi_mobile_path(kind: i32) -> *const c_char;
    fn nabi_mobile_pick();
    fn nabi_mobile_safe_area(insets: *mut f32);
    fn nabi_mobile_keep_awake(enabled: bool);
    fn nabi_mobile_copy_text(text: *const u16, length: i32);
}

struct Paths {
    data: PathBuf,
    cache: PathBuf,
    bundle: PathBuf,
}
static BACK: AtomicBool = AtomicBool::new(false);
static PATHS: OnceLock<Paths> = OnceLock::new();
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();
static IMPORT: Mutex<Option<Result<ImportBatch, String>>> = Mutex::new(None);
static RESULT: Mutex<Option<Result<ImportBatch, String>>> = Mutex::new(None);

fn init_paths() {
    let path = |kind| {
        let p = unsafe { nabi_mobile_path(kind) };
        if p.is_null() {
            PathBuf::new()
        } else {
            PathBuf::from(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
        }
    };
    let paths = Paths {
        data: path(0),
        cache: path(1),
        bundle: path(2),
    };
    // Only clean this app's abandoned import sessions; settings are separate.
    if paths.cache.is_absolute() {
        if let Ok(entries) = std::fs::read_dir(paths.cache.join("imports")) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("selection-")
                    && entry.file_type().is_ok_and(|t| t.is_dir())
                {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
    }
    let _ = PATHS.set(paths);
}

#[cfg(target_os = "ios")]
pub fn init_ios() {
    init_paths();
}

#[cfg(target_os = "android")]
pub fn init_android(app: &android_activity::AndroidApp) {
    unsafe extern "C" {
        fn nabi_android_init(vm: *mut std::ffi::c_void, activity: *mut std::ffi::c_void);
    }
    unsafe {
        nabi_android_init(app.vm_as_ptr(), app.activity_as_ptr());
    }
    init_paths();
}

pub fn set_context(ctx: egui::Context) {
    let _ = CONTEXT.set(ctx);
}
fn repaint() {
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

pub fn store() -> Result<Store, String> {
    let paths = PATHS.get().ok_or("Mobile host is not initialized")?;
    if !paths.data.is_absolute() {
        return Err("App storage is unavailable".into());
    }
    Ok(Store::in_directories(
        paths.data.join("config"),
        paths.data.join("state"),
    ))
}

pub fn sample_path() -> Option<PathBuf> {
    let cache = &PATHS.get()?.cache;
    if !cache.is_absolute() {
        return None;
    }
    let directory = cache.join("sample-v1");
    std::fs::create_dir_all(&directory).ok()?;
    let path = directory.join("01 Alpine Light.png");
    if !path.exists() {
        std::fs::write(
            &path,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/field-notes/01 Alpine Light.png"
            )),
        )
        .ok()?;
    }
    Some(path)
}

pub fn notices_path() -> Option<PathBuf> {
    let paths = PATHS.get()?;
    [
        paths.bundle.join("THIRD_PARTY_NOTICES.txt"),
        paths.cache.join("THIRD_PARTY_NOTICES.txt"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

pub fn font_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "ios")]
    {
        unsafe extern "C" {
            fn nabi_mobile_font() -> *const c_char;
        }
        let p = unsafe { nabi_mobile_font() };
        if p.is_null() {
            Vec::new()
        } else {
            vec![PathBuf::from(
                unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned(),
            )]
        }
    }
    #[cfg(target_os = "android")]
    {
        vec![
            PathBuf::from("/system/fonts/NotoSansCJK-Regular.ttc"),
            PathBuf::from("/system/fonts/NotoSansCJK-VF.ttc"),
        ]
    }
}

pub fn pick() {
    unsafe {
        nabi_mobile_pick();
    }
}
pub fn take_result() -> Option<Result<ImportBatch, String>> {
    RESULT.lock().unwrap().take()
}
pub fn safe_area() -> [f32; 4] {
    let mut result = [0.; 4];
    unsafe {
        nabi_mobile_safe_area(result.as_mut_ptr());
    }
    result
}
pub fn keep_awake(enabled: bool) {
    unsafe {
        nabi_mobile_keep_awake(enabled);
    }
}

#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_import_begin() -> bool {
    let batch = PATHS
        .get()
        .filter(|p| p.cache.is_absolute())
        .ok_or_else(|| "App cache is unavailable".to_owned())
        .and_then(|paths| ImportBatch::new(&paths.cache.join("imports")));
    let ok = batch.is_ok();
    *IMPORT.lock().unwrap() = Some(batch);
    ok
}

fn import_fd(fd: i32, name: &str) -> bool {
    if fd < 0 {
        import_error("The selected document could not be opened.".into());
        return false;
    }
    // The native picker transfers an owned descriptor. Close it on every path.
    let source = unsafe { File::from_raw_fd(fd) };
    let mut pending = IMPORT.lock().unwrap();
    let Some(Ok(batch)) = pending.as_mut() else {
        return false;
    };
    match batch.add(name, source) {
        Ok(()) => true,
        Err(error) => {
            *pending = Some(Err(error));
            false
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn nabi_mobile_import_file(fd: i32, name: *const c_char) -> bool {
    let name = if name.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    };
    import_fd(fd, &name)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
unsafe extern "C" fn nabi_mobile_import_utf16(fd: i32, name: *const u16, length: usize) -> bool {
    let name = if name.is_null() {
        String::new()
    } else {
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(name, length) })
    };
    import_fd(fd, &name)
}

fn import_error(error: String) {
    *IMPORT.lock().unwrap() = Some(Err(error));
}

#[unsafe(no_mangle)]
unsafe extern "C" fn nabi_mobile_import_error(message: *const c_char) {
    let message = if message.is_null() {
        "Could not read the selected document.".into()
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };
    import_error(message);
}

#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_import_finish() {
    if let Some(result) = IMPORT.lock().unwrap().take() {
        // A cancelled/empty picker leaves the current selection untouched.
        if result
            .as_ref()
            .map_or(true, |batch| batch.first().is_some())
        {
            *RESULT.lock().unwrap() = Some(result);
            repaint();
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_back() {
    BACK.store(true, Ordering::Relaxed);
    repaint();
}
pub fn take_back() -> bool {
    BACK.swap(false, Ordering::Relaxed)
}
#[cfg(target_os = "android")]
pub fn exit() {
    unsafe extern "C" {
        fn nabi_mobile_exit();
    }
    unsafe {
        nabi_mobile_exit();
    }
}

pub fn clipboard(ctx: &egui::Context) {
    ctx.output_mut(|output| {
        output.commands.retain(|command| {
            if let egui::OutputCommand::CopyText(text) = command {
                let text: Vec<u16> = text.encode_utf16().collect();
                if let Ok(length) = i32::try_from(text.len()) {
                    unsafe {
                        nabi_mobile_copy_text(text.as_ptr(), length);
                    }
                }
                false
            } else {
                true
            }
        })
    });
}
