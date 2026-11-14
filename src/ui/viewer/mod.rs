mod actions;
mod animation;
mod archive;
#[cfg(test)]
mod cache_memory_tests;
mod canvas;
mod checked_files;
mod chrome;
mod chrome_icons;
mod details;
mod forms;
mod image_pipeline;
mod input;
mod library;
mod menu;
mod metadata;
mod mobile_chrome;
#[cfg(any(target_os = "ios", target_os = "android", test))]
mod mobile_sources;
mod move_files;
mod navigation;
mod overlay;
mod rename_files;
mod settings;
mod sorting;
mod toasts;
mod window_frame;

use crate::core::commands::*;
use crate::core::library::Kind;
use crate::core::media::{self, Media, MediaEvent};
use crate::core::settings::{self as limits, KeyboardShortcut, Settings};
use crate::i18n::{Language, tr};
use crate::storage::Store;
use chrome_icons::HoverText;
use eframe::egui::{self, Color32, FontId, Key, Rect, RichText, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const BAR: Color32 = Color32::from_rgb(21, 23, 25);
const INK: Color32 = Color32::from_rgb(236, 237, 231);
const MUTED: Color32 = Color32::from_rgb(159, 165, 161);
const ACCENT: Color32 = Color32::from_rgb(204, 232, 165);
/// Notices that need attention without being failures.
const WARNING: Color32 = Color32::from_rgb(244, 198, 145);
/// Below this window width the bars use smaller buttons and tighter spacing.
const NARROW_WIDTH: f32 = 700.;
/// Smallest comfortable touch target on a phone.
const TOUCH_TARGET: f32 = 44.;
/// Manual zoom limits, in percent of the image's own pixels.
const ZOOM_PERCENT: std::ops::RangeInclusive<f32> = 20. ..=3000.;
/// One zoom-in or zoom-out step.
const ZOOM_STEP: f32 = 1.25;
/// Longest side decoded for a page; larger images show a reduced preview.
const MAX_DECODE_SIDE: u32 = 4096;
/// Longest side decoded for the thumbnail strip.
const THUMBNAIL_DECODE_SIDE: u32 = 192;
/// Page steps typed while a folder is still loading; later ones are dropped.
const MAX_PENDING_STEPS: usize = i16::MAX as usize;
const TOP_BAR_HEIGHT: f32 = 47.;
const STATUS_BAR_HEIGHT: f32 = 45.;
const THUMBNAIL_BAR_HEIGHT: f32 = 78.;
const THUMBNAIL_WIDTH: f32 = 65.;
const THUMBNAIL_MARGIN: f32 = 12.;
const BOTTOM_CHROME_HEIGHT: f32 = STATUS_BAR_HEIGHT + THUMBNAIL_BAR_HEIGHT;

fn command_supported(id: &str) -> bool {
    if mobile_only(id) {
        return crate::app::is_mobile();
    }
    (!crate::app::is_mobile() || crate::ui::mobile::supported(id))
        && (id != "reveal" || crate::platform::reveal_file::supported())
}

/// Native phones open private copies, so "trash" only removes them from the list.
const REMOVES_FROM_LIST: bool = crate::core::NATIVE_MOBILE;

fn command_label(id: &str) -> &'static str {
    match id {
        "trash" if REMOVES_FROM_LIST => "remove-from-list",
        "undo-trash" if REMOVES_FROM_LIST => "restore-to-list",
        _ => COMMANDS
            .iter()
            .find(|command| command.0 == id)
            .map_or("", |command| command.1),
    }
}

fn orientation_command(mode: &str) -> &'static str {
    match mode {
        "portrait" => "orientation-portrait",
        "landscape" => "orientation-landscape",
        _ => "orientation-auto",
    }
}

struct Picture {
    hdr: Option<std::sync::Arc<crate::ui::hdr::Texture>>,
    animation: Option<media::animation::Animation>,
    handle: egui::TextureHandle,
    original: [u32; 2],
    requested: u32,
    last_used: u64,
}
type Ticket = (u64, PathBuf, u32);

// Keep texture ownership, not just texture IDs: cache eviction must not free
// the last successfully painted frame while another image is being decoded.
struct PresentedImage {
    hdr: Option<std::sync::Arc<crate::ui::hdr::Texture>>,
    handle: egui::TextureHandle,
    rect: Rect,
    clip: Rect,
    backdrop: Rect,
}

#[derive(Default)]
struct Presentation {
    images: Vec<PresentedImage>,
    canvas: Option<Rect>,
    pan: Vec2,
}

