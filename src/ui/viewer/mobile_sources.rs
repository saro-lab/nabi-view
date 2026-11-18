//! Connect bounded native imports to stable local library identities.
use super::*;
use crate::core::documents::ImportBatch;

impl Viewer {
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    pub(super) fn mobile_import_loading(&self) -> bool {
        #[cfg(target_os = "ios")]
        if self.ios_photos_refreshing() {
            return false;
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        {
            crate::platform::mobile::import_pending()
                && !self.source_list_pending()
                && !self.library.opening_preview_active()
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        false
    }

    #[cfg(any(target_os = "ios", target_os = "android"))]
    pub(super) fn draw_mobile_import_loading(&mut self, root: &mut egui::Ui) {
        let mut cancel = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(21, 23, 25)))
            .show(root, |ui| {
                let rect = ui.ctx().content_rect().intersect(ui.max_rect());
                let center = rect.center();
                if !self.settings.reduce_motion {
                    ui.put(
                        Rect::from_center_size(center - egui::vec2(0., 33.), Vec2::splat(24.)),
                        egui::Spinner::new().color(ACCENT),
                    );
                }
                ui.put(
                    Rect::from_center_size(center, egui::vec2(rect.width(), 30.)),
                    egui::Label::new(tr("opening-image-dialog")),
                );
                cancel = ui
                    .put(
                        Rect::from_center_size(
                            center + egui::vec2(0., 48.),
                            egui::vec2(120., TOUCH_TARGET),
                        ),
                        egui::Button::new(tr("cancel")),
                    )
                    .clicked();
            });
        if cancel {
            self.open_mobile_home(root.ctx());
        }
    }

    pub(super) fn source_list_pending(&self) -> bool {
        self.mobile_selection
            .as_ref()
            .is_some_and(ImportBatch::is_seed)
    }
    /// Native metadata has resolved exactly one current page. The same import
    /// continues building its list, so do not scan partially populated files.
    #[cfg(any(target_os = "ios", test))]
    pub(super) fn accept_mobile_seed(&mut self, selection: ImportBatch) {
        if !selection.is_seed() || selection.files().len() != 1 {
            return;
        }
        let Some(path) = selection.initial_path() else {
            return;
        };
        if !path.starts_with(selection.directory()) {
            return;
        }
        self.remember_page();
        self.reset_view();
        self.folder = Some(selection.directory().to_owned());
        self.files = vec![path];
        self.sorting.stamps = selection.sort_metadata();
        self.listing = None;
        self.folder_watch = None;
        self.trash_history.clear();
        // A partial list must not overwrite saved source/recent metadata.
        self.library.remember = None;
        self.mobile_selection = Some(selection);
        self.library_preview_import_accepted();
        self.queue_images();
    }

    #[cfg(any(target_os = "ios", test))]
    pub(super) fn discard_mobile_seed(&mut self) {
        if self
            .mobile_selection
            .as_ref()
            .is_some_and(ImportBatch::is_seed)
        {
            self.cancel_library_preview();
            self.reset_view();
            self.folder = None;
            self.listing = None;
            self.folder_watch = None;
            self.mobile_selection = None;
            self.library.remember = None;
            self.library.open = true;
        }
    }

    #[cfg(target_os = "ios")]
    pub(super) fn poll_mobile_seed(&mut self) {
        if crate::platform::mobile::take_seed_cancelled() {
            self.discard_mobile_seed();
        }
        if let Some(seed) = crate::platform::mobile::take_seed() {
            self.accept_ios_seed(
                seed,
                crate::platform::mobile::photo_status(),
                crate::platform::mobile::access_revision(),
            );
        }
    }

