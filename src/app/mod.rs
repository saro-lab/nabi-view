pub(crate) mod licenses;

#[cfg(target_os = "macos")]
use crate::platform::mac_open;
#[cfg(any(target_os = "ios", target_os = "android"))]
use crate::platform::mobile;
use crate::{
    storage,
    ui::{mobile as mobile_ui, viewer},
};
use eframe::egui;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
use std::path::Path;
use std::path::PathBuf;

/// Touch UI for native phones/tablets or the resizable desktop mobile preview.
pub(crate) fn is_mobile() -> bool {
    crate::core::NATIVE_MOBILE || mobile_ui::preview_enabled()
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn bundled_file(relative: &Path) -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    let resources = directory.join("../Resources").join(relative);
    [resources, directory.join(relative)]
        .into_iter()
        .find(|p| p.is_file())
}

pub(crate) fn sample_path() -> Option<PathBuf> {
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        return mobile::sample_path();
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let relative = Path::new("field-notes/01 Alpine Light.png");
        bundled_file(relative).or_else(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(relative);
            path.is_file().then_some(path)
        })
    }
}

pub(crate) fn notices_path() -> Option<PathBuf> {
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        return mobile::notices_path();
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let name = Path::new("THIRD_PARTY_NOTICES.txt");
        bundled_file(name).or_else(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/notices")
                .join(name);
            path.is_file().then_some(path)
        })
    }
}

#[cfg(target_os = "macos")]
pub(crate) enum OpenRequest {
    Here(PathBuf),
    New(Option<PathBuf>),
}

// Each root window owns one process, so closing it never closes a sibling.
// Launch Services must be asked explicitly for a fresh instance on macOS.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub(crate) fn launch_viewer(path: Option<&Path>) -> Result<(), String> {
    use std::process::Command;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    if let Some(bundle) = exe.ancestors().nth(3).filter(|p| {
        p.extension().is_some_and(|ext| ext == "app") && p.join("Contents/Info.plist").is_file()
    }) {
        return mac_open::launch_window(bundle, path);
    }
    let mut child = Command::new(exe)
        .arg("--new-window")
        .args(path)
        .spawn()
        .map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn run_desktop() -> eframe::Result {
    let mut args = std::env::args_os()
        .skip(1)
        .filter(|arg| !arg.to_string_lossy().starts_with("-psn_"))
        .peekable();
    let _new_window = args.next_if(|arg| arg == "--new-window").is_some();
    let preview = args.next_if(|arg| arg == "--mobile-preview").is_some();
    mobile_ui::set_preview(preview);
    let mut paths = args
        .map(PathBuf::from)
        .map(|p| std::path::absolute(&p).unwrap_or(p));
    let input = paths
        .next()
        .or_else(|| if preview { sample_path() } else { None });
    #[cfg(target_os = "macos")]
    mac_open::install(_new_window || input.is_some());
    let store = if preview {
        dirs::cache_dir()
            .ok_or_else(|| "App cache is unavailable".to_owned())
            .map(|root| {
                let root = root.join("NabiView/mobile-preview");
                storage::Store::in_directories(root.join("config"), root.join("state"))
            })
    } else {
        storage::Store::new()
    };
    let window_size = if preview {
        [390., 844.]
    } else {
        store
            .as_ref()
            .ok()
            .and_then(|store| store.load_window_size().ok().flatten())
            .unwrap_or(crate::core::settings::DEFAULT_WINDOW_SIZE)
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/icons/png/nabi-icon-256.png"
                )))
                .expect("bundled application icon"),
            )
            .with_app_id("dev.nabi.NabiView")
            .with_decorations(preview)
            .with_inner_size(window_size)
            .with_min_inner_size(if preview {
                [320., 480.]
            } else {
                crate::core::settings::MIN_WINDOW_SIZE
            })
            .with_clamp_size_to_monitor_size(true)
            .with_resizable(true)
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "NABI VIEW",
        options,
        Box::new(move |cc| {
            #[cfg(target_os = "macos")]
            mac_open::set_context(cc.egui_ctx.clone());
            #[cfg(target_os = "macos")]
            let input = mac_open::take_initial().or(input);
            let mut viewer = viewer::Viewer::new(cc, input, store);
            for extra in paths {
                viewer.new_window(Some(&extra));
            }
            crate::platform::fonts::install(cc.egui_ctx.clone());
            Ok(Box::new(viewer))
        }),
    )
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn run_mobile(options: eframe::NativeOptions) -> eframe::Result {
    let store = mobile::store();
    eframe::run_native(
        "NABI VIEW",
        options,
        Box::new(move |cc| {
            mobile::set_context(cc.egui_ctx.clone());
            let viewer = viewer::Viewer::new(cc, None, store);
            crate::platform::fonts::install(cc.egui_ctx.clone());
            Ok(Box::new(viewer))
        }),
    )
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn mobile_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_decorations(false),
        renderer: eframe::Renderer::Wgpu,
        run_and_return: false,
        ..Default::default()
    }
}

#[cfg(target_os = "ios")]
pub fn run_ios() -> eframe::Result {
    mobile::init_ios();
    run_mobile(mobile_options())
}

#[cfg(target_os = "android")]
pub fn run_android(app: android_activity::AndroidApp) -> eframe::Result {
    mobile::init_android(&app);
    let mut options = mobile_options();
    options.android_app = Some(app);
    run_mobile(options)
}