impl Presentation {
    fn paint(
        &mut self,
        ui: &egui::Ui,
        canvas: Rect,
        image_uv: Rect,
        background: Color32,
        checker: bool,
    ) {
        let Some(previous) = self.canvas else { return };
        let transform = egui::emath::RectTransform::from_to(previous, canvas);
        let scale = (canvas.width() / previous.width()).min(canvas.height() / previous.height());
        if self.pan != Vec2::ZERO
            && self.images.iter().all(|image| {
                let slot = transform.transform_rect(image.clip).size();
                let shown = image.backdrop.size() * scale;
                shown.x <= slot.x + 0.001 && shown.y <= slot.y + 0.001
            })
        {
            // Loading the next page must not leave a now-fitted retained image
            // off-center after a resize. Discard the offset permanently.
            for image in &mut self.images {
                image.rect = image.rect.translate(-self.pan);
                image.backdrop = image.backdrop.translate(-self.pan);
            }
            self.pan = Vec2::ZERO;
        }
        for image in &self.images {
            let clip = if previous == canvas {
                image.clip
            } else {
                transform.transform_rect(image.clip)
            };
            let rect = if previous == canvas {
                image.rect
            } else {
                Rect::from_center_size(
                    canvas.center() + (image.rect.center() - previous.center()) * scale,
                    image.rect.size() * scale,
                )
            };
            let painter = ui.painter().with_clip_rect(clip);
            painter.rect_filled(rect, 0., background);
            let backdrop = if previous == canvas {
                image.backdrop
            } else {
                Rect::from_center_size(
                    canvas.center() + (image.backdrop.center() - previous.center()) * scale,
                    image.backdrop.size() * scale,
                )
            };
            if checker {
                crate::ui::transparency::paint(
                    &painter.with_clip_rect(clip.intersect(rect)),
                    backdrop,
                    12.,
                );
            }
            painter.image(
                image
                    .hdr
                    .as_ref()
                    .filter(|hdr| hdr.active())
                    .map_or_else(|| image.handle.id(), |hdr| hdr.id()),
                rect,
                image_uv,
                Color32::WHITE,
            );
        }
    }
}

struct Toast {
    message: String,
    deadline: std::time::Instant,
    image: Option<PathBuf>,
    keep_on_slideshow: bool,
}

#[derive(Clone, Copy)]
enum CanvasDrag {
    Window,
    Image {
        pointer_origin: egui::Pos2,
        pan_origin: Vec2,
    },
}

