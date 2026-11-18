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
    #[cfg(target_os = "ios")]
    fn nabi_mobile_reopen(id: *const u16, length: i32) -> bool;
    #[cfg(target_os = "android")]
    fn nabi_android_reopen_cached(id: *const u16, length: i32, token: u64) -> bool;
    #[cfg(target_os = "android")]
    fn nabi_android_source_epoch() -> u64;
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
static IMPORT_PENDING: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "ios")]
static IOS_SEED: Mutex<Option<(ImportBatch, u64)>> = Mutex::new(None);
#[cfg(target_os = "ios")]
static IOS_SEED_STARTED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "ios")]
static IOS_SEED_CANCELLED: AtomicBool = AtomicBool::new(false);
static SETTINGS: Mutex<Option<Result<PathBuf, String>>> = Mutex::new(None);
static EXPORT_FAILED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "ios")]
static SHARE_ERROR: Mutex<Option<String>> = Mutex::new(None);
static REOPEN: Mutex<Option<PathBuf>> = Mutex::new(None);
#[cfg(target_os = "android")]
static ANDROID_REUSED: Mutex<Option<(PathBuf, u64, u64)>> = Mutex::new(None);
#[cfg(target_os = "android")]
static ANDROID_OPEN: super::android_open::OpenGate = super::android_open::OpenGate::new();
#[cfg(target_os = "ios")]
static IOS_IMPORT: Mutex<crate::core::ios_access::ImportGate> =
    Mutex::new(crate::core::ios_access::ImportGate::new());
#[cfg(target_os = "ios")]
static IOS_ACCESS_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(target_os = "ios")]
static IOS_IMPORT_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(target_os = "ios")]
static IOS_COMPLETION: Mutex<
    Option<(
        Option<PathBuf>,
        Option<crate::core::ios_access::AccessIssue>,
        u64,
    )>,
> = Mutex::new(None);
#[cfg(target_os = "ios")]
static IOS_ACCESS_CHANGED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "ios")]
static IOS_ACCESS_RESUMED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "ios")]
static IOS_PHOTO_VALIDATION_PENDING: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "ios")]
static IOS_PHOTO_VALIDATION_SERIAL: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
#[cfg(target_os = "ios")]
static IOS_PHOTO_VALIDATION_REQUEST: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
#[cfg(target_os = "ios")]
static IOS_PHOTO_VALIDATION_RESULT: Mutex<Option<(u64, bool)>> = Mutex::new(None);
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
        #[cfg(target_os = "ios")]
        if let Ok(entries) = std::fs::read_dir(paths.cache.join("shares")) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("share-")
                    && entry.file_type().is_ok_and(|kind| kind.is_file())
                {
                    let _ = std::fs::remove_file(entry.path());
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
    ANDROID_OPEN.cancel(|| IMPORT_PENDING.store(false, Ordering::Release));
    CONTEXT.lock().unwrap().take();
    IMPORT.lock().unwrap().take();
    RESULT.lock().unwrap().take();
    SETTINGS.lock().unwrap().take();
    REOPEN.lock().unwrap().take();
    ANDROID_REUSED.lock().unwrap().take();
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
extern "C" fn nabi_mobile_is_rtl() -> bool {
    crate::i18n::native_is_rtl()
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
    #[cfg(target_os = "android")]
    {
        let request = ANDROID_OPEN.request()?;
        return ANDROID_OPEN.publish(request, take_import_result).flatten();
    }
    #[cfg(not(target_os = "android"))]
    take_import_result()
}

fn take_import_result() -> Option<Result<ImportBatch, String>> {
    let mut slot = RESULT.lock().unwrap();
    let result = slot.take();
    if result.is_some() {
        IMPORT_PENDING.store(false, Ordering::Release);
    }
    result
}

pub fn import_pending() -> bool {
    IMPORT_PENDING.load(Ordering::Acquire)
}

/// Called when a selection has been confirmed, before any provider I/O.
#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_import_pending() {
    #[cfg(target_os = "android")]
    let accepted = ANDROID_OPEN.request().is_some_and(|request| {
        ANDROID_OPEN
            .publish(request, || IMPORT_PENDING.store(true, Ordering::Release))
            .is_some()
    });
    #[cfg(target_os = "ios")]
    let accepted = {
        let mut gate = IOS_IMPORT.lock().unwrap();
        if !gate.busy() {
            start_ios_import(&mut gate, None);
        }
        let current = gate.current();
        if current {
            IMPORT_PENDING.store(true, Ordering::Release);
        }
        current
    };
    if accepted {
        repaint();
    }
}

