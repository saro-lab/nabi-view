//! Zip and 7z archives open like folders, read-only. Their images are
//! extracted in the background; an encrypted archive asks for its password.
use super::*;
use crate::core::archive::{self, Error, Extracted};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// `Viewer::listing` while an archive is being opened. Media job ids count up
/// from one and never reach it.
pub(super) const LISTING: u64 = u64::MAX;
const PASSWORD_FIELD: &str = "archive-password";

pub(super) struct Archives {
    ctx: egui::Context,
    /// The archive shown in this window.
    open: Option<Opened>,
    job: Option<Job>,
    pub prompt: Option<Prompt>,
}

struct Opened {
    source: PathBuf,
    canonical: PathBuf,
    /// Path inside the archive for each extracted file.
    inner: HashMap<PathBuf, PathBuf>,
    /// Owns the extracted files; dropping it deletes them.
    _extracted: Extracted,
}

struct Job {
    source: PathBuf,
    /// Page to show first, as a path inside the archive.
    start: Option<PathBuf>,
    receiver: mpsc::Receiver<Result<Extracted, Error>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Asks for the password of an encrypted archive.
pub(super) struct Prompt {
    source: PathBuf,
    start: Option<PathBuf>,
    password: String,
    /// The previous password did not open the archive.
    wrong: bool,
    focus: bool,
}

impl Archives {
    pub(super) fn sort_name<'a>(&'a self, file: &'a Path) -> &'a Path {
        self.open
            .as_ref()
            .and_then(|open| open.inner.get(file))
            .map_or(file, PathBuf::as_path)
    }
    pub(super) fn new(ctx: egui::Context) -> Self {
        Self {
            ctx,
            open: None,
            job: None,
            prompt: None,
        }
    }

    /// Forgets the archive and deletes its extracted files.
    pub(super) fn close(&mut self) {
        self.job = None;
        self.prompt = None;
        self.open = None;
    }

    fn active(&self) -> bool {
        self.open.is_some() || self.job.is_some() || self.prompt.is_some()
    }

    fn start(&mut self, source: PathBuf, start: Option<PathBuf>, password: Option<String>) {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (path, cancelled, ctx) = (source.clone(), cancel.clone(), self.ctx.clone());
        let spawned = std::thread::Builder::new()
            .name("archive".into())
            .spawn(move || {
                let live = || !cancelled.load(Ordering::Relaxed);
                let result = archive::extract(&path, password.as_deref(), &live);
                // A cancelled job has no receiver; its files are deleted here.
                let _ = sender.send(result);
                ctx.request_repaint();
            });
        // A failed spawn drops the sender, which poll() reports.
        drop(spawned);
        self.job = Some(Job {
            source,
            start,
            receiver,
            cancel,
        });
    }
}

/// The archive to open for `path`: the archive itself, or the archive that
/// holds the page `path` names, with that page.
pub(super) fn target(path: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    if let Some((source, inner)) = archive::split(path) {
        return Some((source, Some(inner)));
    }
    (archive::is_archive(path) && path.is_file()).then(|| (path.to_owned(), None))
}

impl Viewer {
    /// True while this window shows, opens or asks the password of an archive.
    pub(super) fn in_archive(&self) -> bool {
        self.archives.active()
    }

    /// The archive file shown in this window.
    pub(super) fn archive_source(&self) -> Option<&Path> {
        self.archives
            .open
            .as_ref()
            .map(|open| open.source.as_path())
    }

    /// How the user and saved state name `file`: itself, or for an extracted
    /// page its path inside the archive (`book.zip/ch1/001.jpg`).
    pub(super) fn page_location(&self, file: &Path) -> PathBuf {
        let location = self
            .archives
            .open
            .as_ref()
            .and_then(|open| Some(open.source.join(open.inner.get(file)?)))
            .unwrap_or_else(|| file.to_owned());
        #[cfg(any(target_os = "ios", target_os = "android", test))]
        if let Some(logical) = self
            .mobile_selection
            .as_ref()
            .and_then(|s| s.logical_path(&location))
        {
            return logical;
        }
        location
    }

    /// The extracted file of a page of the open archive.
    pub(super) fn extracted_page(&self, location: &Path) -> Option<PathBuf> {
        let open = self.archives.open.as_ref()?;
        #[cfg(any(target_os = "ios", target_os = "android", test))]
        let local = self
            .mobile_selection
            .as_ref()
            .and_then(|s| s.local_path(location));
        #[cfg(any(target_os = "ios", target_os = "android", test))]
        let location = local.as_deref().unwrap_or(location);
        // Saved state spells the archive by its canonical path.
        let inner = location
            .strip_prefix(&open.source)
            .or_else(|_| location.strip_prefix(&open.canonical))
            .ok()?;
        open.inner
            .iter()
            .find(|(_, page)| page.as_path() == inner)
            .map(|(file, _)| file.clone())
    }

    /// A dialog that takes keyboard and pointer input from the viewer.
    pub(super) fn dialog_open(&self) -> bool {
        self.move_dialog.is_some() || self.rename_dialog.is_some() || self.archives.prompt.is_some()
    }