pub struct Viewer {
    cursor_idle: crate::ui::cursor_idle::CursorIdle,
    display_sleep: crate::platform::display_sleep::DisplaySleep,
    toast: Option<Toast>,
    media: Media,
    details: details::Details,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    inspector: metadata::Inspector,
    store: Option<Store>,
    settings: Settings,
    disk_cache_config: Option<(PathBuf, u32, bool)>,
    files: Vec<PathBuf>,
    sorting: sorting::Sorting,
    #[cfg(all(target_os = "macos", not(test)))]
    mac_access: Option<crate::platform::mac_access::Access>,
    // Current page anchoring the focused spread. This is navigation state,
    // independent of widget/keyboard focus and checked selection.
    focus_index: usize,
    // Window-local pairing alignment selected by one-page navigation.
    spread_shifted: bool,
    checked: HashSet<PathBuf>,
    check_navigation: Option<checked_files::CheckNavigation>,
    random_prompt: bool,
    trash_history: Vec<Vec<crate::platform::trash::TrashEntry>>,
    file_job: Option<checked_files::FileJob>,
    close_after_file_job: bool,
    closing_all: bool,
    close_request_accepted: bool,
    close_signal: Option<crate::platform::close_signal::CloseSignal>,
    folder: Option<PathBuf>,
    listing: Option<u64>,
    folder_watch: Option<u64>,
    tickets: [Option<Ticket>; 2],
    prefetch_ticket: Option<Ticket>,
    prefetch_blocked: HashSet<PathBuf>,
    prefetch_settings: (usize, u32, u32),
    memory_monitor: crate::core::cache::Monitor,
    memory_policy: crate::core::cache::Policy,
    pictures: HashMap<PathBuf, Picture>,
    // Dimensions survive texture eviction, so page grouping remains stable.
    page_sizes: HashMap<PathBuf, [u32; 2]>,
    // Group pages using the monitor, independently of window size and chrome.
    layout_monitor: Vec2,
    // Two-page spreads are offered only when the display is wider than tall.
    landscape: bool,
    span_cache: std::cell::RefCell<Option<SpanCache>>,
    presentation: Presentation,
    transition_waiting: bool,
    pending_steps: Vec<(bool, NavigationStep)>,
    thumb_ticket: Option<(u64, PathBuf)>,
    thumbs: HashMap<PathBuf, egui::TextureHandle>,
    app_icon: egui::TextureHandle,
    thumb_errors: HashSet<PathBuf>,
    failures: HashMap<PathBuf, String>,
    last_error: String,
    frame: u64,
    /// Recent folders, bookmarks and their overlay, dialogs and thumbnails.
    library: library::Library,
    /// The zip or 7z archive open in this window, shown like a folder.
    archives: archive::Archives,
    recent_moves: Vec<PathBuf>,
    move_dialog: Option<move_files::MoveDialog>,
    rename_dialog: Option<rename_files::RenameDialog>,
    zoom: f32,
    zoom_override: Option<f32>,
    zoom_input: Option<String>,
    zoom_input_focus: bool,
    pan: Vec2,
    fit_limit: u32,
    settings_open: bool,
    menu_open: bool,
    settings_section: usize,
    associations: crate::platform::associations::Associations,
    tab_down: bool,
    tab_presses: Vec<(usize, egui::Event)>,
    popup_open_at_input: bool,
    shortcut_editor: Option<settings::ShortcutEditor>,
    shortcut_search: Option<String>,
    shortcut_filter: String,
    shortcut_filter_focused: bool,
    shortcut_reset_prompt: bool,
    reset_prompt: Option<settings::ResetScope>,
    reveal_job: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    notices: Option<Vec<String>>,
    show_full_notices: bool,
    hide_controls: bool,
    controls_visible: bool,
    hover_until: f64,
    held_key: Option<(Key, &'static str, f64)>,
    held_button: Option<(bool, f64)>,
    canvas_drag: Option<CanvasDrag>,
    touch_gesture: crate::ui::mobile::TouchGesture,
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    mobile_selection: Option<crate::core::documents::ImportBatch>,
    window_size_state: window_frame::WindowSizeState,
    fullscreen: bool,
    random: bool,
    slideshow: bool,
    slide_deadline: Option<f64>,
}

impl Viewer {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        initial: Option<PathBuf>,
        store: Result<Store, String>,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        // All quit keys belong to the editable app bindings, including capture dialogs.
        ctx.options_mut(|options| options.quit_shortcuts.clear());
        let mut style = (*ctx.global_style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BAR;
        style.visuals.window_fill = Color32::from_rgb(28, 30, 31);
        style.visuals.selection.bg_fill = Color32::from_rgb(64, 82, 55);
        style.visuals.widgets.active.fg_stroke.color = ACCENT;
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(9.0, 5.0);
        if crate::app::is_mobile() {
            style.spacing.interact_size = Vec2::splat(TOUCH_TARGET);
        }
        ctx.set_global_style(style);
        let icon = eframe::icon_data::from_png_bytes(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/png/nabi-icon-256.png"
        )))
        .expect("bundled application icon");
        let app_icon = ctx.load_texture(
            "nabi-app-icon",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        let mut app = Self {
            cursor_idle: Default::default(),
            display_sleep: Default::default(),
            app_icon,
            toast: None,
            media: Media::new(ctx.clone()),
            details: details::Details::new(ctx.clone()),
            render_state: cc.wgpu_render_state.clone(),
            inspector: metadata::Inspector::default(),
            store: None,
            settings: Settings::default(),
            disk_cache_config: None,
            files: Vec::new(),
            sorting: sorting::Sorting::default(),
            #[cfg(all(target_os = "macos", not(test)))]
            mac_access: None,
            focus_index: 0,
            spread_shifted: false,
            checked: HashSet::new(),
            check_navigation: None,
            random_prompt: false,
            trash_history: Vec::new(),
            file_job: None,
            close_after_file_job: false,
            closing_all: false,
            close_request_accepted: false,
            close_signal: store
                .as_ref()
                .ok()
                .filter(|_| !crate::app::is_mobile())
                .map(|store| {
                    crate::platform::close_signal::CloseSignal::new(
                        store.close_signal_path(),
                        ctx.clone(),
                    )
                }),
            folder: None,
            listing: None,
            folder_watch: None,
            tickets: [None, None],
            prefetch_ticket: None,
            prefetch_blocked: HashSet::new(),
            prefetch_settings: (0, 0, 0),
            memory_monitor: crate::core::cache::Monitor::new(ctx.clone()),
            memory_policy: crate::core::cache::Policy::default(),
            pictures: HashMap::new(),
            page_sizes: HashMap::new(),
            layout_monitor: Vec2::ZERO,
            landscape: true,
            span_cache: Default::default(),
            presentation: Presentation::default(),
            transition_waiting: false,
            pending_steps: Vec::new(),
            thumb_ticket: None,
            thumbs: HashMap::new(),
            thumb_errors: HashSet::new(),
            failures: HashMap::new(),
            last_error: String::new(),
            frame: 0,
            library: Default::default(),
            archives: archive::Archives::new(ctx.clone()),
            recent_moves: Vec::new(),
            move_dialog: None,
            rename_dialog: None,
            zoom: 1.0,
            zoom_override: None,
            zoom_input: None,
            zoom_input_focus: false,
            pan: Vec2::ZERO,
            fit_limit: 2048,
            settings_open: false,
            menu_open: false,
            settings_section: 0,
            associations: Default::default(),
            tab_down: false,
            tab_presses: Vec::new(),
            popup_open_at_input: false,
            shortcut_editor: None,
            shortcut_search: None,
            shortcut_filter: String::new(),
            shortcut_filter_focused: false,
            shortcut_reset_prompt: false,
            reset_prompt: None,
            reveal_job: None,
            notices: None,
            show_full_notices: false,
            hide_controls: !crate::app::is_mobile(),
            controls_visible: false,
            hover_until: 0.,
            held_key: None,
            held_button: None,
            canvas_drag: None,
            touch_gesture: Default::default(),
            #[cfg(any(target_os = "ios", target_os = "android", test))]
            mobile_selection: None,
            window_size_state: window_frame::WindowSizeState::default(),
            fullscreen: false,
            random: false,
            slideshow: false,
            slide_deadline: None,
        };
        match store {
            Ok(store) => {
                match store.load().and_then(|s| {
                    validate_keys(&s)?;
                    Ok(s)
                }) {
                    Ok(s) => app.settings = s,
                    Err(e) => app.last_error = e,
                }
                app.store = Some(store);
            }
            Err(e) => app.last_error = e,
        }
        #[cfg(target_os = "android")]
        {
            let language = crate::platform::locale::restore_preference(&app.settings.language);
            if language != app.settings.language {
                app.settings.language = language;
                app.save();
            }
        }
        app.sync_disk_cache();
        #[cfg(all(target_os = "macos", not(test)))]
        if let Some(store) = &app.store {
            if let Err(error) = crate::platform::mac_access::configure(store.state_directory()) {
                app.last_error = error;
            }
        }
        if let Some(path) = initial {
            let error = std::mem::take(&mut app.last_error);
            app.open(path);
            if app.last_error.is_empty() {
                app.last_error = error;
            }
        }
        crate::i18n::set_language(app.settings.language.clone());
        if app.files.is_empty() && app.listing.is_none() {
            app.library.open = true;
            app.reload_library();
        }
        app
    }