    pub(super) fn accept_mobile_selection(&mut self, selection: ImportBatch) {
        let Some(path) = selection.initial_path() else {
            self.cancel_library_preview();
            return;
        };
        if !path.starts_with(selection.directory()) {
            self.last_error = "Invalid imported document path.".into();
            self.cancel_library_preview();
            return;
        }
        if self
            .mobile_selection
            .as_ref()
            .is_some_and(|current| current.is_seed() && current.same_import(&selection))
        {
            // Same files and provider capabilities: keep the current texture,
            // in-flight visible ticket, focus, zoom and navigation settings.
            self.sorting.stamps = selection.sort_metadata();
            self.refresh_folder(selection.files().to_vec(), &[]);
            let listing = self.media.request_folder(selection.directory().to_owned());
            // Imported copies change only through a new import, not on disk.
            self.media.stop_folder_watch(listing);
            self.listing = Some(listing);
            self.folder_watch = self.listing;
            self.library.remember = selection.source().map(|source| source.root.clone());
            if let Some(notice) = selection.notice() {
                self.last_error = notice;
            }
            self.mobile_selection = Some(selection);
            self.library_preview_import_accepted();
            self.queue_images();
            return;
        }
        // open() records the previous page while its source mapping is alive.
        // Only after it resets the old view do we replace its owned copies.
        self.open(path);
        if let Some(listing) = self.listing {
            self.media.stop_folder_watch(listing);
        }
        if let Some(notice) = selection.notice() {
            self.last_error = notice;
        }
        self.trash_history.clear();
        self.library.remember = selection.source().map(|source| source.root.clone());
        self.mobile_selection = Some(selection);
        self.library_preview_import_accepted();
    }
}