/// A successful Java reservation is the only action that permits another open
/// after home. Never acquire IMPORT here: a provider may be blocking in read().
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
extern "C" fn nabi_android_request_started() {
    let abandoned = ANDROID_OPEN.start(|| {
        IMPORT_PENDING.store(false, Ordering::Release);
        ANDROID_REUSED.lock().unwrap().take();
        RESULT.lock().unwrap().take()
    });
    discard_android_result(abandoned);
}

#[cfg(target_os = "android")]
pub fn cancel_android_open() {
    let abandoned = ANDROID_OPEN.cancel(|| {
        IMPORT_PENDING.store(false, Ordering::Release);
        REOPEN.lock().unwrap().take();
        ANDROID_REUSED.lock().unwrap().take();
        RESULT.lock().unwrap().take()
    });
    discard_android_result(abandoned);
}

#[cfg(target_os = "android")]
fn discard_android_result(abandoned: Option<Result<ImportBatch, String>>) {
    // A completed folder owns many temporary copies. Returning home must not
    // synchronously delete them on the viewer/Java UI thread either.
    if let Some(Ok(batch)) = abandoned {
        std::thread::spawn(move || {
            super::worker_priority::background();
            drop(batch);
        });
    }
}

#[cfg(target_os = "ios")]
pub fn take_seed() -> Option<ImportBatch> {
    let gate = IOS_IMPORT.lock().unwrap();
    let (seed, revision) = IOS_SEED.lock().unwrap().take()?;
    (gate.current()
        && revision == access_revision()
        && seed.ios_access_current(revision, photo_status()))
    .then_some(seed)
}

#[cfg(target_os = "ios")]
pub fn take_seed_cancelled() -> bool {
    IOS_SEED_CANCELLED.swap(false, Ordering::AcqRel)
}