    fn sync_disk_cache(&mut self) {
        let cache_config = self.store.as_ref().map(|store| {
            (
                store.cache_dir().to_owned(),
                self.settings.disk_cache_mb,
                self.settings.disk_cache_enabled,
            )
        });
        if cache_config != self.disk_cache_config {
            self.media.configure_cache(
                cache_config
                    .as_ref()
                    .filter(|(_, _, enabled)| *enabled)
                    .map(|(root, limit, _)| {
                        crate::core::disk_cache::Cache::new(
                            root.clone(),
                            u64::from(*limit) * 1024 * 1024,
                        )
                    }),
            );
            self.disk_cache_config = cache_config;
        }
    }

    fn save(&mut self) {
        if let Some(store) = &self.store
            && let Err(e) = store.save(&self.settings)
        {
            self.last_error = e;
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        self.menu_window(ctx);
        self.random_confirmation(ctx);
        self.settings_window(ctx);
        self.poll_library_thumbnails(ctx);
        self.library_window(ctx);
        self.move_window(ctx);
        self.rename_window(ctx);
        self.bookmark_window(ctx);
        self.archive_window(ctx);
        self.clear_recent_window(ctx);
    }

    /// Stops key and button auto-repeat, e.g. when a dialog takes the input.
    fn release_held_input(&mut self) {
        self.held_key = None;
        self.held_button = None;
    }

    /// Leaves shortcut editing, search and filtering in the settings screen.
    fn close_shortcut_editing(&mut self) {
        self.shortcut_editor = None;
        self.shortcut_search = None;
        self.shortcut_filter.clear();
        self.shortcut_filter_focused = false;
        self.shortcut_reset_prompt = false;
    }

    /// A menu, list or dialog above the canvas that takes its input.
    fn overlay_open(&self) -> bool {
        self.menu_open || self.library.modal() || self.dialog_open() || self.random_prompt
    }

    /// The phone's back gesture closes the topmost layer. Returns false when
    /// nothing is open, so the platform host can leave the app.
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    fn handle_back(&mut self, ctx: &egui::Context) -> bool {
        if ctx.any_popup_open() {
            egui::Popup::close_all(ctx);
        } else if self.shortcut_reset_prompt || self.reset_prompt.is_some() {
            self.shortcut_reset_prompt = false;
            self.reset_prompt = None;
        } else if self.shortcut_editor.take().is_some() || self.zoom_input.take().is_some() {
            // Taking the editor or the zoom field closed it.
        } else if self.random_prompt {
            self.random_prompt = false;
        } else if self.library.clear_prompt {
            self.library.clear_prompt = false;
        } else if self.library.dialog.is_some() {
            self.library.dialog = None;
        } else if self.archives.prompt.is_some() {
            self.leave_archive();
        } else if self.rename_dialog.is_some() {
            if self.file_job.is_none() {
                self.rename_dialog = None;
            }
        } else if self.move_dialog.is_some() {
            // A running move keeps its dialog; the gesture is still used up.
            if self.file_job.is_none() {
                self.move_dialog = None;
            }
        } else if self.menu_open {
            self.menu_open = false;
        } else if self.settings_open {
            self.settings_open = false;
            self.close_shortcut_editing();
        } else if self.library.open && (!self.files.is_empty() || self.listing.is_some()) {
            // Without images the list is the start screen; nothing is behind it.
            self.library.open = false;
        } else if self.inspector.open {
            self.inspector.close();
        } else if self.fullscreen {
            self.fullscreen = false;
        } else {
            return false;
        }
        // A closed dialog must not keep the on-screen keyboard up.
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        self.release_held_input();
        true
    }
}

impl eframe::App for Viewer {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // eframe 0.37 skips ui for minimized/occluded windows. Shutdown and
        // file-operation completion must still run there, without painting.
        self.close_request_accepted = false;
        #[cfg(target_os = "macos")]
        if crate::platform::mac_open::take_close_all() && !self.shortcut_input_active() {
            self.run("close-all", ctx);
        }
        if self
            .close_signal
            .as_mut()
            .is_some_and(|signal| signal.poll())
        {
            self.request_graceful_close(ctx);
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.shortcut_input_active() && !self.closing_all {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if self.file_job.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_after_file_job = true;
                self.show_toast(
                    ctx,
                    tr("the-window-will-close-when-the-file-operation-finishes"),
                );
            } else {
                if let Err(error) = self.window_size_state.flush(self.store.as_ref()) {
                    self.last_error = error;
                }
                self.close_request_accepted = true;
            }
        }
        self.poll_file_job(ctx);
    }

