//! Small native boundary shared by the iOS and Android hosts.
use crate::{core::documents::ImportBatch, storage::Store};
use eframe::egui;
use std::{
    ffi::{CStr, c_char},
    fs::File,
    io::Read,
    os::fd::FromRawFd,
    path::{Path, PathBuf},
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
    fn nabi_mobile_pick_folder();
    fn nabi_mobile_set_orientation(mode: i32);
    fn nabi_mobile_set_immersive(enabled: bool);
    fn nabi_mobile_share(path: *const u16, length: i32);
    fn nabi_mobile_export_settings(path: *const u16, length: i32);
    fn nabi_mobile_import_settings();
    fn nabi_mobile_reopen(id: *const u16, length: i32) -> bool;
}

/// Upper bound for a picked settings file; the store applies its own limit.
const SETTINGS_FILE_LIMIT: u64 = 16 * 1024 * 1024;

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
static SETTINGS: Mutex<Option<Result<PathBuf, String>>> = Mutex::new(None);
static EXPORT_FAILED: AtomicBool = AtomicBool::new(false);
static REOPEN: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Orientation mode and immersive state last sent to the host.
static DISPLAY: Mutex<Option<(i32, bool)>> = Mutex::new(None);

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
    let normalize = |path: PathBuf| {
        if path.is_absolute() && std::fs::create_dir_all(&path).is_ok() {
            crate::core::paths::canonicalize(&path).unwrap_or(path)
        } else {
            path
        }
    };
    let paths = Paths {
        data: normalize(paths.data),
        cache: normalize(paths.cache),
        bundle: paths.bundle,
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
    if !paths.data.is_absolute() || !paths.cache.is_absolute() {
        return Err("App storage is unavailable".into());
    }
    Ok(Store::in_directories_with_cache(
        paths.data.join("config"),
        paths.data.join("state"),
        paths.cache.clone(),
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

#[cfg(target_os = "android")]
#[derive(Clone, Copy, Debug)]
pub struct HdrDisplay {
    /// Desired content peak from current display capabilities, in cd/m². This
    /// is not a live luminance measurement or a full-frame brightness limit.
    pub desired_max_nits: Option<f32>,
    /// API 34+ current target HDR peak / current SDR white. Some(1) means there
    /// is currently no extra headroom; None means the OS cannot report it.
    pub hdr_sdr_ratio: Option<f32>,
}

/// Reads the Activity's current display, including external-screen moves.
/// Neither absolute current SDR white nor HDR surface readiness is implied.
/// A renderer using nominal 203-nit SDR white must identify that as an
/// assumption, and independently require an HDR-capable swapchain format.
#[cfg(target_os = "android")]
pub fn hdr_display() -> Option<HdrDisplay> {
    unsafe extern "C" {
        fn nabi_mobile_hdr(desired_max_nits: *mut f32, hdr_sdr_ratio: *mut f32) -> bool;
    }
    let (mut peak, mut ratio) = (0., 0.);
    // SAFETY: the JNI bridge writes two floats synchronously and releases its
    // local references/thread attachment before returning.
    if !unsafe { nabi_mobile_hdr(&mut peak, &mut ratio) } {
        return None;
    }
    let desired_max_nits = (peak.is_finite() && peak > 0.).then_some(peak);
    let hdr_sdr_ratio = (ratio.is_finite() && ratio >= 1.).then_some(ratio);
    (desired_max_nits.is_some() || hdr_sdr_ratio.is_some()).then_some(HdrDisplay {
        desired_max_nits,
        hdr_sdr_ratio,
    })
}
pub fn keep_awake(enabled: bool) {
    unsafe {
        nabi_mobile_keep_awake(enabled);
    }
}
pub fn pick_folder() {
    unsafe {
        nabi_mobile_pick_folder();
    }
}

/// Reopens a locally registered source through its native grant. The result is
/// asynchronous and uses take_result(), including revoked/unavailable grants.
pub fn reopen(path: &Path) -> bool {
    if !source_available(path) {
        return false;
    }
    let Some(record) = crate::core::documents::source_record(path) else {
        return false;
    };
    let Some(paths) = PATHS.get() else {
        return false;
    };
    if record.root.parent() != Some(paths.data.join("document-sources").as_path()) {
        return false;
    }
    {
        let mut pending = REOPEN.lock().unwrap();
        // A second click can arrive before the native worker calls begin_import.
        // Preserve that worker's requested page instead of replacing it with
        // a request the native busy guard will reject.
        if pending.is_some() {
            return true;
        }
        *pending = Some(path.to_owned());
    }
    let id: Vec<u16> = record.id.encode_utf16().collect();
    if !unsafe { nabi_mobile_reopen(id.as_ptr(), id.len() as i32) } {
        REOPEN.lock().unwrap().take();
        *RESULT.lock().unwrap() = Some(Err(
            "Another document operation is still in progress.".into()
        ));
        repaint();
    }
    true
}

pub fn source_available(path: &Path) -> bool {
    let Some(paths) = PATHS.get() else {
        return false;
    };
    crate::core::documents::source_record(path).is_some_and(|record| {
        record.root.parent() == Some(paths.data.join("document-sources").as_path())
    })
}

/// Applies the orientation lock (`auto`, `portrait`, `landscape`) and
/// immersive full screen. Cheap to call every frame; only changes reach the host.
pub fn sync_display(orientation: &str, immersive: bool) {
    let mode = match orientation {
        "portrait" => 1,
        "landscape" => 2,
        _ => 0,
    };
    let mut applied = DISPLAY.lock().unwrap();
    let previous = applied.replace((mode, immersive));
    unsafe {
        if previous.is_none_or(|(old, _)| old != mode) {
            nabi_mobile_set_orientation(mode);
        }
        if previous.is_none_or(|(_, old)| old != immersive) {
            nabi_mobile_set_immersive(immersive);
        }
    }
}

fn with_utf16(path: &Path, send: impl FnOnce(*const u16, i32)) {
    let text: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    if let Ok(length) = i32::try_from(text.len()) {
        send(text.as_ptr(), length);
    }
}

/// Opens the system share sheet for a viewer-owned file.
pub fn share(path: &Path) {
    with_utf16(path, |text, length| unsafe {
        nabi_mobile_share(text, length)
    });
}

pub fn settings_export_path() -> Result<PathBuf, String> {
    let cache = &PATHS
        .get()
        .filter(|p| p.cache.is_absolute())
        .ok_or("App cache is unavailable")?
        .cache;
    let directory = cache.join("exports");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    Ok(directory.join("nabi-view-settings.json"))
}
/// Lets the user save an exported settings file with the system picker.
pub fn export_settings(path: &Path) {
    with_utf16(path, |text, length| unsafe {
        nabi_mobile_export_settings(text, length)
    });
}
pub fn take_export_failed() -> bool {
    EXPORT_FAILED.swap(false, Ordering::Relaxed)
}
pub fn import_settings() {
    unsafe {
        nabi_mobile_import_settings();
    }
}
pub fn take_settings_import() -> Option<Result<PathBuf, String>> {
    SETTINGS.lock().unwrap().take()
}

fn begin_import(folder: bool) -> bool {
    let requested = REOPEN.lock().unwrap().take();
    let mut batch = PATHS
        .get()
        .filter(|p| p.cache.is_absolute())
        .ok_or_else(|| "App cache is unavailable".to_owned())
        .and_then(|paths| {
            let cache = paths.cache.join("imports");
            if folder {
                ImportBatch::new_folder(&cache)
            } else {
                ImportBatch::new(&cache)
            }
        });
    if let Ok(batch) = &mut batch {
        batch.request_page(requested);
    }
    let ok = batch.is_ok();
    *IMPORT.lock().unwrap() = Some(batch);
    ok
}

#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_import_begin() -> bool {
    begin_import(false)
}

/// Starts importing every image of a picked folder; other files are skipped.
#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_import_begin_folder() -> bool {
    begin_import(true)
}

fn import_source(id: &str, name: &str) {
    let mut pending = IMPORT.lock().unwrap();
    if let (Some(Ok(batch)), Some(paths)) = (pending.as_mut(), PATHS.get()) {
        if let Err(error) = batch.set_source(&paths.data.join("document-sources"), id, name) {
            *pending = Some(Err(error));
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn nabi_mobile_import_source(id: *const c_char, name: *const c_char) {
    if id.is_null() || name.is_null() {
        return;
    }
    import_source(
        &unsafe { CStr::from_ptr(id) }.to_string_lossy(),
        &unsafe { CStr::from_ptr(name) }.to_string_lossy(),
    );
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
unsafe extern "C" fn nabi_mobile_source_utf16(
    id: *const u16,
    id_len: usize,
    name: *const u16,
    name_len: usize,
) {
    if id.is_null() || name.is_null() {
        return;
    }
    import_source(
        &String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(id, id_len) }),
        &String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(name, name_len) }),
    );
}

#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_settings_import(fd: i32) {
    let result = (|| -> Result<PathBuf, String> {
        if fd < 0 {
            return Err("The selected document could not be opened.".to_owned());
        }
        // The host transfers an owned descriptor. Close it on every path.
        let source = unsafe { File::from_raw_fd(fd) };
        let path = PATHS
            .get()
            .filter(|p| p.cache.is_absolute())
            .ok_or("App cache is unavailable")?
            .cache
            .join("imports");
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        let path = path.join("settings-import.json");
        let mut output = File::create(&path).map_err(|e| e.to_string())?;
        std::io::copy(&mut source.take(SETTINGS_FILE_LIMIT), &mut output)
            .map_err(|e| e.to_string())?;
        Ok(path)
    })();
    *SETTINGS.lock().unwrap() = Some(result);
    repaint();
}

#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_export_result(saved: bool) {
    if !saved {
        EXPORT_FAILED.store(true, Ordering::Relaxed);
        repaint();
    }
}

fn import_fd(fd: i32, name: &str) -> bool {
    if fd < 0 {
        import_error("The selected document could not be opened.".into());
        return false;
    }
    // The native picker transfers an owned descriptor. Close it on every path.
    let source = unsafe { File::from_raw_fd(fd) };
    let modified = source
        .metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok());
    let mut pending = IMPORT.lock().unwrap();
    let Some(Ok(batch)) = pending.as_mut() else {
        return false;
    };
    match batch.add_with_modified(name, source, modified) {
        Ok(more) => more,
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
    if let Some(mut result) = IMPORT.lock().unwrap().take() {
        if result
            .as_ref()
            .is_ok_and(|batch| batch.is_folder() && batch.first().is_none())
        {
            result = Err("No supported images were found in the folder.".into());
        }
        if let Ok(batch) = &mut result {
            if batch.first().is_some() {
                if let Err(error) = batch.commit_source() {
                    result = Err(error);
                }
            }
        }
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
