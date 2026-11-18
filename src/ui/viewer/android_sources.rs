//! Reuse only the currently owned MediaStore batch after native validation.
//! Tokens describe a complete foreground import, never a persisted OS grant.
use super::*;

impl Viewer {
    fn android_reuse_candidate(&self, path: &Path) -> Option<(u64, usize)> {
        if self.listing.is_some()
            || self.file_job.is_some()
            || self.in_archive()
            || self.files.is_empty()
        {
            return None;
        }
        let selection = self.mobile_selection.as_ref()?;
        let source = selection.source()?;
        let token = selection.android_reuse_token();
        if token == 0
            || !selection.is_folder()
            || self.folder.as_deref() != Some(selection.directory())
        {
            return None;
        }
        let focus = if path == source.root {
            self.focus_index
        } else {
            let local = selection.local_path(path)?;
            self.files
                .iter()
                .position(|candidate| candidate == &local)?
        };
        self.files.get(focus).filter(|file| file.is_file())?;
        Some((token, focus))
    }

    fn reuse_android_selection(&mut self, path: &Path, token: u64, epoch_valid: bool) -> bool {
        if !epoch_valid {
            return false;
        }
        let Some((current_token, focus)) = self.android_reuse_candidate(path) else {
            return false;
        };
        if current_token != token {
            return false;
        }
        if focus != self.focus_index {
            self.focus(focus);
        }
        // Same owned batch: retain decoded pages, thumbnails, zoom and checks.
        self.library.open = false;
        self.menu_open = false;
        self.last_error.clear();
        self.release_held_input();
        true
    }

    #[cfg(target_os = "android")]
    pub(super) fn open_android_source(&mut self, path: &Path) -> bool {
        let token = self.android_reuse_candidate(path).map_or(0, |c| c.0);
        crate::platform::mobile::reopen_android(path, token)
    }

    #[cfg(target_os = "android")]
    pub(super) fn poll_android_reuse(&mut self) {
        use crate::platform::mobile;
        let Some((path, token, epoch)) = mobile::take_android_reused() else {
            return;
        };
        // A completion from a replaced source or batch cannot reopen old state.
        if self.android_reuse_candidate(&path).map(|c| c.0) != Some(token) {
            return;
        }
        let valid = epoch != 0 && epoch == mobile::android_source_epoch();
        if !self.reuse_android_selection(&path, token, valid) {
            // Permission, lifecycle or observer changed after worker validation.
            // Only the still-owned request retries through the regular importer.
            mobile::reopen_android(&path, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::documents::ImportBatch;
    use std::time::{Duration, Instant};

    #[test]
    fn android_media_reuse_retains_owned_batch_and_pixels_with_current_token() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            None,
            Ok(Store::in_directories(
                root.path().join("config"),
                root.path().join("state"),
            )),
        );
        let mut selection = ImportBatch::new_folder(root.path()).unwrap();
        let image = std::fs::read(crate::app::sample_path().unwrap()).unwrap();
        selection.add("first.png", image.as_slice()).unwrap();
        selection.add("second.png", image.as_slice()).unwrap();
        selection
            .set_source(
                &root.path().join("document-sources"),
                "nabi-android-pictures",
                "Photos",
            )
            .unwrap();
        selection.commit_source().unwrap();
        selection.set_android_reuse_token(7);
        let source = selection.source().unwrap().root.clone();
        let directory = selection.directory().to_owned();
        app.accept_mobile_selection(selection);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.listing.is_some() || !app.pictures.contains_key(&directory.join("first.png")) {
            assert!(Instant::now() < deadline, "{}", app.last_error);
            app.collect(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        let texture = app.pictures[&directory.join("first.png")].handle.id();
        let files = app.files.clone();
        app.focus(1);
        app.zoom = 2.;
        app.pan = egui::vec2(11., 19.);
        app.checked.insert(directory.join("second.png"));
        app.library.open = true;
        assert!(app.reuse_android_selection(&source, 7, true));
        assert!(!app.library.open);
        assert_eq!(app.focus_index, 1);
        assert_eq!(app.zoom, 2.);
        assert_eq!(app.pan, egui::vec2(11., 19.));
        assert!(app.checked.contains(&directory.join("second.png")));
        assert_eq!(
            app.pictures[&directory.join("first.png")].handle.id(),
            texture
        );
        assert_eq!(
            app.mobile_selection.as_ref().unwrap().directory(),
            directory
        );
        assert_eq!(app.files, files);
        assert!(app.listing.is_none());
        assert!(app.reuse_android_selection(&source.join("first.png"), 7, true));
        assert_eq!(app.focus_index, 0);

        for (path, token, valid) in [
            (source.clone(), 7, false),
            (source.clone(), 8, true),
            (source.clone(), 0, true),
            (source.join("missing.png"), 7, true),
            (source.with_file_name("nabi-android-screenshots"), 7, true),
        ] {
            app.library.open = true;
            assert!(!app.reuse_android_selection(&path, token, valid));
            assert!(app.library.open);
        }
        // A fresh import of the same logical source still owns a different batch.
        app.mobile_selection
            .as_mut()
            .unwrap()
            .set_android_reuse_token(8);
        assert!(!app.reuse_android_selection(&source, 7, true));
        assert!(app.reuse_android_selection(&source, 8, true));
        // SAF, limited, partial and failed imports never receive a token.
        app.mobile_selection
            .as_mut()
            .unwrap()
            .set_android_reuse_token(0);
        assert!(app.android_reuse_candidate(&source).is_none());
    }
}