    fn on_exit(&mut self) {
        // The folder reopens at the page this window showed last.
        self.remember_page();
        // Nothing reads the extracted pages any more.
        self.archives.close();
        if let Err(error) = self.window_size_state.flush(self.store.as_ref()) {
            eprintln!("Could not save window size: {error}");
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0., 0., 0., 1.]
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.prepare_pointer_input(ctx, input);
        #[cfg(any(target_os = "ios", target_os = "android"))]
        prepare_mobile_safe_area(ctx, input, crate::platform::mobile::safe_area());
        self.sync_native_shortcut_capture();
        #[cfg(all(target_os = "macos", not(test)))]
        if let Some(viewport) = input.viewports.get_mut(&input.viewport_id) {
            viewport.fullscreen = Some(self.fullscreen);
        }
        self.popup_open_at_input = self.menu_open || ctx.any_popup_open();
        let capturing = self.shortcut_editor.as_ref().is_some_and(|e| e.listening)
            || self
                .shortcut_search
                .as_ref()
                .is_some_and(|search| search.is_empty());
        if self.zoom_input.is_some()
            || (!capturing
                && (self.settings_open
                    || self.library.modal()
                    || self.dialog_open()
                    || self.random_prompt
                    || self.popup_open_at_input))
        {
            self.tab_down = false;
            self.tab_presses.clear();
            return;
        }
        // egui schedules focus traversal before App::update. Keep Tab in the
        // app shortcut stream only, including when a toolbar button has focus.
        let mut index = 0;
        input.events.retain(|event| {
            match event {
                egui::Event::Key {
                    key: Key::Tab,
                    pressed,
                    repeat,
                    ..
                } => {
                    if *pressed && !self.tab_down && !repeat {
                        self.tab_presses.push((index, event.clone()));
                        index += 1;
                    }
                    self.tab_down = *pressed;
                    return false;
                }
                egui::Event::WindowFocused(false) => self.tab_down = false,
                _ => {}
            }
            index += 1;
            true
        });
        if !input.focused {
            self.tab_down = false;
        }
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        if self.close_request_accepted {
            return;
        }
        if let Some(state) = &self.render_state {
            crate::ui::hdr::configure_display(state);
            if state.target_format == eframe::wgpu::TextureFormat::Rgba16Float {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
        }
        self.sync_sort_settings();
        if !crate::app::is_mobile() {
            self.cursor_idle.observe(ctx);
        }
        // Widgets may replace this with resize/grab/text cursors below.
        ctx.set_cursor_icon(egui::CursorIcon::Default);
        self.sync_disk_cache();
        let animation_time = if self.settings.reduce_motion {
            0.0
        } else {
            egui::Style::default().animation_time
        };
        if ctx.global_style().animation_time != animation_time {
            ctx.global_style_mut(|style| style.animation_time = animation_time);
        }
        crate::i18n::set_language(self.settings.language.clone());
        crate::platform::fonts::sync_language(ctx);
        #[cfg(all(target_os = "macos", not(test)))]
        crate::platform::mac_open::localize_menu();
        #[cfg(all(target_os = "macos", not(test)))]
        if self.fullscreen {
            self.fullscreen =
                crate::platform::mac_open::fullscreen(true, self.settings.blackout_other_monitors);
        }
        self.frame += 1;
        #[cfg(any(target_os = "ios", target_os = "android"))]
        if crate::platform::mobile::take_back() && !self.handle_back(ctx) {
            #[cfg(target_os = "android")]
            crate::platform::mobile::exit();
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        if let Some(result) = crate::platform::mobile::take_result() {
            match result {
                Ok(selection) => {
                    self.accept_mobile_selection(selection);
                }
                Err(error) => self.last_error = error,
            }
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        {
            match crate::platform::mobile::take_settings_import() {
                Some(Ok(path)) => self.import_settings_from(&path),
                Some(Err(error)) => self.last_error = error,
                None => {}
            }
            if crate::platform::mobile::take_export_failed() {
                self.last_error = "Could not save the settings file.".into();
            }
            crate::platform::mobile::sync_display(&self.settings.orientation, self.fullscreen);
        }
        #[cfg(target_os = "macos")]
        if crate::platform::mac_open::take_about() {
            self.run("about", ctx);
        }
        if !crate::app::is_mobile()
            && let Err(error) = self.window_size_state.observe(ctx, self.store.as_ref())
        {
            self.last_error = error;
        }
        #[cfg(target_os = "macos")]
        if let Some(request) = crate::platform::mac_open::take() {
            match request {
                crate::app::OpenRequest::Here(path) => self.open(path),
                crate::app::OpenRequest::New(path) => {
                    #[cfg(not(test))]
                    if let Some(path) = path
                        .as_ref()
                        .filter(|path| crate::platform::mac_access::temporary(path))
                    {
                        self.open(path.clone());
                    } else {
                        self.new_window(path.as_deref());
                    }
                    #[cfg(test)]
                    self.new_window(path.as_deref());
                }
            }
        }
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        for path in dropped {
            #[cfg(all(target_os = "macos", not(test)))]
            let permission_error = crate::platform::mac_access::remember(&path).err();
            #[cfg(all(target_os = "macos", not(test)))]
            if permission_error.is_some() {
                self.open(path);
            } else {
                self.open_from_user(path);
            }
            #[cfg(not(all(target_os = "macos", not(test))))]
            self.open_from_user(path);
            #[cfg(all(target_os = "macos", not(test)))]
            if let Some(error) = permission_error {
                self.last_error = error;
            }
        }
        // Persist only after a frame containing an image has already been submitted.
        if !self.pictures.is_empty()
            && let Some(dir) = self.library.remember.take()
            && self.settings.remember_recent
            && let Some(s) = &self.store
        {
            if let Err(e) = s.record_folder(&dir, self.settings.recent_limit) {
                self.last_error = e;
            } else {
                self.remember_page();
            }
        }
        if let Some(sample) = self.memory_monitor.take() {
            self.apply_memory_sample(sample);
        }
        // Keep the last known monitor if the OS temporarily cannot report it.
        // Never fall back to window geometry, which would change page grouping
        // when resizing, pinning chrome, or entering full screen.
        if let Some(size) = ctx
            .input(|i| i.viewport().monitor_size)
            .filter(|size| size.is_finite() && size.x > 0. && size.y > 0.)
        {
            self.layout_monitor = size;
        }
        // Mobile follows the current app window, including tablet rotation and
        // split-screen resizing. Desktops follow the monitor so that resizing
        // the window does not toggle the spread.
        let display = if crate::app::is_mobile() || self.layout_monitor == Vec2::ZERO {
            ctx.viewport_rect().size()
        } else {
            self.layout_monitor
        };
        self.landscape = display.x > display.y;
        self.poll_archive(ctx);
        self.collect(ctx);
        self.poll_reveal();
        self.keyboard(ctx);
        self.library_mouse(ctx);
        self.advance_slideshow(ctx);
        self.update_controls(ctx);
        self.inspector.poll();
        self.animate(ctx);
        // The settings screen covers the viewer and its bars.
        let chrome = self.controls_visible && !self.settings_open;
        if self.settings_open {
            // Only toasts about a setting belong on the settings screen.
            self.toast.take_if(|toast| !toast.keep_on_slideshow);
            self.held_button = None;
        }
        self.canvas(root);
        if chrome {
            self.top(ctx);
            self.bottom(ctx);
            self.thumbnail_bar(ctx);
            self.notice_overlay(ctx);
        }
        self.windows(ctx);
        self.sync_native_shortcut_capture();
        self.toast_overlay(ctx);
        if !crate::app::is_mobile() {
            window_frame::resize(ctx);
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.held_button = None;
        }
        self.queue_images();
        if !chrome {
            self.media.clear_thumbnail();
            self.thumb_ticket = None;
        }
        self.queue_prefetch();
        if self.library.remember.is_some() && !self.pictures.is_empty() {
            ctx.request_repaint();
        }
        if !crate::app::is_mobile() {
            self.cursor_idle.apply(ctx, self.settings.cursor_hide_ms);
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        crate::platform::mobile::clipboard(ctx);
        self.display_sleep
            .update(self.wants_display_awake(ctx), ctx);
    }
}

impl Viewer {
    /// Desktop keeps the display awake only while presenting; mobile while focused.
    fn wants_display_awake(&self, ctx: &egui::Context) -> bool {
        self.settings.prevent_display_sleep
            && if crate::app::is_mobile() {
                ctx.input(|i| i.focused)
            } else {
                self.fullscreen || self.slideshow
            }
    }
}

#[cfg(any(target_os = "ios", target_os = "android", test))]
fn prepare_mobile_safe_area(
    ctx: &egui::Context,
    input: &mut egui::RawInput,
    physical_insets: [f32; 4],
) {
    // Keep the existing native-window pointer coordinates and inset viewport.
    // egui-winit 0.37 also supplies iOS insets: clear those every time (None
    // retains the preceding frame), or Area/Window constraints subtract twice.
    input.safe_area_insets = Some(Default::default());
    if let Some(screen) = input.screen_rect {
        let ppp = input
            .viewports
            .get(&input.viewport_id)
            .and_then(|v| v.native_pixels_per_point)
            .unwrap_or(ctx.pixels_per_point());
        input.screen_rect = Some(crate::ui::mobile::safe_rect(screen, physical_insets, ppp));
    }
}

fn format_zoom_percent(percent: f32) -> String {
    format!("{percent:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn parse_zoom_percent(text: &str) -> Option<f32> {
    let text = text.trim().strip_suffix('%').unwrap_or(text.trim()).trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let percent: f32 = text.parse().ok()?;
    ZOOM_PERCENT.contains(&percent).then_some(percent)
}

fn clamp_zoom_percent(percent: f32) -> f32 {
    percent.clamp(*ZOOM_PERCENT.start(), *ZOOM_PERCENT.end())
}

fn chrome_drag(ui: &mut egui::Ui, rect: Rect) {
    // Register behind the controls, so their click/drag interactions take priority.
    // Even a small real movement can drag blank space, but a click or stationary
    // hold (especially the click that activates the window) must not start it.
    let response = ui.interact(
        rect,
        ui.scope_id().with("window-drag"),
        egui::Sense::click_and_drag(),
    );
    let blocked_id = response.id.with("pressed-on-control");
    if ui.input(|i| i.pointer.primary_pressed()) {
        let hovered = ui
            .ctx()
            .interaction_snapshot(|snapshot| snapshot.hovered.clone());
        let blocked = hovered.iter().any(|&id| {
            id != response.id
                && ui
                    .ctx()
                    .read_response(id)
                    .is_some_and(|r| r.sense.is_focusable() && r.sense.senses_click())
        });
        ui.ctx()
            .data_mut(|data| data.insert_temp(blocked_id, blocked));
    }
    if !ui
        .ctx()
        .data(|data| data.get_temp::<bool>(blocked_id).unwrap_or(false))
    {
        window_frame::drag(ui.ctx(), &response);
    }
}

fn uv() -> Rect {
    Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.))
}

fn image_uv(horizontal: bool, vertical: bool) -> Rect {
    // Reversed UV axes mirror the sampled pixels without modifying the texture or source.
    Rect {
        min: egui::pos2(
            if horizontal { 1. } else { 0. },
            if vertical { 1. } else { 0. },
        ),
        max: egui::pos2(
            if horizontal { 0. } else { 1. },
            if vertical { 0. } else { 1. },
        ),
    }
}

fn key_chord(key: Key, m: egui::Modifiers) -> String {
    let mut parts = Vec::new();
    if m.command {
        parts.push("Primary");
    }
    if cfg!(target_os = "macos") && m.ctrl {
        parts.push("Ctrl");
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    parts.push(key.name());
    parts.join("+")
}

fn thumbnail_range(len: usize, focus: usize, width: f32, spacing: f32) -> std::ops::Range<usize> {
    let count = (((width.max(0.) + spacing) / (THUMBNAIL_WIDTH + spacing)).floor() as usize)
        .max(1)
        .min(len);
    let first = focus.saturating_sub(count / 2).min(len - count);
    first..first + count
}

struct SpanCache {
    key: (usize, usize, bool, bool, bool, Vec2),
    span: (usize, usize),
}

enum NavigationStep {
    Spread,
    Page,
    Check,
}

/// Commands that switch between single-page and two-page layouts.
fn is_layout_command(id: &str) -> bool {
    matches!(id, "single" | "spread" | "spread-left" | "spread-right")
}

// A page limited by the width of half the monitor gets the whole canvas.
// Only the aspect ratio matters, so OS/UI scaling does not affect grouping.
fn width_limited(size: [u32; 2], monitor: Vec2) -> bool {
    size[0] > 0
        && size[1] > 0
        && monitor.x > 0.
        && monitor.y > 0.
        && size[0] as f64 * monitor.y as f64 * 2. >= size[1] as f64 * monitor.x as f64
}

fn adaptive_focus_span(
    focus: usize,
    len: usize,
    spread: bool,
    cover: bool,
    shifted: bool,
    single: impl Fn(usize) -> bool,
) -> (usize, usize) {
    if len == 0 || !spread || (cover && focus == 0) || single(focus) {
        return (focus, focus);
    }
    // Pair only within the run after the nearest preceding standalone page.
    let mut offset = (0..focus)
        .rev()
        .find(|&i| (cover && i == 0) || single(i))
        .map_or(0, |i| i + 1);
    if shifted {
        if focus == offset {
            return (focus, focus);
        }
        offset += 1;
    }
    let first = offset + (focus - offset) / 2 * 2;
    let last = (first + 1).min(len - 1);
    (first, if single(last) { first } else { last })
}

#[cfg(test)]
// Keep the focused page as the anchor when cover, direction, or spread mode changes.
fn next_page(focus_index: usize, len: usize, spread: bool, cover: bool, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    let (left, right) = page_focus_span(focus_index, len, spread, cover);
    if forward {
        (right + 1) % len
    } else if left == 0 {
        len - 1
    } else {
        left - 1
    }
}

// Match the filesystem's spelling before image and folder jobs use the path
// as a key. Finder/Explorer can supply a different Unicode/case spelling.
// Preserve a selected symlink entry and its neighbors, rather than opening
// the target in a different folder. Missing paths retain their error page.
fn normalize_open_path(path: PathBuf) -> PathBuf {
    let path = crate::core::paths::simplified(std::path::absolute(&path).unwrap_or(path));
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return path;
    }
    // read_dir retains the supplied parent path. Only its entry name comes
    // from the filesystem; keep parent aliases (and Windows path syntax).
    path.canonicalize()
        .ok()
        .and_then(|resolved| resolved.file_name().map(|name| path.with_file_name(name)))
        .unwrap_or(path)
}

fn refreshed_focus_index(old: &[PathBuf], focus_index: usize, new: &[PathBuf]) -> usize {
    if new.is_empty() {
        return 0;
    }
    let Some(focused_path) = old.get(focus_index) else {
        return 0;
    };
    new.iter()
        .position(|p| p == focused_path)
        .or_else(|| {
            new.iter()
                .position(|p| media::compare_file_names(p, focused_path).is_gt())
        })
        .unwrap_or(0)
}

fn shuffle_paths(files: &mut [PathBuf]) {
    use std::hash::BuildHasher;
    // A fresh, independently seeded hash order for each cycle, without a new dependency.
    let previous = files.to_vec();
    let seed = std::collections::hash_map::RandomState::new();
    files.sort_by_cached_key(|path| seed.hash_one(path));
    if files.len() > 1 && files == previous {
        files.rotate_left(1);
    }
}

fn reconcile_random(previous: &[PathBuf], live: &[PathBuf]) -> Vec<PathBuf> {
    let live_set: HashSet<_> = live.iter().collect();
    let old_set: HashSet<_> = previous.iter().collect();
    let mut result: Vec<_> = previous
        .iter()
        .filter(|p| live_set.contains(p))
        .cloned()
        .collect();
    let mut added: Vec<_> = live
        .iter()
        .filter(|p| !old_set.contains(p))
        .cloned()
        .collect();
    shuffle_paths(&mut added);
    result.extend(added);
    result
}

#[cfg(test)]
fn page_focus_span(focus_index: usize, len: usize, spread: bool, cover: bool) -> (usize, usize) {
    if len == 0 || !spread || (cover && focus_index == 0) {
        return (focus_index, focus_index);
    }
    let offset = usize::from(cover);
    let first = offset + (focus_index - offset) / 2 * 2;
    (first, (first + 1).min(len - 1))
}

fn effective_keys(s: &Settings, id: &str) -> Vec<String> {
    if !command_supported(id) {
        return Vec::new();
    }
    s.keyboard_shortcuts
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|row| row.action == id)
        .map(|row| canonical_chord(&row.key))
        .collect()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod half_step_tests;

fn scroll_source(drag: bool) -> egui::scroll_area::ScrollSource {
    egui::scroll_area::ScrollSource {
        drag: if drag {
            egui::scroll_area::DragScroll::Always
        } else {
            egui::scroll_area::DragScroll::Never
        },
        ..Default::default()
    }
}
