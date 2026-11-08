mod actions;
mod animation;
mod canvas;
mod chrome;
mod image_pipeline;
mod input;
mod navigation;
mod toasts;
use crate::core::commands::*;
use crate::i18n::{Language, tr};
#[cfg(test)]
mod cache_memory_tests;
mod checked_files;
mod chrome_icons;
mod forms;
mod menu;
mod metadata;
mod mobile_chrome;
mod move_files;
mod overlay;
mod recent;
mod settings;
mod window_frame;
use crate::core::media::{self, Media, MediaEvent};
use crate::core::settings::{KeyboardShortcut, Settings};
use crate::storage::Store;
use eframe::egui::{self, Color32, FontId, Key, Rect, RichText, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const BAR: Color32 = Color32::from_rgb(21, 23, 25);
const INK: Color32 = Color32::from_rgb(236, 237, 231);
const MUTED: Color32 = Color32::from_rgb(159, 165, 161);
const ACCENT: Color32 = Color32::from_rgb(204, 232, 165);
const TOP_BAR_HEIGHT: f32 = 47.;
const STATUS_BAR_HEIGHT: f32 = 45.;
const THUMBNAIL_BAR_HEIGHT: f32 = 78.;
const THUMBNAIL_WIDTH: f32 = 65.;
const THUMBNAIL_MARGIN: f32 = 12.;
const BOTTOM_CHROME_HEIGHT: f32 = STATUS_BAR_HEIGHT + THUMBNAIL_BAR_HEIGHT;

fn command_supported(id: &str) -> bool {
    (!crate::app::is_mobile() || crate::ui::mobile::supported(id))
        && (id != "reveal" || crate::platform::reveal_file::supported())
}

struct Picture {
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
    handle: egui::TextureHandle,
    rect: Rect,
    clip: Rect,
}

#[derive(Default)]
struct Presentation {
    images: Vec<PresentedImage>,
    canvas: Option<Rect>,
}

impl Presentation {
    fn paint(&self, ui: &egui::Ui, canvas: Rect, image_uv: Rect) {
        let Some(previous) = self.canvas else { return };
        let transform = egui::emath::RectTransform::from_to(previous, canvas);
        let scale = (canvas.width() / previous.width()).min(canvas.height() / previous.height());
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
            crate::ui::transparency::paint(&painter, rect, 12.);
            painter.image(image.handle.id(), rect, image_uv, Color32::WHITE);
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
    inspector: metadata::Inspector,
    store: Option<Store>,
    settings: Settings,
    files: Vec<PathBuf>,
    // Current page anchoring the focused spread. This is navigation state,
    // independent of widget/keyboard focus and checked selection.
    focus_index: usize,
    checked: HashSet<PathBuf>,
    random_prompt: bool,
    trash_history: Vec<Vec<crate::platform::trash::TrashEntry>>,
    file_job: Option<checked_files::FileJob>,
    close_after_file_job: bool,
    closing_all: bool,
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
    span_cache: std::cell::RefCell<Option<SpanCache>>,
    presentation: Presentation,
    transition_waiting: bool,
    pending_steps: Vec<(bool, bool)>,
    thumb_ticket: Option<(u64, PathBuf)>,
    thumbs: HashMap<PathBuf, egui::TextureHandle>,
    app_icon: egui::TextureHandle,
    thumb_errors: HashSet<PathBuf>,
    failures: HashMap<PathBuf, String>,
    last_error: String,
    frame: u64,
    remember: Option<PathBuf>,
    recent: Vec<PathBuf>,
    recent_moves: Vec<PathBuf>,
    move_dialog: Option<move_files::MoveDialog>,
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
    reveal_job: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    recent_open: bool,
    clear_prompt: bool,
    notices: Option<Vec<String>>,
    show_full_notices: bool,
    hide_controls: bool,
    controls_visible: bool,
    hover_until: f64,
    held_key: Option<(Key, &'static str, f64)>,
    held_button: Option<(bool, f64)>,
    canvas_drag: Option<CanvasDrag>,
    touch_gesture: crate::ui::mobile::TouchGesture,
    #[cfg(any(target_os = "ios", target_os = "android"))]
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
        let mut style = (*ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BAR;
        style.visuals.window_fill = Color32::from_rgb(28, 30, 31);
        style.visuals.selection.bg_fill = Color32::from_rgb(64, 82, 55);
        style.visuals.widgets.active.fg_stroke.color = ACCENT;
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(9.0, 5.0);
        if crate::app::is_mobile() {
            style.spacing.interact_size = egui::vec2(44., 44.);
        }
        ctx.set_style(style);
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
            inspector: metadata::Inspector::default(),
            store: None,
            settings: Settings::default(),
            files: Vec::new(),
            focus_index: 0,
            checked: HashSet::new(),
            random_prompt: false,
            trash_history: Vec::new(),
            file_job: None,
            close_after_file_job: false,
            closing_all: false,
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
            remember: None,
            recent: Vec::new(),
            recent_moves: Vec::new(),
            move_dialog: None,
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
            reveal_job: None,
            recent_open: false,
            clear_prompt: false,
            notices: None,
            show_full_notices: false,
            hide_controls: !crate::app::is_mobile(),
            controls_visible: false,
            hover_until: 0.,
            held_key: None,
            held_button: None,
            canvas_drag: None,
            touch_gesture: Default::default(),
            #[cfg(any(target_os = "ios", target_os = "android"))]
            mobile_selection: None,
            window_size_state: window_frame::WindowSizeState::default(),
            fullscreen: false,
            random: false,
            slideshow: false,
            slide_deadline: None,
        };
        if let Some(path) = initial {
            app.open(path);
        }
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
        crate::i18n::set_language(app.settings.language.clone());
        if app.files.is_empty() && app.listing.is_none() {
            app.recent_open = true;
            app.reload_recent();
        }
        app
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
        self.recent_window(ctx);
        self.move_window(ctx);
        if self.clear_prompt {
            egui::Window::new(tr("clear-recent-items-confirm"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(tr("remove-the-recent-list-on-this-device-confirm"));
                    ui.horizontal(|ui| {
                        if ui.button(tr("cancel")).clicked() {
                            self.clear_prompt = false;
                        }
                        if ui.button(tr("clear-list")).clicked() {
                            if let Some(s) = &self.store {
                                match s.clear_recent() {
                                    Ok(()) => {
                                        self.recent.clear();
                                        self.remember = None;
                                    }
                                    Err(e) => self.last_error = e,
                                }
                            }
                            self.clear_prompt = false;
                        }
                    });
                });
        }
    }
}

impl eframe::App for Viewer {
    fn on_exit(&mut self) {
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
        if let Some(screen) = input.screen_rect {
            let ppp = input
                .viewports
                .get(&input.viewport_id)
                .and_then(|v| v.native_pixels_per_point)
                .unwrap_or(ctx.pixels_per_point());
            input.screen_rect = Some(crate::ui::mobile::safe_rect(
                screen,
                crate::platform::mobile::safe_area(),
                ppp,
            ));
        }
        self.sync_native_shortcut_capture();
        #[cfg(all(target_os = "macos", not(test)))]
        if let Some(viewport) = input.viewports.get_mut(&input.viewport_id) {
            viewport.fullscreen = Some(self.fullscreen);
        }
        self.popup_open_at_input = self.menu_open || ctx.memory(|m| m.any_popup_open());
        let capturing = self.shortcut_editor.as_ref().is_some_and(|e| e.listening)
            || self
                .shortcut_search
                .as_ref()
                .is_some_and(|search| search.is_empty());
        if self.zoom_input.is_some()
            || (!capturing
                && (self.settings_open
                    || self.recent_open
                    || self.move_dialog.is_some()
                    || self.clear_prompt
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

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !crate::app::is_mobile() {
            self.cursor_idle.observe(ctx);
        }
        // Widgets may replace this with resize/grab/text cursors below.
        ctx.set_cursor_icon(egui::CursorIcon::Default);
        crate::i18n::set_language(self.settings.language.clone());
        #[cfg(all(target_os = "macos", not(test)))]
        crate::platform::mac_open::localize_menu();
        #[cfg(all(target_os = "macos", not(test)))]
        if self.fullscreen {
            self.fullscreen =
                crate::platform::mac_open::fullscreen(true, self.settings.blackout_other_monitors);
        }
        self.frame += 1;
        #[cfg(any(target_os = "ios", target_os = "android"))]
        if crate::platform::mobile::take_back() {
            if self.menu_open {
                self.menu_open = false;
            } else if self.settings_open {
                self.settings_open = false;
            } else if self.inspector.open {
                self.inspector.close();
            } else {
                #[cfg(target_os = "android")]
                crate::platform::mobile::exit();
            }
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        if let Some(result) = crate::platform::mobile::take_result() {
            match result {
                Ok(selection) => {
                    if let Some(path) = selection.first() {
                        self.open(path.to_owned());
                        self.mobile_selection = Some(selection);
                    }
                }
                Err(error) => self.last_error = error,
            }
        }
        #[cfg(target_os = "macos")]
        if crate::platform::mac_open::take_about() {
            self.run("about", ctx);
        }
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
        if let Err(error) = if crate::app::is_mobile() {
            Ok(())
        } else {
            self.window_size_state.observe(ctx, self.store.as_ref())
        } {
            self.last_error = error;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.shortcut_input_active() && !self.closing_all {
                // Alt-F4 / native window close is not an egui key action.
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
                return;
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(request) = crate::platform::mac_open::take() {
            match request {
                crate::app::OpenRequest::Here(path) => self.open(path),
                crate::app::OpenRequest::New(path) => self.new_window(path.as_deref()),
            }
        }
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect::<Vec<_>>()
        });
        for path in dropped {
            self.open_from_user(path);
        }
        // Persist only after a frame containing an image has already been submitted.
        if !self.pictures.is_empty()
            && let Some(dir) = self.remember.take()
            && self.settings.remember_recent
            && let Some(s) = &self.store
            && let Err(e) = s.record_folder(&dir, self.settings.recent_limit)
        {
            self.last_error = e;
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
        self.collect(ctx);
        if let Some(receiver) = &self.reveal_job {
            match receiver.try_recv() {
                Ok(result) => {
                    self.reveal_job = None;
                    if let Err(error) = result {
                        self.last_error = error;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.reveal_job = None;
                    self.last_error = tr("file-manager-request-failed").into();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        self.poll_file_job(ctx);
        self.keyboard(ctx);
        self.advance_slideshow(ctx);
        self.update_controls(ctx);
        self.inspector.poll();
        self.animate(ctx);
        if self.settings_open {
            if self
                .toast
                .as_ref()
                .is_some_and(|toast| !toast.keep_on_slideshow)
            {
                self.toast = None;
            }
            self.held_button = None;
            self.canvas(ctx);
            self.windows(ctx);
            self.toast_overlay(ctx);
            self.sync_native_shortcut_capture();
            self.queue_images();
            self.media.clear_thumbnail();
            self.thumb_ticket = None;
            self.queue_prefetch();
            if !crate::app::is_mobile() {
                window_frame::resize(ctx);
            }
            if !crate::app::is_mobile() {
                self.cursor_idle.apply(ctx, self.settings.cursor_hide_ms);
            }
            #[cfg(any(target_os = "ios", target_os = "android"))]
            crate::platform::mobile::clipboard(ctx);
            self.display_sleep.update(
                self.settings.prevent_display_sleep
                    && (!crate::app::is_mobile() || ctx.input(|i| i.focused)),
                ctx,
            );
            return;
        }
        self.canvas(ctx);
        if self.controls_visible {
            if crate::app::is_mobile() {
                self.mobile_chrome(ctx);
            } else {
                self.top(ctx);
                self.bottom(ctx);
                self.thumbnail_bar(ctx);
            }
            if !self.last_error.is_empty() {
                egui::Area::new(egui::Id::new("notice"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(ctx.screen_rect().min + egui::vec2(14., 52.))
                    .show(ctx, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui.small_button("×").clicked() {
                                self.last_error.clear();
                            }
                            ui.label(
                                RichText::new(crate::i18n::value_label(
                                    "error-details",
                                    &self.last_error,
                                ))
                                .color(Color32::from_rgb(244, 198, 145)),
                            );
                        });
                    });
            }
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
        if !self.controls_visible {
            self.media.clear_thumbnail();
            self.thumb_ticket = None;
        }
        self.queue_prefetch();
        if self.remember.is_some() && !self.pictures.is_empty() {
            ctx.request_repaint();
        }
        if !crate::app::is_mobile() {
            self.cursor_idle.apply(ctx, self.settings.cursor_hide_ms);
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        crate::platform::mobile::clipboard(ctx);
        self.display_sleep.update(
            self.settings.prevent_display_sleep
                && (!crate::app::is_mobile() || ctx.input(|i| i.focused)),
            ctx,
        );
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
    (10. ..=3000.).contains(&percent).then_some(percent)
}

fn chrome_drag(ui: &mut egui::Ui, rect: Rect) {
    // Register behind the controls, so their click/drag interactions take priority.
    // Even a small real movement can drag blank space, but a click or stationary
    // hold (especially the click that activates the window) must not start it.
    let response = ui.interact(
        rect,
        ui.id().with("window-drag"),
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
    key: (usize, usize, bool, bool, Vec2),
    span: (usize, usize),
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
    single: impl Fn(usize) -> bool,
) -> (usize, usize) {
    if len == 0 || !spread || (cover && focus == 0) || single(focus) {
        return (focus, focus);
    }
    // Pair only within the run after the nearest preceding standalone page.
    let offset = (0..focus)
        .rev()
        .find(|&i| (cover && i == 0) || single(i))
        .map_or(0, |i| i + 1);
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
    let path = std::path::absolute(&path).unwrap_or(path);
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
