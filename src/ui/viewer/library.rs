//! Recent folders and bookmarks share one overlay. Each list scrolls on its
//! own: side by side when both fit, otherwise behind tabs.
use super::*;
use crate::core::library::{self, Entry, Kind};
use std::sync::mpsc;

const ROW_HEIGHT: f32 = 68.;
const THUMBNAIL_SIZE: f32 = 56.;
/// Narrowest list that still fits a thumbnail, two lines of text and its buttons.
const MIN_COLUMN_WIDTH: f32 = 240.;
/// Thumbnail textures kept while the overlay is open; rows reload evicted ones.
const TEXTURE_LIMIT: usize = 64;
/// Unused thumbnails stay this long, so another window can still record new ones.
const THUMBNAIL_GRACE: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const NAME_FIELD: &str = "bookmark-name";

pub(super) struct Item {
    pub entry: Entry,
    /// Cache key of the entry's image as it is now; None without an image.
    key: Option<String>,
}

impl Item {
    fn new(entry: Entry) -> Self {
        let key = entry.image.as_deref().and_then(library::thumbnail_key);
        Self { entry, key }
    }

    /// The current image's thumbnail, or the last one written for the entry.
    fn shown(&self) -> Option<&str> {
        self.key.as_deref().or(self.entry.thumbnail.as_deref())
    }
}

#[derive(Default)]
pub(super) struct Library {
    pub open: bool,
    /// The list shown when only one fits.
    tab: Kind,
    pub items: Vec<Item>,
    /// Folder opened in this window, recorded once its first image is painted.
    pub remember: Option<PathBuf>,
    pub clear_prompt: bool,
    pub dialog: Option<BookmarkDialog>,
    thumbnails: Thumbnails,
}

impl Library {
    /// Library layers that take keyboard input away from the viewer.
    pub(super) fn modal(&self) -> bool {
        self.open || self.clear_prompt || self.dialog.is_some()
    }
}

/// Adds a bookmark at the shown image, or renames an existing bookmark.
pub(super) struct BookmarkDialog {
    folder: PathBuf,
    image: PathBuf,
    name: String,
    preview: Option<egui::TextureHandle>,
    rename: bool,
    /// Adding to a bookmarked folder updates that bookmark.
    replaces: bool,
    focus: bool,
    error: String,
}

#[derive(Default)]
struct Thumbnails {
    worker: Option<Worker>,
    /// Textures by cache key, with the frame that last showed them.
    textures: HashMap<String, (egui::TextureHandle, u64)>,
    requested: HashSet<String>,
    failed: HashSet<String>,
}

struct Job {
    /// The texture this job fills.
    shown: String,
    /// Image to make the thumbnail from, and its cache key.
    source: Option<(PathBuf, String)>,
    /// Thumbnail written earlier, shown when the image cannot be read.
    fallback: Option<String>,
    owner: (Kind, PathBuf),
}

struct Done {
    shown: String,
    owner: (Kind, PathBuf),
    /// Pixels to show, and the cache key to record when they are the image's own.
    result: Result<(egui::ColorImage, Option<String>), String>,
}

