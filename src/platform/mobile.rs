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
static CONTEXT: Mutex<Option<egui::Context>> = Mutex::new(None);
#[cfg(target_os = "android")]
static LOCALE_CHANGED: AtomicBool = AtomicBool::new(false);
static IMPORT: Mutex<Option<Result<ImportBatch, String>>> = Mutex::new(None);
static RESULT: Mutex<Option<Result<ImportBatch, String>>> = Mutex::new(None);
static SETTINGS: Mutex<Option<Result<PathBuf, String>>> = Mutex::new(None);
static EXPORT_FAILED: AtomicBool = AtomicBool::new(false);
static REOPEN: Mutex<Option<PathBuf>> = Mutex::new(None);
#[cfg(target_os = "ios")]
static IOS_IMPORT: Mutex<crate::core::ios_access::ImportGate> =
    Mutex::new(crate::core::ios_access::ImportGate::new());
#[cfg(target_os = "ios")]
static IOS_COMPLETION: Mutex<
    Option<(
        Option<PathBuf>,
        Option<crate::core::ios_access::AccessIssue>,
    )>,
> = Mutex::new(None);
#[cfg(target_os = "ios")]
static IOS_ACCESS_CHANGED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "ios")]
static IOS_ACCESS_ACTIONS: Mutex<Vec<(i32, bool)>> = Mutex::new(Vec::new());
static DISPLAY: Mutex<super::mobile_display::DisplayState> =
    Mutex::new(super::mobile_display::DisplayState::new());

fn init_paths() {
    if PATHS.get().is_some() {
        return;
    }
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

/// Called after the event loop has saved and released the viewer. The native
/// gate drains active callbacks and rejects callbacks from the old Activity.
#[cfg(target_os = "android")]
pub fn shutdown_android() {
    unsafe extern "C" {
        fn nabi_android_shutdown();
    }
    unsafe {
        nabi_android_shutdown();
    }
    CONTEXT.lock().unwrap().take();
    IMPORT.lock().unwrap().take();
    RESULT.lock().unwrap().take();
    SETTINGS.lock().unwrap().take();
    REOPEN.lock().unwrap().take();
    *DISPLAY.lock().unwrap() = super::mobile_display::DisplayState::new();
    BACK.store(false, Ordering::Relaxed);
    LOCALE_CHANGED.store(false, Ordering::Relaxed);
    EXPORT_FAILED.store(false, Ordering::Relaxed);
    crate::platform::locale::invalidate_cache();
}

pub fn set_context(ctx: egui::Context) {
    *CONTEXT.lock().unwrap() = Some(ctx);
}
fn repaint() {
    let ctx = CONTEXT.lock().unwrap().clone();
    if let Some(ctx) = ctx {
        ctx.request_repaint();
    }
}

/// Native hosts report the OS window mode, independently of immersive UI.
/// -1 means the host has not attached a window yet.
#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_window_mode_changed(state: i32) {
    let windowed = match state {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };
    let changed = DISPLAY.lock().unwrap().set_windowed(windowed);
    if changed {
        repaint();
    }
}

pub fn windowed() -> Option<bool> {
    DISPLAY.lock().unwrap().windowed()
}

/// Native dialogs use the same active language as the Rust UI, including on
/// Android versions without per-app LocaleManager support.
#[unsafe(no_mangle)]
unsafe extern "C" fn nabi_mobile_text(
    key: *const c_char,
    output: *mut u16,
    capacity: usize,
) -> usize {
    if key.is_null() {
        return 0;
    }
    let key = unsafe { CStr::from_ptr(key) }.to_string_lossy();
    let text: Vec<u16> = crate::i18n::native_tr(&key).encode_utf16().collect();
    if !output.is_null() && capacity >= text.len() {
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), output, text.len());
        }
    }
    text.len()
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_locale_changed() {
    LOCALE_CHANGED.store(true, Ordering::Relaxed);
    repaint();
}

#[cfg(target_os = "android")]
pub fn take_locale_changed() -> bool {
    LOCALE_CHANGED.swap(false, Ordering::Relaxed)
}

pub fn store() -> Result<Store, String> {
    let paths = PATHS.get().ok_or("Mobile host is not initialized")?;
    if !paths.data.is_absolute() || !paths.cache.is_absolute() {
        return Err("App storage is unavailable".into());
    }
    let store = Store::in_directories_with_cache(
        paths.data.join("config"),
        paths.data.join("state"),
        paths.cache.clone(),
    );
    #[cfg(target_os = "android")]
    let defaults = crate::core::photo_library::ANDROID;
    #[cfg(target_os = "ios")]
    let defaults = crate::core::photo_library::IOS_DEFAULTS;
    store.initialize_photo_bookmarks(&paths.data.join("document-sources"), defaults)?;
    Ok(store)
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

#[cfg(target_os = "ios")]
pub fn photo_status() -> crate::core::ios_access::PhotoStatus {
    unsafe extern "C" {
        fn nabi_mobile_photo_status() -> i32;
    }
    crate::core::ios_access::PhotoStatus::from_native(unsafe { nabi_mobile_photo_status() })
}

/// Recovery may rebase an old container path only if it is already a saved
/// library entry. Native code must still validate/reacquire the actual grant.
#[cfg(target_os = "ios")]
pub fn recovery_source(path: &Path) -> Option<PathBuf> {
    let (root, id) = crate::core::documents::source_identity(path)?;
    let sources = PATHS.get()?.data.join("document-sources");
    let suffix = path.strip_prefix(&root).ok()?;
    if !suffix
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        return None;
    }
    if root.parent() != Some(sources.as_path())
        && !store()
            .ok()?
            .library()
            .ok()?
            .iter()
            .any(|entry| entry.folder == root)
    {
        return None;
    }
    Some(sources.join(id).join(suffix))
}