#[cfg(test)]
#[path = "mobile_seed_tests.rs"]
mod seed_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        time::{Duration, Instant},
    };

    fn window(root: &Path) -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            None,
            Ok(Store::in_directories(
                root.join("config"),
                root.join("state"),
            )),
        );
        (app, ctx)
    }

    fn wait(app: &mut Viewer, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.listing.is_some() {
            assert!(
                Instant::now() < deadline,
                "document import did not open: {}",
                app.last_error
            );
            app.poll_archive(ctx);
            app.collect(ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn mobile_library_waits_for_native_open_without_flashing_empty_menu() {
        use eframe::App;

        struct Preview;
        impl Drop for Preview {
            fn drop(&mut self) {
                crate::ui::mobile::set_preview(false);
            }
        }
        crate::ui::mobile::set_preview(true);
        let _preview = Preview;

        fn draw(app: &mut Viewer, ctx: &egui::Context, time: &mut f64, events: Vec<egui::Event>) {
            *time += 1. / 60.;
            let mut input = egui::RawInput {
                time: Some(*time),
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(390., 844.),
                )),
                focused: true,
                events,
                ..Default::default()
            };
            app.raw_input_hook(ctx, &mut input);
            let mut frame = eframe::Frame::_new_kittest();
            let _ = ctx.run_ui(input, |ui| {
                app.logic(ui.ctx(), &mut frame);
                app.ui(ui, &mut frame);
            });
        }

        fn click(app: &mut Viewer, ctx: &egui::Context, time: &mut f64, pos: egui::Pos2) {
            draw(app, ctx, time, vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                draw(
                    app,
                    ctx,
                    time,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
        }

        for collection in [
            crate::core::photo_library::IOS[0],
            crate::core::photo_library::ANDROID[1],
        ] {
            for complete in [false, true] {
                let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
                let store =
                    Store::in_directories(root.path().join("config"), root.path().join("state"));
                store
                    .initialize_photo_bookmarks(
                        &root.path().join("document-sources"),
                        &[collection],
                    )
                    .unwrap();
                let source = store.library().unwrap()[0].folder.clone();
                let ctx = egui::Context::default();
                let mut app = Viewer::new(
                    &eframe::CreationContext::_new_kittest(ctx.clone()),
                    None,
                    Ok(store),
                );
                app.deferred_mobile_open_requests = Some(Vec::new());
                let row = egui::Id::unique(("bookmark-folder".to_owned(), &source));
                let menu = egui::Id::unique(("menu-command", "open"));
                let mut time = 0.;
                for _ in 0..4 {
                    draw(&mut app, &ctx, &mut time, Vec::new());
                }
                let position = ctx.read_response(row).unwrap().rect.center();
                click(&mut app, &ctx, &mut time, position);
                assert_eq!(
                    app.deferred_mobile_open_requests.as_deref(),
                    Some([source].as_slice()),
                    "The real bookmark click must start the deferred native request"
                );
                // The native provider deliberately has not published a result.
                // Observe every intervening frame, not just the final screen.
                for frame in 0..20 {
                    draw(&mut app, &ctx, &mut time, Vec::new());
                    assert!(ctx.read_response(menu).is_none(), "menu flashed at {frame}");
                    assert!(
                        ctx.read_response(row).is_some(),
                        "library vanished at {frame}"
                    );
                }
                // An empty/cancelled selection must leave the library intact.
                app.accept_mobile_selection(ImportBatch::new(root.path()).unwrap());
                draw(&mut app, &ctx, &mut time, Vec::new());
                assert!(ctx.read_response(menu).is_none());
                assert!(ctx.read_response(row).is_some());

                if complete {
                    let mut selection = ImportBatch::new_folder(root.path()).unwrap();
                    let image = std::fs::read(crate::app::sample_path().unwrap()).unwrap();
                    selection.add("page.png", image.as_slice()).unwrap();
                    let path = selection.first().unwrap().to_owned();
                    app.accept_mobile_selection(selection);
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while !app.pictures.contains_key(&path) {
                        assert!(Instant::now() < deadline, "{}", app.last_error);
                        draw(&mut app, &ctx, &mut time, Vec::new());
                        assert!(ctx.read_response(menu).is_none());
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    draw(&mut app, &ctx, &mut time, Vec::new());
                    assert!(!app.library.open);
                    assert!(ctx.read_response(row).is_none());
                    assert_eq!(app.files, [path]);
                } else {
                    let close = ctx
                        .read_response(egui::Id::unique(("titlebar", "library-close")))
                        .unwrap();
                    click(&mut app, &ctx, &mut time, close.rect.center());
                    for _ in 0..4 {
                        draw(&mut app, &ctx, &mut time, Vec::new());
                    }
                    assert!(ctx.read_response(menu).is_some());
                    assert!(ctx.read_response(row).is_none());
                }
            }
        }
    }

    #[test]
    fn deferred_folder_opens_before_originals_and_only_reads_requested_pages() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let image = Arc::new(std::fs::read(crate::app::sample_path().unwrap()).unwrap());
        let mut selection = ImportBatch::new_folder(root.path()).unwrap();
        let release = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        for index in 0..20 {
            let (image, release, calls) = (image.clone(), release.clone(), calls.clone());
            selection
                .add_deferred(
                    &format!("page{index:02}.png"),
                    image.len() as u64,
                    None,
                    Arc::new(move |file, _, live| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        let deadline = Instant::now() + Duration::from_secs(10);
                        while !release.load(Ordering::Acquire) {
                            if !live() || Instant::now() > deadline {
                                return Err("Cancelled".into());
                            }
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        file.write_all(&image).map_err(|error| error.to_string())
                    }),
                )
                .unwrap();
        }
        let first = selection.first().unwrap().to_owned();
        let key = crate::core::library::thumbnail_key(&first).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let (mut app, ctx) = window(root.path());
        app.accept_mobile_selection(selection);
        wait(&mut app, &ctx);
        // The complete navigable list is available even while the provider
        // deliberately withholds every original's bytes.
        assert_eq!(app.files.len(), 20);
        assert_eq!(std::fs::metadata(&first).unwrap().len(), 0);
        assert!(app.pictures.is_empty());
        release.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app.pictures.contains_key(&first) {
            assert!(Instant::now() < deadline, "{}", app.last_error);
            app.collect(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(crate::core::library::thumbnail_key(&first), Some(key));
        // Metadata receives the real original, while untouched pages still
        // have no copied pixels and the user's source remains read-only.
        assert!(crate::core::metadata::read(&first).is_ok());
        assert_eq!(std::fs::metadata(&app.files[19]).unwrap().len(), 0);
    }

    #[test]
    fn deferred_sizes_reorder_after_reading_and_survive_folder_refresh() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let image = std::fs::read(crate::app::sample_path().unwrap()).unwrap();
        let release = Arc::new(AtomicBool::new(false));
        let mut selection = ImportBatch::new_folder(root.path()).unwrap();
        for (name, padding) in [("large.png", 256), ("small.png", 32), ("unread.png", 0)] {
            let mut bytes = image.clone();
            bytes.resize(bytes.len() + padding, 0);
            let release = release.clone();
            selection
                .add_deferred(
                    name,
                    0,
                    None,
                    Arc::new(move |file, _, live| {
                        let deadline = Instant::now() + Duration::from_secs(10);
                        while !release.load(Ordering::Acquire) {
                            if !live() || Instant::now() > deadline {
                                return Err("Cancelled".into());
                            }
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        file.write_all(&bytes).map_err(|error| error.to_string())
                    }),
                )
                .unwrap();
        }
        let large = selection.first().unwrap().to_owned();
        let small = selection.directory().join("small.png");
        let unread = selection.directory().join("unread.png");
        let (mut app, ctx) = window(root.path());
        app.settings.sort_by = "size".into();
        app.accept_mobile_selection(selection);
        wait(&mut app, &ctx);
        assert_eq!(app.files, [large.clone(), small.clone(), unread.clone()]);
        assert_eq!(app.sorting.stamps[&large].bytes, 0);
        assert_eq!(crate::core::documents::deferred_size(&large), None);

        release.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app.pictures.contains_key(&large) {
            assert!(Instant::now() < deadline, "{}", app.last_error);
            app.collect(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.sorting.stamps[&large].bytes, image.len() as u64 + 256);
        app.zoom = 2.0;
        app.pan = egui::vec2(12., 34.);
        app.touch_gesture.start(egui::pos2(200., 100.));
        let id = app.media.request_thumbnail(small.clone(), 192);
        app.thumb_ticket = Some((id, small.clone()));
        while app.sorting.stamps[&small].bytes == 0 {
            assert!(
                Instant::now() < deadline,
                "thumbnail did not reveal its size"
            );
            app.collect(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        // Background results must not cancel a finger drag or change its
        // next-page ordering. Reorder once idle, even without another event.
        assert_eq!(app.files, [large.clone(), small.clone(), unread.clone()]);
        assert!(app.touch_gesture.active());
        assert!(app.sorting.deferred_size_pending);
        assert_eq!(
            app.touch_gesture.finish(egui::pos2(100., 100.), false),
            Some(true)
        );
        app.collect(&ctx);
        assert_eq!(app.files, [small.clone(), large.clone(), unread.clone()]);
        assert_eq!(app.files[app.focus_index], large);
        assert_eq!(app.zoom, 2.0);
        assert_eq!(app.pan, egui::vec2(12., 34.));
        app.settings.sort_descending = true;
        app.sync_sort_settings();
        assert_eq!(app.files, [large.clone(), small.clone(), unread.clone()]);
        assert_eq!(app.files[app.focus_index], large);

        // A new listing still exposes frozen provider stamps, then the import
        // overlay restores exact known sizes without opening any more originals.
        app.listing = Some(app.media.request_folder(app.folder.clone().unwrap()));
        app.folder_watch = app.listing;
        wait(&mut app, &ctx);
        assert_eq!(app.sorting.stamps[&large].bytes, image.len() as u64 + 256);
        assert_eq!(app.sorting.stamps[&small].bytes, image.len() as u64 + 32);
        assert_eq!(app.sorting.stamps[&unread].bytes, 0);
        assert_eq!(
            crate::core::documents::deferred_stamp(&large),
            Some((0, None))
        );
        assert_eq!(crate::core::documents::deferred_size(&unread), None);
        assert_eq!(app.files, [large.clone(), small, unread]);
        assert_eq!(app.files[app.focus_index], large);
        app.touch_gesture.start(egui::pos2(200., 100.));
        app.resort_preserving_focus();
        assert!(
            app.touch_gesture.active(),
            "unchanged order must preserve input"
        );
    }

    #[test]
    fn imported_folder_sort_uses_provider_time_and_does_not_poll_staging_copies() {
        use std::time::SystemTime;
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let image = std::fs::read(crate::app::sample_path().unwrap()).unwrap();
        let mut selection = ImportBatch::new_folder(root.path()).unwrap();
        for (name, age) in [
            ("00-unknown.png", None),
            ("01-new.png", Some(20)),
            ("02-old.png", Some(10)),
        ] {
            selection
                .add_with_modified(
                    name,
                    image.as_slice(),
                    age.map(|value| SystemTime::UNIX_EPOCH + Duration::from_secs(value)),
                )
                .unwrap();
        }
        let copy = selection.first().unwrap().to_owned();
        let (mut app, ctx) = window(root.path());
        app.settings.sort_by = "modified".into();
        app.accept_mobile_selection(selection);
        wait(&mut app, &ctx);
        let names = |app: &Viewer| {
            app.files
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&app), ["02-old.png", "01-new.png", "00-unknown.png"]);
        assert_eq!(app.sorting.stamps[&copy].modified, None);

        // App-owned copies are listed once, not polled: a local change to a
        // staging file publishes nothing, and sorting keeps provider metadata.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&copy)
            .unwrap()
            .write_all(&[0])
            .unwrap();
        let watch = app.folder_watch.unwrap();
        // Several poll intervals (350 ms each).
        std::thread::sleep(Duration::from_millis(1200));
        assert_eq!(
            app.media.folder_stamps(watch)[&copy].bytes,
            image.len() as u64,
            "an imported copy must not be rescanned"
        );
        app.collect(&ctx);
        assert_eq!(app.sorting.stamps[&copy].modified, None);
        assert_eq!(app.sorting.stamps[&copy].bytes, image.len() as u64);
        assert_eq!(names(&app), ["02-old.png", "01-new.png", "00-unknown.png"]);
        app.settings.sort_descending = true;
        app.sync_sort_settings();
        assert_eq!(names(&app), ["01-new.png", "02-old.png", "00-unknown.png"]);
    }

    #[test]
    fn selected_mobile_image_stays_current_when_its_folder_is_added_and_sorted() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let image = std::fs::read(crate::app::sample_path().unwrap()).unwrap();
        let sources = root.path().join("document-sources");
        let mut selection = ImportBatch::new_folder(root.path()).unwrap();
        // The selected image arrives first, then its accessible neighbors.
        for name in ["page10.png", "page2.png", "page20.png"] {
            selection.add(name, image.as_slice()).unwrap();
        }
        selection
            .set_source(&sources, "12345678-photo-folder", "Download/Book/")
            .unwrap();
        selection.commit_source().unwrap();
        let source = selection.source().unwrap().root.clone();
        let (mut app, ctx) = window(root.path());
        app.accept_mobile_selection(selection);
        wait(&mut app, &ctx);
        assert_eq!(app.files.len(), 3);
        assert_eq!(app.focus_index, 1);
        assert_eq!(
            app.page_location(&app.files[app.focus_index]),
            source.join("page10.png")
        );
        assert_eq!(app.files[0].file_name().unwrap(), "page2.png");
        assert_eq!(app.files[2].file_name().unwrap(), "page20.png");
        // A later reopen still starts at the saved page rather than the first
        // item returned by MediaStore's new query order.
        let mut reopened = ImportBatch::new_folder(root.path()).unwrap();
        for name in ["page2.png", "page10.png", "page20.png"] {
            reopened.add(name, image.as_slice()).unwrap();
        }
        reopened
            .set_source(&sources, "12345678-photo-folder", "Download/Book/")
            .unwrap();
        reopened.request_page(Some(source.join("page10.png")));
        reopened.commit_source().unwrap();
        app.accept_mobile_selection(reopened);
        wait(&mut app, &ctx);
        assert_eq!(app.focus_index, 1);
        assert_eq!(
            app.page_location(&app.files[app.focus_index]),
            source.join("page10.png")
        );
    }

    #[test]
    fn imported_images_record_stable_sources_and_keep_bookmarks_after_copy_cleanup() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let image = std::fs::read(crate::app::sample_path().unwrap()).unwrap();
        let mut selection = ImportBatch::new(root.path()).unwrap();
        selection.add("01.png", image.as_slice()).unwrap();
        selection.add("02.png", image.as_slice()).unwrap();
        selection
            .set_source(
                &root.path().join("document-sources"),
                "12345678-images",
                "My photos",
            )
            .unwrap();
        selection.commit_source().unwrap();
        let source = selection.source().unwrap().root.clone();
        let copy = selection.first().unwrap().to_owned();
        let (mut app, ctx) = window(root.path());
        app.accept_mobile_selection(selection);
        wait(&mut app, &ctx);
        assert_eq!(app.files.len(), 2);
        assert_eq!(app.library.remember, Some(source.clone()));
        app.focus_index = 1;
        let logical = app.page_location(&app.files[1]);
        assert_eq!(logical, source.join("02.png"));
        let store = app.store.as_ref().unwrap();
        store.record_folder(&source, 50).unwrap();
        app.library.remember = None;
        app.remember_page();
        app.open_bookmark_dialog();
        assert!(app.library.dialog.is_some());
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600., 800.),
            )),
            events: vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ = ctx.run_pass(input, |ctx| app.bookmark_window(ctx));
        assert!(app.library.dialog.is_none(), "{}", app.last_error);
        let entries = app.store.as_ref().unwrap().library().unwrap();
        assert_eq!(entries[0].image, Some(logical.clone()));
        assert!(
            entries
                .iter()
                .any(|entry| entry.kind == crate::core::library::Kind::Bookmark
                    && entry.folder == source
                    && entry.image.as_ref() == Some(&logical))
        );
        // Defensive file targeting refuses marker paths even if one is injected.
        app.files = vec![logical];
        app.focus_index = 0;
        assert!(app.file_targets().is_empty());
        drop(app);
        assert!(!copy.exists());
        let store = Store::in_directories(root.path().join("config"), root.path().join("state"));
        assert_eq!(store.library().unwrap()[0].folder, source);
    }

    #[test]
    fn mobile_archive_reopens_internal_page_and_never_targets_original_for_file_work() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let image = std::fs::read(crate::app::sample_path().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in ["chapter/01.png", "chapter/02.png"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&image).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let sources = root.path().join("document-sources");
        let mut selection = ImportBatch::new(root.path()).unwrap();
        selection.add("book.zip", bytes.as_slice()).unwrap();
        selection
            .set_source(&sources, "12345678-archive", "Book")
            .unwrap();
        let source = selection.source().unwrap().root.clone();
        let logical = source.join("book.zip/chapter/02.png");
        selection.request_page(Some(logical.clone()));
        selection.commit_source().unwrap();
        let (mut app, ctx) = window(root.path());
        app.accept_mobile_selection(selection);
        wait(&mut app, &ctx);
        assert!(app.in_archive());
        assert_eq!(app.page_location(&app.files[app.focus_index]), logical);
        assert_eq!(
            app.extracted_page(&logical),
            Some(app.files[app.focus_index].clone())
        );
        assert_eq!(app.library.remember, Some(source));
        for command in ["trash", "move", "undo-trash"] {
            assert!(app.blocked_in_archive(&ctx, command));
        }
        assert!(app.file_job.is_none());
    }
}