    /// Moving, deleting and restoring files does not apply inside an archive.
    pub(super) fn blocked_in_archive(&mut self, ctx: &egui::Context, id: &str) -> bool {
        let blocked =
            matches!(id, "move" | "normalize-names" | "trash" | "undo-trash") && self.in_archive();
        if blocked {
            self.show_toast(ctx, tr("not-available-in-archives"));
        }
        blocked
    }

    pub(super) fn open_archive(
        &mut self,
        source: PathBuf,
        start: Option<PathBuf>,
        password: Option<String>,
    ) {
        #[cfg(all(target_os = "macos", not(test)))]
        let access = match crate::platform::mac_access::resolve(&source) {
            Ok(access) => access,
            Err(error) => {
                self.last_error = error;
                return;
            }
        };
        #[cfg(all(target_os = "macos", not(test)))]
        let source = access.path.clone();
        if self.folder.as_ref() != Some(&source) {
            self.checked.clear();
        }
        self.reset_view();
        #[cfg(all(target_os = "macos", not(test)))]
        {
            self.mac_access = Some(access);
        }
        self.folder = Some(source.clone());
        // Recorded like a folder once its first image is painted.
        self.library.remember = Some(source.clone()).filter(|_| !crate::core::NATIVE_MOBILE);
        self.listing = Some(LISTING);
        self.folder_watch = None;
        self.archives.start(source, start, password);
    }

    /// Back to the start screen when an archive could not be opened.
    pub(super) fn leave_archive(&mut self) {
        self.archives.close();
        #[cfg(all(target_os = "macos", not(test)))]
        {
            self.mac_access = None;
        }
        self.listing = None;
        self.pending_steps.clear();
        self.folder = None;
        self.library.remember = None;
        self.library.open = true;
        self.reload_library();
    }