/// Reads and writes thumbnail files away from the UI thread.
struct Worker {
    jobs: mpsc::Sender<Job>,
    done: mpsc::Receiver<Done>,
    pending: usize,
    limit: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Worker {
    fn start(dir: PathBuf, ctx: egui::Context, cache_limit: u64) -> Option<Self> {
        let limit = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(cache_limit));
        let worker_limit = limit.clone();
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (outbox, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("thumbnails".into())
            .spawn(move || {
                for job in inbox {
                    let limit = worker_limit.load(std::sync::atomic::Ordering::Relaxed);
                    let result = if limit == 0 {
                        job.source
                            .as_ref()
                            .ok_or_else(|| "No thumbnail".to_owned())
                            .and_then(|(image, _)| {
                                library::decode_thumbnail(image).map(|pixels| (pixels, None))
                            })
                    } else {
                        let result = make_thumbnail(&dir, &job);
                        if let Some(root) = dir.parent() {
                            let _ =
                                crate::core::disk_cache::Cache::new(root.to_owned(), limit).prune();
                        }
                        result
                    };
                    let done = Done {
                        shown: job.shown,
                        owner: job.owner,
                        result,
                    };
                    if outbox.send(done).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self {
            jobs,
            done,
            pending: 0,
            limit,
        })
    }
}

fn make_thumbnail(dir: &Path, job: &Job) -> Result<(egui::ColorImage, Option<String>), String> {
    let mut error = String::from("No thumbnail");
    if let Some((image, key)) = &job.source {
        // Another window may already have written it.
        match library::load_thumbnail(dir, key)
            .or_else(|_| library::create_thumbnail(image, dir, key))
        {
            Ok(pixels) => return Ok((pixels, Some(key.clone()))),
            Err(e) => error = e,
        }
    }
    match &job.fallback {
        Some(key) => library::load_thumbnail(dir, key).map(|pixels| (pixels, None)),
        None => Err(error),
    }
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Recent => "recent",
        Kind::Bookmark => "bookmarks",
    }
}

#[cfg(test)]
mod cache_disabled_tests {
    use super::*;
    use image::ImageEncoder;
    use std::{io::Write, time::Duration};

    #[test]
    fn uncached_thumbnails_resolve_mobile_images_and_archive_pages() {
        let root = tempfile::tempdir().unwrap();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[220, 80, 40, 128], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("chapter/page.png", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&png).unwrap();
        let archive = zip.finish().unwrap().into_inner();
        let mut selection = crate::core::documents::ImportBatch::new(root.path()).unwrap();
        selection.add("page.png", png.as_slice()).unwrap();
        selection.add("book.zip", archive.as_slice()).unwrap();
        selection
            .set_source(
                &root.path().join("document-sources"),
                "12345678-thumbnails",
                "Photos",
            )
            .unwrap();
        selection.commit_source().unwrap();
        let source = selection.source().unwrap().root.clone();
        let cache = root.path().join("cache/v1/thumbnails");
        let worker = Worker::start(cache.clone(), egui::Context::default(), 0).unwrap();
        for relative in ["page.png", "book.zip/chapter/page.png"] {
            let logical = source.join(relative);
            let key = library::thumbnail_key(&logical).unwrap();
            worker
                .jobs
                .send(Job {
                    shown: key.clone(),
                    owner: (Kind::Recent, source.clone()),
                    source: Some((logical, key)),
                    fallback: None,
                })
                .unwrap();
            let done = worker.done.recv_timeout(Duration::from_secs(10)).unwrap();
            let (pixels, stored) = done.result.unwrap();
            assert_eq!(pixels.size, [1, 1]);
            assert_eq!(pixels.pixels[0].a(), 128);
            assert!(stored.is_none());
        }
        assert!(!cache.exists(), "disabled cache wrote a thumbnail");
    }
}

impl Viewer {
    pub(super) fn reload_library(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        match store.library() {
            Ok(entries) => self.library.items = entries.into_iter().map(Item::new).collect(),
            Err(error) => self.last_error = error,
        }
        // Thumbnails go once no entry uses them.
        if let Err(error) = store.prune_thumbnails(THUMBNAIL_GRACE) {
            self.last_error = error;
        }
    }

    /// Saves the page shown in this window's folder for its recent entry, so the
    /// folder reopens there and shows it as its thumbnail.
    pub(super) fn remember_page(&mut self) {
        // A folder whose first image was never painted has not been recorded.
        if !self.settings.remember_recent || self.library.remember.is_some() {
            return;
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        if self
            .mobile_selection
            .as_ref()
            .and_then(|selection| selection.source())
            .is_none()
        {
            return;
        }
        // A page of an archive is saved by its path inside the archive.
        if let (Some(store), Some(image)) = (&self.store, self.files.get(self.focus_index))
            && let Err(error) = store.remember_page(&self.page_location(image))
        {
            self.last_error = error;
        }
    }

    pub(super) fn library_window(&mut self, ctx: &egui::Context) {
        if !self.library.open {
            return;
        }
        let mut close = false;
        let mut action = None;
        let mut opened = None;
        let mut removed = None;
        let mut renamed = None;
        let mut visible = Vec::new();
        let mut tab = self.library.tab;
        let compact = overlay::compact(ctx);
        let library = &self.library;
        let modal = overlay::show(ctx, "library", self.fullscreen, |ui| {
            let side_by_side = ui.available_width() >= 2. * MIN_COLUMN_WIDTH + forms::GAP;
            crate::ui::direction::horizontal(ui, |ui| {
                for (id, icon) in [("open", "library-open"), ("folder", "library-folder")] {
                    if command_supported(id)
                        && chrome_icons::sized_button(
                            ui,
                            icon,
                            command_label(id),
                            false,
                            &self.shortcut_text(id),
                            forms::height(),
                        )
                        .clicked()
                    {
                        action = Some(id);
                    }
                }
                if compact {
                    ui.with_layout(crate::ui::direction::trailing(), |ui| {
                        close = forms::icon(ui, "library-close", "back-to-viewer").clicked();
                    });
                }
            });
            if !side_by_side {
                crate::ui::direction::horizontal(ui, |ui| {
                    let width = (ui.available_width() - forms::GAP) / 2.;
                    for kind in Kind::ALL {
                        let response = ui.push_id(("library-tab", kind.code()), |ui| {
                            ui.add_sized(
                                [width, forms::height()],
                                egui::Button::new(tr(kind_label(kind)))
                                    .selected(tab == kind)
                                    .truncate(),
                            )
                        });
                        if response.inner.clicked() {
                            tab = kind;
                        }
                    }
                });
            }
            ui.add_space(4.);
            if !self.last_error.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(&self.last_error).small().color(WARNING))
                        .truncate(),
                )
                .hover_text(&self.last_error);
            }
            let mut list = |ui: &mut egui::Ui, kind: Kind| {
                let rows = entry_list(ui, library, kind);
                visible.extend(rows.visible);
                opened = opened.or(rows.opened);
                removed = removed.or(rows.removed);
                renamed = renamed.or(rows.renamed);
            };
            let height = ui.available_height().max(1.);
            if side_by_side {
                let width = (ui.available_width() - forms::GAP) / 2.;
                let layout = if crate::i18n::is_rtl() {
                    egui::Layout::right_to_left(egui::Align::Min)
                } else {
                    egui::Layout::left_to_right(egui::Align::Min)
                };
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), height),
                    layout,
                    |ui| {
                        for kind in Kind::ALL {
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, height),
                                crate::ui::direction::vertical(),
                                |ui| {
                                    ui.set_min_size(egui::vec2(width, height));
                                    ui.set_max_width(width);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(tr(kind_label(kind))).strong().color(INK),
                                        )
                                        .truncate(),
                                    );
                                    list(ui, kind);
                                },
                            );
                        }
                    },
                );
            } else {
                list(ui, tab);
            }
        });
        self.library.tab = tab;
        if close || modal.should_close() {
            self.library.open = false;
        }
        self.request_thumbnails(ctx, &visible);
        if let Some(index) = renamed {
            self.open_rename_dialog(index);
        }
        if let Some(index) = opened {
            let entry = &self.library.items[index].entry;
            #[cfg(all(target_os = "macos", not(test)))]
            let target = Some(entry.image.clone().unwrap_or_else(|| entry.folder.clone()));
            #[cfg(not(all(target_os = "macos", not(test))))]
            let target = entry
                .image
                .clone()
                .filter(|image| library::page_exists(image))
                .or_else(|| library::first_image(&entry.folder));
            if let Some(target) = target {
                // Open a concrete image, so the folder starts at its saved page
                // in this window, a new window or on a phone alike.
                if self.open_from_user(target) {
                    self.library.open = false;
                }
            } else if entry.kind == Kind::Recent {
                // Access or folder contents may have changed since opening the list.
                removed = Some(index);
            } else {
                self.library.items[index].entry.available = false;
                self.last_error = tr("folder-unavailable").into();
            }
        }
        if let Some(index) = removed {
            self.remove_library_entry(index);
        }
        // A cancelled picker keeps the start screen available. Opening in the same
        // window already closes it in open(); opening a new window closes it here.
        if let Some(action) = action {
            let had_images = !self.files.is_empty() || self.listing.is_some();
            self.run(action, ctx);
            if had_images {
                self.library.open = false;
            }
        }
        self.release_held_input();
    }

    /// Confirms removing every recent folder; bookmarks stay.
    pub(super) fn clear_recent_window(&mut self, ctx: &egui::Context) {
        if !self.library.clear_prompt {
            return;
        }
        egui::Window::new(tr("clear-recent-items-confirm"))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(tr("remove-the-recent-list-on-this-device-confirm"));
                ui.horizontal(|ui| {
                    if ui.button(tr("cancel")).clicked() {
                        self.library.clear_prompt = false;
                    }
                    if ui.button(tr("clear-list")).clicked() {
                        self.clear_recent();
                        self.library.clear_prompt = false;
                    }
                });
            });
    }

    fn clear_recent(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        match store.clear_recent() {
            Ok(()) => {
                self.library
                    .items
                    .retain(|item| item.entry.kind != Kind::Recent);
                self.library.remember = None;
            }
            Err(error) => self.last_error = error,
        }
    }

    fn remove_library_entry(&mut self, index: usize) {
        let (kind, folder) = {
            let entry = &self.library.items[index].entry;
            (entry.kind, entry.folder.clone())
        };
        let Some(store) = &self.store else {
            self.library.items.remove(index);
            return;
        };
        match store.remove_entry(kind, &folder) {
            Ok(()) => {
                self.library.items.remove(index);
                if kind == Kind::Recent && self.library.remember.as_ref() == Some(&folder) {
                    self.library.remember = None;
                }
                if let Err(error) = store.prune_thumbnails(THUMBNAIL_GRACE) {
                    self.last_error = error;
                }
            }
            Err(error) => self.last_error = error,
        }
    }

    /// Loads or creates thumbnails for rows shown this frame.
    fn request_thumbnails(&mut self, ctx: &egui::Context, visible: &[usize]) {
        let mut jobs = Vec::new();
        let thumbnails = &mut self.library.thumbnails;
        for &index in visible {
            let item = &self.library.items[index];
            let Some(shown) = item.shown() else {
                continue;
            };
            if let Some((_, used)) = thumbnails.textures.get_mut(shown) {
                *used = self.frame;
                continue;
            }
            if thumbnails.requested.contains(shown) || thumbnails.failed.contains(shown) {
                continue;
            }
            jobs.push(Job {
                shown: shown.to_owned(),
                source: item
                    .key
                    .clone()
                    .zip(item.entry.image.clone())
                    .map(|(key, image)| (image, key)),
                fallback: item
                    .entry
                    .thumbnail
                    .clone()
                    .filter(|key| Some(key) != item.key.as_ref()),
                owner: (item.entry.kind, item.entry.folder.clone()),
            });
        }
        for mut job in jobs {
            self.use_extracted_page(&mut job);
            self.queue_thumbnail(ctx, job);
        }
        let thumbnails = &mut self.library.thumbnails;
        if thumbnails.textures.len() > TEXTURE_LIMIT {
            let mut used: Vec<_> = thumbnails
                .textures
                .iter()
                .map(|(key, (_, frame))| (*frame, key.clone()))
                .collect();
            used.sort_unstable();
            for (_, key) in used
                .into_iter()
                .take(thumbnails.textures.len() - TEXTURE_LIMIT)
            {
                thumbnails.textures.remove(&key);
            }
        }
    }

    /// Reads a page of the archive open in this window from its extracted
    /// file, which also works when the archive needs a password.
    fn use_extracted_page(&self, job: &mut Job) {
        if let Some((image, _)) = &mut job.source
            && let Some(file) = self.extracted_page(image)
        {
            *image = file;
        }
    }

    fn queue_thumbnail(&mut self, ctx: &egui::Context, job: Job) {
        let thumbnails = &mut self.library.thumbnails;
        if thumbnails.worker.is_none() {
            let Some(store) = &self.store else {
                return;
            };
            thumbnails.worker = Worker::start(
                store.thumbnail_dir(),
                ctx.clone(),
                u64::from(self.settings.disk_cache_mb) * 1_048_576,
            );
        }
        let Some(worker) = &mut thumbnails.worker else {
            return;
        };
        worker.limit.store(
            if self.settings.disk_cache_enabled {
                u64::from(self.settings.disk_cache_mb) * 1_048_576
            } else {
                0
            },
            std::sync::atomic::Ordering::Relaxed,
        );
        let shown = job.shown.clone();
        if worker.jobs.send(job).is_ok() {
            worker.pending += 1;
            thumbnails.requested.insert(shown);
        }
    }

    /// Shows finished thumbnails and records the files that were written.
    pub(super) fn poll_library_thumbnails(&mut self, ctx: &egui::Context) {
        let Some(worker) = &mut self.library.thumbnails.worker else {
            return;
        };
        let mut finished = Vec::new();
        while let Ok(done) = worker.done.try_recv() {
            worker.pending = worker.pending.saturating_sub(1);
            finished.push(done);
        }
        let idle = worker.pending == 0;
        for done in finished {
            let thumbnails = &mut self.library.thumbnails;
            thumbnails.requested.remove(&done.shown);
            let (pixels, written) = match done.result {
                Ok(result) => result,
                Err(_) => {
                    thumbnails.failed.insert(done.shown);
                    continue;
                }
            };
            let texture = ctx.load_texture(
                format!("library-thumbnail-{}", done.shown),
                pixels,
                egui::TextureOptions::LINEAR,
            );
            thumbnails
                .textures
                .insert(done.shown, (texture, self.frame));
            let Some(key) = written else {
                continue;
            };
            let (kind, folder) = done.owner;
            let item = self
                .library
                .items
                .iter_mut()
                .find(|item| item.entry.kind == kind && item.entry.folder == folder);
            if let Some(item) = &item
                && item.entry.thumbnail.as_deref() == Some(key.as_str())
            {
                continue;
            }
            if let Some(item) = item {
                item.entry.thumbnail = Some(key.clone());
            }
            if let Some(store) = &self.store
                && let Err(error) = store.set_thumbnail(kind, &folder, &key)
            {
                self.last_error = error;
            }
        }
        // Free the thread and the textures once nothing shows or waits for them.
        if idle && !self.library.open && self.library.dialog.is_none() {
            self.library.thumbnails = Thumbnails::default();
        }
    }

    pub(super) fn open_bookmark_dialog(&mut self) {
        #[cfg(any(target_os = "ios", target_os = "android"))]
        if self
            .mobile_selection
            .as_ref()
            .and_then(|selection| selection.source())
            .is_none()
        {
            self.last_error = "This provider did not grant lasting access. Select the source again to bookmark it.".into();
            return;
        }
        let Some(file) = self.files.get(self.focus_index).cloned() else {
            return;
        };
        // In an archive the bookmark is the archive, at this page inside it.
        let image = self.page_location(&file);
        let Some((folder, _)) = library::container(&image) else {
            return;
        };
        let Some(store) = &self.store else {
            self.last_error = "App state is unavailable; cannot save bookmarks.".into();
            return;
        };
        let existing = match store.bookmark_title(&folder) {
            Ok(title) => title,
            Err(error) => {
                self.last_error = error;
                return;
            }
        };
        self.library.dialog = Some(BookmarkDialog {
            name: existing
                .clone()
                .unwrap_or_else(|| library::folder_name(&folder)),
            preview: self.pictures.get(&file).map(|p| p.handle.clone()),
            folder,
            image,
            rename: false,
            replaces: existing.is_some(),
            focus: true,
            error: String::new(),
        });
        self.menu_open = false;
        self.zoom_input = None;
        self.release_held_input();
        self.canvas_drag = None;
    }

    fn open_rename_dialog(&mut self, index: usize) {
        let item = &self.library.items[index];
        self.library.dialog = Some(BookmarkDialog {
            folder: item.entry.folder.clone(),
            image: item.entry.image.clone().unwrap_or_default(),
            name: item.entry.title(),
            preview: item
                .shown()
                .and_then(|key| self.library.thumbnails.textures.get(key))
                .map(|(texture, _)| texture.clone()),
            rename: true,
            replaces: false,
            focus: true,
            error: String::new(),
        });
    }

    pub(super) fn bookmark_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.library.dialog.take() else {
            return;
        };
        if dialog.preview.is_none() && !dialog.rename {
            let file = self.extracted_page(&dialog.image);
            dialog.preview = self
                .pictures
                .get(file.as_ref().unwrap_or(&dialog.image))
                .map(|p| p.handle.clone());
        }
        // Consume Enter before the text field and buttons react to it.
        let mut submit = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        let mut cancel = false;
        let screen = ctx.viewport_rect();
        let width = (screen.width() - 48.).clamp(200., 360.);
        let preview_height = (screen.height() * 0.35).clamp(80., 200.);
        let modal = egui::Modal::new(egui::Id::unique("bookmark-dialog"))
            .frame(
                egui::Frame::NONE
                    .fill(overlay::background())
                    .corner_radius(12)
                    .inner_margin(20),
            )
            .backdrop_color(Color32::from_black_alpha(96))
            .show(ctx, |ui| {
                forms::style(ui);
                ui.set_width(width - 40.);
                ui.with_layout(crate::ui::direction::vertical(), |ui| {
                    ui.heading(tr(if dialog.rename {
                        "rename-bookmark"
                    } else {
                        "add-bookmark"
                    }));
                    let (_, rect) =
                        ui.allocate_space(egui::vec2(ui.available_width(), preview_height));
                    paint_thumbnail(ui, rect, dialog.preview.as_ref(), true);
                    let path = dialog.folder.display().to_string();
                    ui.add(egui::Label::new(RichText::new(&path).small().color(MUTED)).truncate())
                        .hover_text(&path);
                    if dialog.replaces {
                        ui.label(
                            RichText::new(tr("this-folder-is-already-bookmarked"))
                                .small()
                                .color(WARNING),
                        );
                    }
                    let response = ui.add_sized(
                        [ui.available_width(), forms::height()],
                        forms::text_edit(&mut dialog.name)
                            .id(egui::Id::unique(NAME_FIELD))
                            .hint_text(library::folder_name(&dialog.folder))
                            .char_limit(120),
                    );
                    if dialog.focus {
                        // Typing replaces the suggested title.
                        response.request_focus();
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(dialog.name.chars().count()),
                                )));
                            state.store(ui.ctx(), response.id);
                        }
                        dialog.focus = false;
                    }
                    if !dialog.error.is_empty() {
                        ui.label(
                            RichText::new(&dialog.error)
                                .small()
                                .color(Color32::LIGHT_RED),
                        );
                    }
                    crate::ui::direction::horizontal(ui, |ui| {
                        ui.with_layout(crate::ui::direction::trailing(), |ui| {
                            submit |= ui
                                .push_id("bookmark-ok", |ui| forms::button(ui, tr("ok")))
                                .inner
                                .clicked();
                            cancel = ui
                                .push_id("bookmark-cancel", |ui| forms::button(ui, tr("cancel")))
                                .inner
                                .clicked();
                        });
                    });
                });
            });
        self.release_held_input();
        if cancel || modal.should_close() {
            ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::unique(NAME_FIELD)));
            return;
        }
        if submit {
            self.save_bookmark(ctx, dialog);
        } else {
            self.library.dialog = Some(dialog);
        }
    }

    fn save_bookmark(&mut self, ctx: &egui::Context, mut dialog: BookmarkDialog) {
        let Some(store) = &self.store else {
            return;
        };
        let saved = if dialog.rename {
            store
                .rename_bookmark(&dialog.folder, &dialog.name)
                .map(|()| None)
        } else {
            store.add_bookmark(&dialog.image, &dialog.name).map(Some)
        };
        let image = match saved {
            Ok(image) => image,
            Err(error) => {
                dialog.error = error;
                self.library.dialog = Some(dialog);
                return;
            }
        };
        ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::unique(NAME_FIELD)));
        if let Some(image) = image {
            // Write the thumbnail now, from the image the user chose.
            if let (Some((folder, _)), Some(key)) =
                (library::container(&image), library::thumbnail_key(&image))
            {
                let mut job = Job {
                    shown: key.clone(),
                    source: Some((image.clone(), key)),
                    fallback: None,
                    owner: (Kind::Bookmark, folder),
                };
                self.use_extracted_page(&mut job);
                self.queue_thumbnail(ctx, job);
            }
            let title = library::clean_name(&dialog.name)
                .unwrap_or_else(|| library::folder_name(&dialog.folder));
            self.show_toast(ctx, crate::i18n::value_label("bookmark-added", &title));
        }
        if self.library.open {
            self.reload_library();
        }
    }
}