#[cfg(target_os = "ios")]
pub fn access_action(action: i32, source: Option<&Path>) -> bool {
    unsafe extern "C" {
        fn nabi_mobile_open_settings() -> bool;
        fn nabi_mobile_manage_photos() -> bool;
        fn nabi_mobile_reselect(id: *const u16, length: i32) -> bool;
        fn nabi_mobile_request_photo_access() -> bool;
    }
    match action {
        1 => unsafe { nabi_mobile_open_settings() },
        2 => unsafe { nabi_mobile_manage_photos() },
        4 => unsafe { nabi_mobile_request_photo_access() },
        3 => {
            let Some(path) = source else {
                return false;
            };
            let Some((root, id)) = crate::core::documents::source_identity(path) else {
                return false;
            };
            if PATHS.get().is_none_or(|paths| {
                root.parent() != Some(paths.data.join("document-sources").as_path())
            }) {
                return false;
            }
            if !IOS_IMPORT.lock().unwrap().start(Some(path.to_owned())) {
                return false;
            }
            *REOPEN.lock().unwrap() = Some(path.to_owned());
            let text: Vec<u16> = id.encode_utf16().collect();
            let accepted = unsafe { nabi_mobile_reselect(text.as_ptr(), text.len() as i32) };
            if !accepted {
                IOS_IMPORT.lock().unwrap().finish();
                REOPEN.lock().unwrap().take();
            }
            accepted
        }
        _ => false,
    }
}

#[cfg(target_os = "ios")]
pub fn cancel_access_import() {
    IOS_IMPORT.lock().unwrap().cancel();
    RESULT.lock().unwrap().take();
    IOS_COMPLETION.lock().unwrap().take();
    REOPEN.lock().unwrap().take();
}

#[cfg(target_os = "ios")]
pub fn access_import_busy() -> bool {
    IOS_IMPORT.lock().unwrap().busy()
}
#[cfg(target_os = "ios")]
pub fn access_import_source() -> Option<PathBuf> {
    IOS_IMPORT.lock().unwrap().source()
}

#[cfg(target_os = "ios")]
pub fn take_access_completion() -> Option<(
    Option<PathBuf>,
    Option<crate::core::ios_access::AccessIssue>,
)> {
    IOS_COMPLETION.lock().unwrap().take()
}

#[cfg(target_os = "ios")]
pub fn take_access_changed() -> bool {
    IOS_ACCESS_CHANGED.swap(false, Ordering::Relaxed)
}

#[cfg(target_os = "ios")]
pub fn take_access_actions() -> Vec<(i32, bool)> {
    std::mem::take(&mut *IOS_ACCESS_ACTIONS.lock().unwrap())
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_access_changed() {
    IOS_ACCESS_CHANGED.store(true, Ordering::Relaxed);
    repaint();
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_access_action_result(action: i32, success: bool) {
    if action == 3 && !success {
        IOS_IMPORT.lock().unwrap().finish();
        REOPEN.lock().unwrap().take();
    }
    IOS_ACCESS_ACTIONS.lock().unwrap().push((action, success));
    repaint();
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
    #[cfg(target_os = "ios")]
    if !IOS_IMPORT.lock().unwrap().start(Some(path.to_owned())) {
        return true;
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
        #[cfg(target_os = "ios")]
        {
            *IOS_COMPLETION.lock().unwrap() = IOS_IMPORT.lock().unwrap().finish();
        }
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
    // UIKit may call back synchronously. Never hold DISPLAY across native code.
    let (orientation, immersive) = DISPLAY.lock().unwrap().requests(orientation, immersive);
    unsafe {
        if let Some(mode) = orientation {
            nabi_mobile_set_orientation(mode);
        }
        if let Some(immersive) = immersive {
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
    #[cfg(target_os = "ios")]
    {
        let mut gate = IOS_IMPORT.lock().unwrap();
        if !gate.busy() {
            gate.start(None);
        }
        if !gate.current() {
            return false;
        }
    }
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
        } else {
            #[cfg(target_os = "ios")]
            if let Some(source) = batch.source() {
                IOS_IMPORT.lock().unwrap().identify(source.root.clone());
            }
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
    #[cfg(target_os = "ios")]
    if !IOS_IMPORT.lock().unwrap().current() {
        return false;
    }
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
extern "C" fn nabi_mobile_photo_error(reason: i32) {
    #[cfg(target_os = "ios")]
    {
        let issue = crate::core::ios_access::AccessIssue::from_native(reason);
        IOS_IMPORT.lock().unwrap().issue = Some(issue);
        import_error(issue.message().into());
    }
    #[cfg(not(target_os = "ios"))]
    import_error(
        match reason {
            1 => "photo-access-denied",
            2 => "photo-collection-empty",
            _ => "folder-unavailable",
        }
        .into(),
    );
}

#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_import_truncated() {
    if let Some(Ok(batch)) = IMPORT.lock().unwrap().as_mut() {
        batch.mark_truncated();
    }
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
    #[cfg(target_os = "ios")]
    let mut gate = IOS_IMPORT.lock().unwrap();
    #[cfg(target_os = "ios")]
    {
        if !gate.current() {
            gate.finish();
            IMPORT.lock().unwrap().take();
            repaint();
            return;
        }
        *IOS_COMPLETION.lock().unwrap() = gate.finish();
    }
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