    pub(super) fn poll_archive(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.archives.job else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(Error::Failed("The archive reader stopped.".into()))
            }
        };
        let (source, start) = (std::mem::take(&mut job.source), job.start.take());
        self.archives.job = None;
        match result {
            Ok(extracted) => self.show_archive(ctx, source, start, extracted),
            Err(error @ (Error::PasswordRequired | Error::WrongPassword)) => {
                self.archives.prompt = Some(Prompt {
                    source,
                    start,
                    password: String::new(),
                    wrong: error == Error::WrongPassword,
                    focus: true,
                });
                self.menu_open = false;
                self.release_held_input();
                self.canvas_drag = None;
            }
            Err(Error::Cancelled) => {}
            Err(error) => {
                self.leave_archive();
                self.last_error = error.to_string();
            }
        }
    }

    fn show_archive(
        &mut self,
        ctx: &egui::Context,
        source: PathBuf,
        start: Option<PathBuf>,
        extracted: Extracted,
    ) {
        if extracted.pages.is_empty() {
            self.leave_archive();
            self.last_error = tr("there-are-no-images-in-this-archive").into();
            return;
        }
        let files: Vec<PathBuf> = extracted.pages.iter().map(|p| p.file.clone()).collect();
        self.sorting.stamps = extracted
            .pages
            .iter()
            .map(|page| (page.file.clone(), page.sort))
            .collect();
        let first = start
            .and_then(|start| extracted.pages.iter().find(|page| page.inner == start))
            .map(|page| page.file.clone());
        if extracted.skipped > 0 {
            self.last_error = format!(
                "{} images in the archive could not be opened.",
                extracted.skipped
            );
        }
        self.archives.open = Some(Opened {
            canonical: crate::core::paths::canonicalize(&source).unwrap_or_else(|_| source.clone()),
            source,
            inner: extracted
                .pages
                .iter()
                .map(|page| (page.file.clone(), page.inner.clone()))
                .collect(),
            _extracted: extracted,
        });
        self.listing = None;
        self.refresh_folder(files, &[]);
        // Random order shuffles the list, so look the saved page up afterwards.
        if let Some(index) = first.and_then(|first| self.files.iter().position(|f| *f == first)) {
            self.focus_index = index;
            self.span_cache.get_mut().take();
        }
        self.replay_pending_navigation(ctx);
    }

    pub(super) fn archive_window(&mut self, ctx: &egui::Context) {
        if self.archives.job.is_some() && self.files.is_empty() {
            // Large archives take a moment; say what the window is doing.
            egui::Area::new(egui::Id::unique("archive-opening"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.label(RichText::new(tr("opening-the-archive")).color(MUTED));
                });
        }
        let Some(mut prompt) = self.archives.prompt.take() else {
            return;
        };
        // Consume Enter before the text field and buttons react to it.
        let mut submit = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        let mut cancel = false;
        let width = (ctx.viewport_rect().width() - 48.).clamp(200., 360.);
        let modal = egui::Modal::new(egui::Id::unique("archive-password-dialog"))
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
                    ui.heading(tr("archive-password"));
                    let path = prompt.source.display().to_string();
                    ui.add(egui::Label::new(RichText::new(&path).small().color(MUTED)).truncate())
                        .hover_text(&path);
                    ui.label(tr("enter-the-password-to-open-this-archive"));
                    let response = ui.add_sized(
                        [ui.available_width(), forms::height()],
                        forms::text_edit(&mut prompt.password)
                            .id(egui::Id::unique(PASSWORD_FIELD))
                            .password(true),
                    );
                    if prompt.focus {
                        response.request_focus();
                        prompt.focus = false;
                    }
                    if response.changed() {
                        prompt.wrong = false;
                    }
                    if prompt.wrong {
                        ui.label(
                            RichText::new(tr("the-password-is-incorrect"))
                                .small()
                                .color(Color32::LIGHT_RED),
                        );
                    }
                    crate::ui::direction::horizontal(ui, |ui| {
                        ui.with_layout(crate::ui::direction::trailing(), |ui| {
                            submit |= ui
                                .push_id("archive-password-ok", |ui| forms::button(ui, tr("ok")))
                                .inner
                                .clicked();
                            cancel = ui
                                .push_id("archive-password-cancel", |ui| {
                                    forms::button(ui, tr("cancel"))
                                })
                                .inner
                                .clicked();
                        });
                    });
                });
            });
        self.release_held_input();
        if cancel || modal.should_close() {
            ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::unique(PASSWORD_FIELD)));
            self.leave_archive();
        } else if submit && !prompt.password.is_empty() {
            ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::unique(PASSWORD_FIELD)));
            self.archives
                .start(prompt.source, prompt.start, Some(prompt.password));
        } else {
            self.archives.prompt = Some(prompt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn open_archive_window(root: &Path) -> (Viewer, egui::Context, PathBuf) {
        let path = root.join("pages.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let image = std::fs::read(crate::app::sample_path().unwrap()).unwrap();
        for name in ["b/1.png", "a/2.png", "a/10.png"] {
            zip.start_file(name, options).unwrap();
            zip.write_all(&image).unwrap();
        }
        zip.finish().unwrap();
        let ctx = egui::Context::default();
        let store = Store::in_directories(root.join("config"), root.join("state"));
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            Some(path.clone()),
            Ok(store),
        );
        assert!(app.in_archive());
        assert_eq!(app.listing, Some(LISTING));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while app.listing.is_some() {
            assert!(std::time::Instant::now() < deadline, "archive did not open");
            std::thread::sleep(std::time::Duration::from_millis(5));
            app.poll_archive(&ctx);
        }
        (app, ctx, path)
    }

    #[test]
    fn pages_keep_their_archive_names_and_file_commands_are_refused() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let (mut app, ctx, path) = open_archive_window(root.path());
        let source = app.archive_source().unwrap().to_owned();
        assert_eq!(app.folder.as_ref(), Some(&source));
        let pages: Vec<PathBuf> = app.files.iter().map(|f| app.page_location(f)).collect();
        assert_eq!(
            pages,
            ["a/2.png", "a/10.png", "b/1.png"].map(|page| source.join(page))
        );
        assert_eq!(app.extracted_page(&pages[1]), Some(app.files[1].clone()));
        assert_eq!(
            target(&pages[1]),
            Some((path.clone(), Some("a/10.png".into())))
        );

        app.checked.insert(app.files[0].clone());
        for command in ["trash", "move", "undo-trash"] {
            app.toast = None;
            app.run(command, &ctx);
            let toast = app.toast.as_ref().expect(command);
            assert_eq!(toast.message, tr("not-available-in-archives"), "{command}");
            assert!(
                app.file_job.is_none() && app.move_dialog.is_none(),
                "{command}"
            );
        }
        assert!(app.files.iter().all(|file| file.is_file()));
        assert!(path.is_file());
    }

    #[test]
    fn leaving_an_archive_deletes_its_extracted_pages() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let (mut app, ctx, path) = open_archive_window(root.path());
        let extracted = app.files.clone();
        // A queued step waits for the archive like it waits for a folder.
        app.open(path.join("a").join("10.png"));
        app.run("next", &ctx);
        while app.listing.is_some() {
            std::thread::sleep(std::time::Duration::from_millis(5));
            app.poll_archive(&ctx);
        }
        assert_eq!(app.focus_index, 2);
        let reopened = app.files.clone();
        assert!(reopened.iter().all(|file| !extracted.contains(file)));

        let folder = root.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        let image = folder.join("only.png");
        std::fs::copy(crate::app::sample_path().unwrap(), &image).unwrap();
        app.open(image);
        assert!(!app.in_archive());
        app.toast = None;
        app.run("undo-trash", &ctx);
        assert_eq!(
            app.toast.as_ref().map(|toast| toast.message.as_str()),
            Some(tr("there-are-no-images-to-restore"))
        );
        // Deletion may finish on the cleanup thread while a decoder lets go.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while extracted.iter().chain(&reopened).any(|file| file.exists()) {
            assert!(
                std::time::Instant::now() < deadline,
                "extracted pages remain"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