#[derive(Default)]
struct ListResponse {
    visible: Vec<usize>,
    opened: Option<usize>,
    removed: Option<usize>,
    renamed: Option<usize>,
}

fn entry_list(ui: &mut egui::Ui, library: &Library, kind: Kind) -> ListResponse {
    let mut response = ListResponse::default();
    egui::ScrollArea::vertical()
        .id_salt(("library-scroll", kind.code()))
        .max_height(ui.available_height().max(1.))
        .auto_shrink([false, false])
        .scroll_source(scroll_source(crate::app::is_mobile()))
        .show(ui, |ui| {
            let mut empty = true;
            for (index, item) in library.items.iter().enumerate() {
                if item.entry.kind != kind {
                    continue;
                }
                empty = false;
                let texture = item
                    .shown()
                    .and_then(|key| library.thumbnails.textures.get(key))
                    .map(|(texture, _)| texture);
                let row = entry_row(ui, &item.entry, texture);
                if row.visible {
                    response.visible.push(index);
                }
                if row.open {
                    response.opened = Some(index);
                }
                if row.remove {
                    response.removed = Some(index);
                }
                if row.rename {
                    response.renamed = Some(index);
                }
            }
            if empty {
                ui.add_space(12.);
                ui.add(
                    egui::Label::new(
                        RichText::new(tr(match kind {
                            Kind::Recent => "recently-opened-items-will-appear-here",
                            Kind::Bookmark => "bookmarked-folders-will-appear-here",
                        }))
                        .color(MUTED),
                    )
                    .wrap(),
                );
            }
        });
    response
}

