use crate::app::OpenRequest;
use eframe::egui;
use std::{
    collections::VecDeque,
    ffi::{CStr, OsStr, c_char},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

unsafe extern "C" {
    fn nabi_install_open_documents();
    #[cfg(not(test))]
    fn nabi_set_fullscreen(enabled: bool, blackout: bool) -> bool;
    fn nabi_launch_window(bundle: *const c_char, path: *const c_char) -> bool;
    fn nabi_configure_menu();
    #[cfg(not(test))]
    fn nabi_set_shortcut_capture(active: bool);
}

pub fn launch_window(
    bundle: &std::path::Path,
    path: Option<&std::path::Path>,
) -> Result<(), String> {
    let bundle =
        std::ffi::CString::new(bundle.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let path = path
        .map(|p| std::ffi::CString::new(p.as_os_str().as_bytes()))
        .transpose()
        .map_err(|e| e.to_string())?;
    if unsafe {
        nabi_launch_window(
            bundle.as_ptr(),
            path.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
        )
    } {
        Ok(())
    } else {
        Err("Could not open a new viewer window.".into())
    }
}

static CLOSE_ALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[unsafe(no_mangle)]
extern "C" fn nabi_request_close_all() {
    CLOSE_ALL.store(true, std::sync::atomic::Ordering::Relaxed);
    wake();
}
pub fn take_close_all() -> bool {
    CLOSE_ALL.swap(false, std::sync::atomic::Ordering::Relaxed)
}

static ABOUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[unsafe(no_mangle)]
extern "C" fn nabi_request_about() {
    ABOUT.store(true, std::sync::atomic::Ordering::Relaxed);
    wake();
}
pub fn take_about() -> bool {
    ABOUT.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// AppKit delivers documents used to launch the app before didFinishLaunching.
/// An already-requested CLI window owns its startup content even in that interval.
enum Phase {
    Launching { first_window_available: bool },
    Running,
    Terminating,
}

struct Requests {
    phase: Phase,
    pending: VecDeque<OpenRequest>,
}

impl Requests {
    const fn starting(explicit_start: bool) -> Self {
        Self {
            phase: Phase::Launching {
                first_window_available: !explicit_start,
            },
            pending: VecDeque::new(),
        }
    }

    fn open(&mut self, path: PathBuf) {
        match &mut self.phase {
            Phase::Launching {
                first_window_available,
            } if *first_window_available => {
                *first_window_available = false;
                self.pending.push_back(OpenRequest::Here(path));
            }
            Phase::Launching { .. } | Phase::Running => {
                self.pending.push_back(OpenRequest::New(Some(path)));
            }
            Phase::Terminating => {}
        }
    }

    fn reopen(&mut self) {
        if matches!(self.phase, Phase::Running) {
            self.pending.push_back(OpenRequest::New(None));
        }
    }

    fn finish_launch(&mut self) {
        if matches!(self.phase, Phase::Launching { .. }) {
            self.phase = Phase::Running;
        }
    }

    fn terminate(&mut self) {
        self.phase = Phase::Terminating;
        self.pending.clear();
    }

    fn take_initial(&mut self) -> Option<PathBuf> {
        if matches!(self.pending.front(), Some(OpenRequest::Here(_)))
            && let Some(OpenRequest::Here(path)) = self.pending.pop_front()
        {
            return Some(path);
        }
        None
    }
}

static REQUESTS: Mutex<Requests> = Mutex::new(Requests::starting(false));
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

pub fn install(explicit_start: bool) {
    *REQUESTS.lock().expect("initialize open events") = Requests::starting(explicit_start);
    unsafe { nabi_install_open_documents() };
}

pub fn set_context(ctx: egui::Context) {
    let _ = CONTEXT.set(ctx);
    // winit installs its default menu during didFinishLaunching, after our
    // notification observer may have run. CreationContext is after that step.
    unsafe { nabi_configure_menu() };
}

#[cfg(not(test))]
pub fn set_shortcut_capture(active: bool) {
    unsafe { nabi_set_shortcut_capture(active) };
}

pub fn take_initial() -> Option<PathBuf> {
    REQUESTS.lock().ok()?.take_initial()
}

pub fn take() -> Option<OpenRequest> {
    let mut requests = REQUESTS.lock().ok()?;
    let request = requests.pending.pop_front();
    let more = !requests.pending.is_empty();
    drop(requests);
    if more {
        wake();
    }
    request
}

fn wake() {
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

#[unsafe(no_mangle)]
extern "C" fn nabi_finished_launching() {
    if let Ok(mut requests) = REQUESTS.lock() {
        requests.finish_launch();
    }
}

#[unsafe(no_mangle)]
extern "C" fn nabi_stop_open_events() {
    if let Ok(mut requests) = REQUESTS.lock() {
        requests.terminate();
    }
}

#[unsafe(no_mangle)]
extern "C" fn nabi_reopen_application() {
    if let Ok(mut requests) = REQUESTS.lock() {
        requests.reopen();
    }
    wake();
}

// Copy NSURL.fileSystemRepresentation while the Apple event still owns the URL.
#[unsafe(no_mangle)]
unsafe extern "C" fn nabi_enqueue_open_document(path: *const c_char) {
    if path.is_null() {
        return;
    }
    let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
    if let Ok(mut requests) = REQUESTS.lock() {
        requests.open(PathBuf::from(OsStr::from_bytes(bytes)));
    }
    wake();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn finder_cold_multiple_files_claim_one_initial_window_then_spawn() {
        let mut requests = Requests::starting(false);
        requests.open("one.jpg".into());
        requests.open("two.jpg".into());
        requests.finish_launch();
        requests.open("three.jpg".into());
        requests.reopen();
        assert_eq!(
            requests.take_initial().as_deref(),
            Some(Path::new("one.jpg"))
        );
        assert_eq!(requests.take_initial(), None);
        for expected in ["two.jpg", "three.jpg"] {
            assert!(
                matches!(requests.pending.pop_front(),Some(OpenRequest::New(Some(path))) if path == Path::new(expected))
            );
        }
        assert!(matches!(
            requests.pending.pop_front(),
            Some(OpenRequest::New(None))
        ));
        assert!(requests.pending.is_empty());
    }

    #[test]
    fn explicit_launch_and_quit_do_not_create_duplicate_windows() {
        let mut requests = Requests::starting(true);
        requests.reopen(); // startup isn't a Dock re-open
        requests.open("another.png".into());
        assert_eq!(requests.take_initial(), None);
        assert_eq!(requests.pending.len(), 1);
        assert!(matches!(
            requests.pending.front(),
            Some(OpenRequest::New(Some(_)))
        ));
        requests.finish_launch();
        requests.terminate();
        requests.finish_launch();
        requests.reopen();
        requests.open("late.jpg".into());
        assert!(requests.pending.is_empty());
    }
}

/// Called only from the native UI thread. Returns the actual fullscreen state.
#[cfg(not(test))]
pub fn fullscreen(enabled: bool, blackout: bool) -> bool {
    unsafe { nabi_set_fullscreen(enabled, blackout) }
}

#[cfg(not(test))]
pub fn localize_menu() {
    thread_local! { static PREVIOUS: std::cell::RefCell<Option<crate::i18n::Language>> = const { std::cell::RefCell::new(None) }; }
    let language = crate::i18n::current();
    let changed = PREVIOUS.with(|previous| previous.borrow().as_ref() != Some(&language));
    if !changed {
        return;
    }
    let titles: Option<Vec<_>> = [
        "about-application",
        "hide-application",
        "hide-other-applications",
        "show-all-applications",
        "quit-application",
        "services",
    ]
    .into_iter()
    .map(|id| std::ffi::CString::new(crate::i18n::format(id, &[("application", "NABI VIEW")])).ok())
    .collect();
    let Some(titles) = titles else {
        return;
    };
    unsafe extern "C" {
        fn nabi_localize_menu(
            about: *const c_char,
            hide: *const c_char,
            others: *const c_char,
            show: *const c_char,
            quit: *const c_char,
            services: *const c_char,
        );
    }
    unsafe {
        nabi_localize_menu(
            titles[0].as_ptr(),
            titles[1].as_ptr(),
            titles[2].as_ptr(),
            titles[3].as_ptr(),
            titles[4].as_ptr(),
            titles[5].as_ptr(),
        );
    }
    PREVIOUS.with(|previous| *previous.borrow_mut() = Some(language));
}