#[cfg(target_os = "ios")]
fn cancel_seed() {
    IOS_SEED.lock().unwrap().take();
    if IOS_SEED_STARTED.swap(false, Ordering::AcqRel) {
        IOS_SEED_CANCELLED.store(true, Ordering::Release);
    }
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
            if !start_access_import(Some(path.to_owned())) {
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
    let mut gate = IOS_IMPORT.lock().unwrap();
    gate.cancel();
    IMPORT_PENDING.store(false, Ordering::Release);
    cancel_seed();
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

/// Only PhotoKit content/selection changes invalidate the content revision.
/// A retained selection may be reused only while its starting revision matches.
#[cfg(target_os = "ios")]
pub fn access_revision() -> u64 {
    IOS_ACCESS_REVISION.load(Ordering::Acquire)
}

#[cfg(target_os = "ios")]
fn start_access_import(source: Option<PathBuf>) -> bool {
    let mut gate = IOS_IMPORT.lock().unwrap();
    start_ios_import(&mut gate, source)
}

#[cfg(target_os = "ios")]
fn start_ios_import(
    gate: &mut crate::core::ios_access::ImportGate,
    source: Option<PathBuf>,
) -> bool {
    if !gate.start(source) {
        return false;
    }
    // A completed event from A must not be consumed after request B starts.
    // finish publishes under this same gate, so clearing is ordered with it.
    RESULT.lock().unwrap().take();
    IOS_COMPLETION.lock().unwrap().take();
    cancel_seed();
    IOS_IMPORT_REVISION.store(access_revision(), Ordering::Release);
    true
}

#[cfg(target_os = "ios")]
fn finish_access_import(
    gate: &mut crate::core::ios_access::ImportGate,
) -> Option<(
    Option<PathBuf>,
    Option<crate::core::ios_access::AccessIssue>,
    u64,
)> {
    // The gate remains locked until this revision is copied into the result.
    // Never stamp completion with the newest revision: a change may have
    // happened while the provider was enumerating the collection.
    gate.finish()
        .map(|(source, issue)| (source, issue, IOS_IMPORT_REVISION.load(Ordering::Acquire)))
}

#[cfg(target_os = "ios")]
pub fn take_access_completion() -> Option<(
    Option<PathBuf>,
    Option<crate::core::ios_access::AccessIssue>,
    u64,
)> {
    IOS_COMPLETION.lock().unwrap().take()
}

#[cfg(target_os = "ios")]
pub fn take_access_changed() -> bool {
    IOS_ACCESS_CHANGED.swap(false, Ordering::Relaxed)
}

#[cfg(target_os = "ios")]
pub fn take_access_resumed() -> bool {
    IOS_ACCESS_RESUMED.swap(false, Ordering::AcqRel)
}

#[cfg(target_os = "ios")]
pub fn photo_validation_pending() -> bool {
    IOS_PHOTO_VALIDATION_PENDING.load(Ordering::Acquire)
        || IOS_ACCESS_RESUMED.load(Ordering::Acquire)
}

#[cfg(target_os = "ios")]
pub fn cancel_photo_validation() {
    let mut result = IOS_PHOTO_VALIDATION_RESULT.lock().unwrap();
    IOS_PHOTO_VALIDATION_REQUEST.store(0, Ordering::Release);
    result.take();
    IOS_PHOTO_VALIDATION_PENDING.store(false, Ordering::Release);
}

#[cfg(target_os = "ios")]
pub fn validate_photo_source(path: &Path) -> Option<u64> {
    let (_, id) = crate::core::documents::source_identity(path)?;
    if !crate::core::photo_library::is_ios(&id) {
        return None;
    }
    let source = std::ffi::CString::new(id).ok()?;
    let request = IOS_PHOTO_VALIDATION_SERIAL.fetch_add(1, Ordering::Relaxed) + 1;
    {
        let mut result = IOS_PHOTO_VALIDATION_RESULT.lock().unwrap();
        result.take();
        IOS_PHOTO_VALIDATION_REQUEST.store(request, Ordering::Release);
        IOS_PHOTO_VALIDATION_PENDING.store(true, Ordering::Release);
    }
    unsafe extern "C" {
        fn nabi_photos_validate(source: *const c_char, request: u64);
    }
    unsafe { nabi_photos_validate(source.as_ptr(), request) };
    Some(request)
}

#[cfg(target_os = "ios")]
pub fn take_photo_validation() -> Option<(u64, bool)> {
    IOS_PHOTO_VALIDATION_RESULT.lock().unwrap().take()
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_access_resumed() {
    IOS_PHOTO_VALIDATION_PENDING.store(true, Ordering::Release);
    IOS_ACCESS_RESUMED.store(true, Ordering::Release);
    repaint();
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_photo_validation_result(request: u64, same: bool) {
    let mut result = IOS_PHOTO_VALIDATION_RESULT.lock().unwrap();
    if request != IOS_PHOTO_VALIDATION_REQUEST.load(Ordering::Acquire) {
        return;
    }
    *result = Some((request, same));
    if !same {
        // The observer may lag behind the fresh metadata query. Invalidate
        // source/cache guards as soon as this request establishes a change.
        nabi_mobile_access_changed();
    }
    drop(result);
    repaint();
}

#[cfg(target_os = "ios")]
pub fn take_access_actions() -> Vec<(i32, bool)> {
    std::mem::take(&mut *IOS_ACCESS_ACTIONS.lock().unwrap())
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_access_changed() {
    IOS_ACCESS_REVISION.fetch_add(1, Ordering::AcqRel);
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
#[cfg(target_os = "ios")]
pub fn reopen(path: &Path) -> bool {
    reopen_impl(path, 0)
}

#[cfg(target_os = "android")]
pub fn reopen_android(path: &Path, token: u64) -> bool {
    reopen_impl(path, token)
}

#[cfg(target_os = "android")]
pub fn android_source_epoch() -> u64 {
    // Only lifecycle/observer epoch and current full permission are checked here.
    // MediaStore generation queries run on the native import worker.
    unsafe { nabi_android_source_epoch() }
}

#[cfg(target_os = "android")]
pub fn take_android_reused() -> Option<(PathBuf, u64, u64)> {
    let request = ANDROID_OPEN.request()?;
    ANDROID_OPEN
        .publish(request, || {
            let result = ANDROID_REUSED.lock().unwrap().take();
            if result.is_some() {
                IMPORT_PENDING.store(false, Ordering::Release);
            }
            result
        })
        .flatten()
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
extern "C" fn nabi_android_source_snapshot(token: u64) {
    if ANDROID_OPEN.importing().is_none() {
        return;
    }
    if let Some(Ok(batch)) = IMPORT.lock().unwrap().as_mut() {
        batch.set_android_reuse_token(token);
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
extern "C" fn nabi_android_source_reused(token: u64, epoch: u64) -> bool {
    if token == 0 || epoch == 0 {
        return false;
    }
    let Some(request) = ANDROID_OPEN.request() else {
        return false;
    };
    let published = ANDROID_OPEN
        .publish(request, || {
            let mut pending = REOPEN.lock().unwrap();
            let Some(path) = pending.take() else {
                return false;
            };
            *ANDROID_REUSED.lock().unwrap() = Some((path, token, epoch));
            true
        })
        .unwrap_or(false);
    if published {
        repaint();
    }
    published
}

fn reopen_impl(path: &Path, token: u64) -> bool {
    #[cfg(target_os = "ios")]
    let _ = token;
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
    if !start_access_import(Some(path.to_owned())) {
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
        // A new request supersedes a completed reuse that UI has not polled yet.
        // Publish/clear this event while holding the same REOPEN gate.
        #[cfg(target_os = "android")]
        ANDROID_REUSED.lock().unwrap().take();
        *pending = Some(path.to_owned());
    }
    let id: Vec<u16> = record.id.encode_utf16().collect();
    #[cfg(target_os = "ios")]
    let accepted = unsafe { nabi_mobile_reopen(id.as_ptr(), id.len() as i32) };
    #[cfg(target_os = "android")]
    let accepted = unsafe { nabi_android_reopen_cached(id.as_ptr(), id.len() as i32, token) };
    if !accepted {
        #[cfg(target_os = "ios")]
        {
            let completion = finish_access_import(&mut IOS_IMPORT.lock().unwrap());
            *IOS_COMPLETION.lock().unwrap() = completion;
        }
        REOPEN.lock().unwrap().take();
        let error = Err("Another document operation is still in progress.".into());
        #[cfg(target_os = "ios")]
        {
            *RESULT.lock().unwrap() = Some(error);
        }
        #[cfg(target_os = "android")]
        if let Some(request) = ANDROID_OPEN.request() {
            ANDROID_OPEN.publish(request, || *RESULT.lock().unwrap() = Some(error));
        }
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
    #[cfg(target_os = "ios")]
    if let Some(source) = crate::core::documents::retain_deferred(path) {
        let path = path.to_owned();
        std::thread::spawn(move || {
            let prepare = || -> Result<PathBuf, String> {
                source.materialize(&path, &|| true)?;
                let directory = PATHS
                    .get()
                    .ok_or("App cache is unavailable")?
                    .cache
                    .join("shares");
                std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
                let extension = path.extension().unwrap_or_default().to_string_lossy();
                let shared = tempfile::Builder::new()
                    .prefix("share-")
                    .suffix(&format!(".{extension}"))
                    .tempfile_in(directory)
                    .map_err(|error| error.to_string())?;
                std::fs::copy(&path, shared.path()).map_err(|error| error.to_string())?;
                let (_, path) = shared.keep().map_err(|error| error.to_string())?;
                Ok(path)
            };
            match prepare() {
                Ok(path) => with_utf16(&path, |text, length| unsafe {
                    unsafe extern "C" {
                        fn nabi_mobile_share_owned(path: *const u16, length: i32);
                    }
                    nabi_mobile_share_owned(text, length)
                }),
                Err(error) => {
                    *SHARE_ERROR.lock().unwrap() = Some(error);
                    repaint();
                }
            }
        });
        return;
    }
    // A listed-but-unread source is an empty placeholder until loaded.
    #[cfg(target_os = "android")]
    if let Some(source) = crate::core::documents::retain_deferred(path) {
        let path = path.to_owned();
        std::thread::spawn(move || {
            if source.materialize(&path, &|| true).is_ok() {
                with_utf16(&path, |text, length| unsafe {
                    nabi_mobile_share(text, length)
                });
            }
        });
        return;
    }
    with_utf16(path, |text, length| unsafe {
        nabi_mobile_share(text, length)
    });
}

#[cfg(target_os = "ios")]
pub fn take_share_error() -> Option<String> {
    SHARE_ERROR.lock().unwrap().take()
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
    #[cfg(target_os = "android")]
    let Some(request) = ANDROID_OPEN.begin() else {
        return false;
    };
    nabi_mobile_import_pending();
    #[cfg(target_os = "ios")]
    {
        let gate = IOS_IMPORT.lock().unwrap();
        if !gate.current() {
            return false;
        }
        IOS_SEED_STARTED.store(false, Ordering::Release);
        IOS_SEED.lock().unwrap().take();
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
        #[cfg(target_os = "ios")]
        batch.set_ios_access_stamp(IOS_IMPORT_REVISION.load(Ordering::Acquire), photo_status());
    }
    let ok = batch.is_ok();
    #[cfg(target_os = "android")]
    if !ANDROID_OPEN.current(request) {
        return false;
    }
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

/// Returns the NUL-inclusive length of the requested concrete page name. A
/// source-root request has no name; the native provider chooses its first page.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
unsafe extern "C" fn nabi_mobile_import_requested_name(
    output: *mut c_char,
    capacity: usize,
) -> usize {
    let pending = IMPORT.lock().unwrap();
    let Some(name) = pending
        .as_ref()
        .and_then(|batch| batch.as_ref().ok())
        .and_then(ImportBatch::requested_name)
        .and_then(|name| name.to_str())
    else {
        return 0;
    };
    let length = name.len() + 1;
    if !output.is_null() && capacity >= length {
        unsafe {
            std::ptr::copy_nonoverlapping(name.as_ptr(), output.cast(), name.len());
            output.add(name.len()).write(0);
        }
    }
    length
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
extern "C" fn nabi_mobile_import_seed() -> bool {
    let gate = IOS_IMPORT.lock().unwrap();
    let revision = IOS_IMPORT_REVISION.load(Ordering::Acquire);
    if !gate.current() || revision != access_revision() || !photo_status().readable() {
        return false;
    }
    if IOS_SEED_STARTED.load(Ordering::Acquire) {
        return true;
    }
    let pending = IMPORT.lock().unwrap();
    if let Some(seed) = pending
        .as_ref()
        .and_then(|batch| batch.as_ref().ok())
        .and_then(ImportBatch::snapshot_seed)
    {
        *IOS_SEED.lock().unwrap() = Some((seed, revision));
        IOS_SEED_STARTED.store(true, Ordering::Release);
        repaint();
    }
    true
}

fn import_source(id: &str, name: &str) {
    #[cfg(target_os = "android")]
    if ANDROID_OPEN.importing().is_none() {
        return;
    }
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
    #[cfg(target_os = "android")]
    let Some(request) = ANDROID_OPEN.importing() else {
        return false;
    };
    #[cfg(target_os = "ios")]
    if !IOS_IMPORT.lock().unwrap().current() {
        return false;
    }
    let modified = source
        .metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok());
    #[cfg(target_os = "android")]
    let source = ANDROID_OPEN.reader(request, source);
    let mut pending = IMPORT.lock().unwrap();
    let Some(Ok(batch)) = pending.as_mut() else {
        return false;
    };
    match batch.add_with_modified(name, source, modified) {
        Ok(more) => {
            #[cfg(target_os = "android")]
            {
                more && ANDROID_OPEN.current(request)
            }
            #[cfg(not(target_os = "android"))]
            {
                more
            }
        }
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

/// Adds a native folder's metadata without opening or copying its original.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
unsafe extern "C" fn nabi_mobile_import_deferred(
    name: *const c_char,
    token: *const c_char,
    bytes: u64,
    modified: f64,
    resource_type: i32,
    content_modified: f64,
) -> bool {
    use std::{
        ffi::{CString, c_void},
        os::fd::AsRawFd,
        sync::Arc,
        time::{Duration, SystemTime},
    };
    unsafe extern "C" {
        fn nabi_mobile_read_source(
            source: *const c_char,
            token: *const c_char,
            fd: i32,
            limit: u64,
            live: unsafe extern "C" fn(*mut c_void) -> bool,
            context: *mut c_void,
        ) -> i32;
    }
    unsafe extern "C" fn still_live(context: *mut c_void) -> bool {
        let live = unsafe { &*(context.cast::<&dyn Fn() -> bool>()) };
        live()
    }
    if name.is_null() || token.is_null() || !IOS_IMPORT.lock().unwrap().current() {
        return false;
    }
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    let token = unsafe { CStr::from_ptr(token) }.to_owned();
    let mut pending = IMPORT.lock().unwrap();
    let Some(Ok(batch)) = pending.as_mut() else {
        return false;
    };
    let Some(source) = batch
        .source()
        .and_then(|source| CString::new(source.id.clone()).ok())
    else {
        return false;
    };
    let timestamp = |value: f64| {
        (value.is_finite() && value > 0.0 && value < 1.0e12)
            .then(|| SystemTime::UNIX_EPOCH + Duration::from_secs_f64(value))
    };
    let modified = timestamp(modified);
    let revision = IOS_IMPORT_REVISION.load(Ordering::Acquire);
    let provider_cache = super::ios_photo_cache::from_authorized_import(
        source.to_str().unwrap_or_default(),
        &token,
        &name,
        resource_type,
        timestamp(content_modified),
        revision,
    )
    .or_else(|| {
        // Files-app folders: a stable key, so reopening hits the disk cache.
        super::document_cache::from_document_folder(
            source.to_str().unwrap_or_default(),
            &name,
            bytes,
            modified,
        )
    });
    match batch.add_deferred_with_cache(
        &name,
        bytes,
        modified,
        provider_cache,
        Arc::new(move |output, limit, live| {
            let mut live = live;
            let result = unsafe {
                nabi_mobile_read_source(
                    source.as_ptr(),
                    token.as_ptr(),
                    output.as_raw_fd(),
                    limit,
                    still_live,
                    (&mut live as *mut &dyn Fn() -> bool).cast(),
                )
            };
            match result {
                0 => Ok(()),
                -1 => Err("Cancelled".into()),
                reason => Err(crate::i18n::tr(
                    crate::core::ios_access::AccessIssue::from_native(reason).message(),
                )
                .into()),
            }
        }),
    ) {
        Ok(more) => more,
        Err(error) => {
            *pending = Some(Err(error));
            false
        }
    }
}

/// Adds a MediaStore or SAF item by metadata only. Its pixels are read through
/// the host when a decoder first needs them; the token stays in memory.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
unsafe extern "C" fn nabi_mobile_import_deferred_utf16(
    name: *const u16,
    name_length: usize,
    token: *const u16,
    token_length: usize,
    bytes: u64,
    modified_millis: i64,
    kind: i32,
) -> bool {
    use super::android_media;
    if name.is_null() || token.is_null() {
        return false;
    }
    let Some(request) = ANDROID_OPEN.importing() else {
        return false;
    };
    let name = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(name, name_length) });
    let token =
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(token, token_length) });
    if !matches!(
        kind,
        android_media::MEDIA_STORE | android_media::DOCUMENT | android_media::MEDIA_STORE_PARTIAL
    ) || token.is_empty()
    {
        return false;
    }
    let cache = android_media::provider_cache(kind, &token, modified_millis, bytes);
    let loader = android_media::loader(kind, token);
    let mut pending = IMPORT.lock().unwrap();
    let Some(Ok(batch)) = pending.as_mut() else {
        return false;
    };
    match batch.add_deferred_with_cache(
        &name,
        bytes,
        android_media::modified_time(modified_millis),
        Some(cache),
        loader,
    ) {
        Ok(more) => more && ANDROID_OPEN.current(request),
        Err(error) => {
            *pending = Some(Err(error));
            false
        }
    }
}

/// Full photo access was lost; earlier MediaStore identities stop matching.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
extern "C" fn nabi_android_media_access_changed() {
    super::android_media::access_changed();
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
    #[cfg(target_os = "android")]
    if ANDROID_OPEN.importing().is_none() {
        return;
    }
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
    #[cfg(target_os = "android")]
    if ANDROID_OPEN.importing().is_none() {
        return;
    }
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
    #[cfg(target_os = "android")]
    let request = ANDROID_OPEN.importing();
    let mut result = IMPORT.lock().unwrap().take();
    #[cfg(target_os = "android")]
    let Some(request) = request else {
        // Dispose the cancelled copies on their import worker, not home's UI.
        return;
    };
    if result.as_ref().is_some_and(|result| {
        result
            .as_ref()
            .is_ok_and(|batch| batch.is_folder() && batch.first().is_none())
    }) {
        result = Some(Err("No supported images were found in the folder.".into()));
    }
    // Keep the request busy, but never hold its UI-facing lock during JSON
    // encoding or fsync. Cancellation remains responsive and cannot start a
    // competing source writer until this native request has drained.
    let prepared = match result.as_mut() {
        Some(Ok(batch)) if batch.first().is_some() => match batch.prepare_source_commit() {
            Ok(prepared) => prepared,
            Err(error) => {
                result = Some(Err(error));
                None
            }
        },
        _ => None,
    };
    #[cfg(target_os = "android")]
    {
        ANDROID_OPEN.publish(request, || {
            if let Some(Ok(batch)) = &result {
                if let Err(error) = batch.commit_prepared_source(prepared) {
                    result = Some(Err(error));
                }
            }
            if let Some(result) = result.filter(|result| {
                result
                    .as_ref()
                    .map_or(true, |batch| batch.first().is_some())
            }) {
                *RESULT.lock().unwrap() = Some(result);
            } else if RESULT.lock().unwrap().is_none() {
                IMPORT_PENDING.store(false, Ordering::Release);
            }
        });
        repaint();
        return;
    }
    #[cfg(target_os = "ios")]
    let mut gate = IOS_IMPORT.lock().unwrap();
    #[cfg(target_os = "ios")]
    {
        let photo = gate
            .source()
            .is_some_and(|path| crate::core::photo_library::title(&path).is_some());
        if photo
            && (IOS_IMPORT_REVISION.load(Ordering::Acquire) != access_revision()
                || !photo_status().readable())
        {
            gate.cancel();
        }
        if !gate.current() {
            gate.finish();
            IMPORT_PENDING.store(false, Ordering::Release);
            cancel_seed();
            repaint();
            return;
        }
    }
    // Only the atomic publication runs under the request gate. A cancelled
    // request drops its prepared marker instead of replacing saved metadata.
    #[cfg(target_os = "ios")]
    if let Some(Ok(batch)) = &result {
        if let Err(error) = batch.commit_prepared_source(prepared) {
            result = Some(Err(error));
        }
    }
    #[cfg(target_os = "ios")]
    {
        *IOS_COMPLETION.lock().unwrap() = finish_access_import(&mut gate);
        if result.as_ref().is_some_and(Result::is_err) {
            cancel_seed();
        } else {
            IOS_SEED.lock().unwrap().take();
        }
    }
    // A cancelled/empty picker leaves the current selection untouched.
    #[cfg(target_os = "ios")]
    if let Some(result) = result.filter(|result| {
        result
            .as_ref()
            .map_or(true, |batch| batch.first().is_some())
    }) {
        *RESULT.lock().unwrap() = Some(result);
    } else {
        IMPORT_PENDING.store(false, Ordering::Release);
    }
    #[cfg(target_os = "ios")]
    repaint();
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