struct RowResponse {
    visible: bool,
    open: bool,
    remove: bool,
    rename: bool,
}

fn entry_row(
    ui: &mut egui::Ui,
    entry: &Entry,
    texture: Option<&egui::TextureHandle>,
) -> RowResponse {
    let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), ROW_HEIGHT));
    let bookmark = entry.kind == Kind::Bookmark;
    let prefix = if bookmark { "bookmark" } else { "recent" };
    let remove_rect = Rect::from_center_size(
        egui::pos2(rect.right() - 22., rect.center().y),
        Vec2::splat(TOUCH_TARGET),
    );
    let rename_rect = remove_rect.translate(egui::vec2(-TOUCH_TARGET, 0.));
    let buttons_left = if bookmark {
        rename_rect.left()
    } else {
        remove_rect.left()
    };
    let open_rect = Rect::from_min_max(rect.min, egui::pos2(buttons_left - 4., rect.bottom()));
    let thumbnail_rect = Rect::from_min_size(
        open_rect.min + egui::vec2(6., (ROW_HEIGHT - THUMBNAIL_SIZE) / 2.),
        Vec2::splat(THUMBNAIL_SIZE),
    );
    let mirror = |inner| crate::ui::direction::mirror(inner, rect);
    let (remove_rect, rename_rect, open_rect, thumbnail_rect) = (
        mirror(remove_rect),
        mirror(rename_rect),
        mirror(open_rect),
        mirror(thumbnail_rect),
    );
    let path = entry.folder.display().to_string();
    let title = entry.title();
    let open = ui.interact(
        open_rect,
        egui::Id::unique((format!("{prefix}-folder"), &entry.folder)),
        egui::Sense::click(),
    );
    open.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &title));
    let remove_label = tr(if bookmark {
        "remove-from-bookmarks"
    } else {
        "remove-from-recent"
    });
    let remove = ui.interact(
        remove_rect,
        egui::Id::unique((format!("{prefix}-remove"), &entry.folder)),
        egui::Sense::click(),
    );
    remove.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{remove_label}: {title}"))
    });
    let rename = bookmark.then(|| {
        let rename = ui.interact(
            rename_rect,
            egui::Id::unique(("bookmark-rename", &entry.folder)),
            egui::Sense::click(),
        );
        rename.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                true,
                format!("{}: {title}", tr("rename-bookmark")),
            )
        });
        rename
    });
    let visible = ui.is_rect_visible(rect);
    if visible {
        highlight(ui, open_rect, &open);
        highlight(ui, remove_rect, &remove);
        if let Some(rename) = &rename {
            highlight(ui, rename_rect, rename);
            chrome_icons::paint(ui.painter(), rename_rect.center(), "rename", MUTED, false);
        }
        paint_thumbnail(ui, thumbnail_rect, texture, entry.available);
        let text_left = if crate::i18n::is_rtl() {
            open_rect.left() + 10.
        } else {
            thumbnail_rect.right() + 10.
        };
        let width = (open_rect.width() - THUMBNAIL_SIZE - 26.).max(1.);
        single_line(
            ui,
            &title,
            egui::pos2(text_left, open_rect.top() + 12.),
            width,
            16.,
            if entry.available { INK } else { MUTED },
        );
        let detail = if entry.available {
            path.clone()
        } else {
            format!("{} · {path}", tr("folder-unavailable"))
        };
        single_line(
            ui,
            &detail,
            egui::pos2(text_left, open_rect.top() + 38.),
            width,
            12.,
            if entry.available { MUTED } else { WARNING },
        );
        chrome_icons::paint(ui.painter(), remove_rect.center(), "trash", MUTED, false);
        ui.painter().hline(
            rect.x_range(),
            rect.bottom(),
            egui::Stroke::new(1_f32, Color32::from_white_alpha(18)),
        );
    }
    RowResponse {
        visible,
        open: open.hover_text(path).clicked(),
        remove: remove.hover_text(remove_label).clicked(),
        rename: rename.is_some_and(|rename| rename.hover_text(tr("rename-bookmark")).clicked()),
    }
}

