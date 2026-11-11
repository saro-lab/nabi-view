//! Connect bounded native imports to stable local library identities.
use super::*;
use crate::core::documents::ImportBatch;

impl Viewer {
    pub(super) fn accept_mobile_selection(&mut self, selection: ImportBatch) {
        let Some(path) = selection.initial_path() else {
            return;
        };
        if !path.starts_with(selection.directory()) {
            self.last_error = "Invalid imported document path.".into();
            return;
        }
        // open() records the previous page while its source mapping is alive.
        // Only after it resets the old view do we replace its owned copies.
        self.open(path);
        if let Some(notice) = selection.notice() {
            self.last_error = notice;
        }
        self.trash_history.clear();
        self.library.remember = selection.source().map(|source| source.root.clone());
        self.mobile_selection = Some(selection);
    }
}

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
    fn imported_folder_sort_uses_provider_time_after_listing_and_watch_update() {
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

        // A watcher rescan sees a changed staging file, but must still use the
        // provider's original metadata rather than the local copy timestamp.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&copy)
            .unwrap()
            .write_all(&[0])
            .unwrap();
        let watch = app.folder_watch.unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app
            .media
            .folder_stamps(watch)
            .get(&copy)
            .is_none_or(|stamp| stamp.bytes != image.len() as u64 + 1)
        {
            assert!(
                Instant::now() < deadline,
                "import folder watcher did not publish the change"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        app.collect(&ctx);
        assert_eq!(app.sorting.stamps[&copy].modified, None);
        assert_eq!(app.sorting.stamps[&copy].bytes, image.len() as u64);
        assert_eq!(names(&app), ["02-old.png", "01-new.png", "00-unknown.png"]);
        app.settings.sort_descending = true;
        app.sync_sort_settings();
        assert_eq!(names(&app), ["01-new.png", "02-old.png", "00-unknown.png"]);
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