/// Fits the thumbnail inside `rect`; a folder icon stands in until it loads.
fn paint_thumbnail(
    ui: &egui::Ui,
    rect: Rect,
    texture: Option<&egui::TextureHandle>,
    available: bool,
) {
    let painter = ui.painter();
    let Some(texture) = texture else {
        chrome_icons::paint(painter, rect.center(), "folder", MUTED, false);
        return;
    };
    let size = texture.size_vec2();
    if size.x <= 0. || size.y <= 0. {
        return;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    let tint = if available {
        Color32::WHITE
    } else {
        Color32::from_white_alpha(90)
    };
    painter.image(
        texture.id(),
        Rect::from_center_size(rect.center(), size * scale),
        uv(),
        tint,
    );
}

fn single_line(
    ui: &egui::Ui,
    text: &str,
    position: egui::Pos2,
    width: f32,
    size: f32,
    color: Color32,
) {
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        FontId::proportional(size),
        color,
        width.max(1.),
    );
    job.wrap.max_rows = 1;
    let galley = ui.painter().layout_job(job);
    let x = if crate::i18n::is_rtl() {
        position.x + width - galley.size().x
    } else {
        position.x
    };
    ui.painter()
        .galley(egui::pos2(x, position.y), galley, color);
}

fn highlight(ui: &egui::Ui, rect: Rect, response: &egui::Response) {
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 6., Color32::from_white_alpha(22));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.),
            5.,
            egui::Stroke::new(1_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
}
