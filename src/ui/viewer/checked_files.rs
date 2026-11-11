use super::*;
use crate::platform::trash::{RestoreBatchResult, TrashBatchResult, TrashEntry};
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) struct FileJob {
    receiver: Receiver<FileResult>,
    // A disconnected worker must not discard the last undo batch.
    undo_backup: Option<Vec<TrashEntry>>,
}

enum FileResult {
    Trashed(TrashBatchResult),
    Restored(RestoreBatchResult),
    Moved(crate::core::file_move::MoveResult),
}

impl Viewer {
    #[cfg(all(target_os = "macos", not(test)))]
    fn authorize_file_folders(
        &mut self,
        paths: &[PathBuf],
        destination: Option<&Path>,
    ) -> Option<Vec<crate::platform::mac_access::Access>> {
        if !crate::platform::mac_access::sandboxed() {
            return Some(Vec::new());
        }
        let folders: std::collections::BTreeSet<_> = paths
            .iter()
            .filter_map(|path| path.parent().map(Path::to_owned))
            .chain(destination.map(Path::to_owned))
            .collect();
        let mut scopes = Vec::new();
        for folder in folders {
            match crate::platform::mac_access::write_scope(&folder) {
                Ok(Some(scope)) => scopes.push(scope),
                Ok(None) => return None,
                Err(error) => {
                    self.last_error = error;
                    return None;
                }
            }
        }
        Some(scopes)
    }
    pub(super) fn replay_pending_navigation(&mut self, ctx: &egui::Context) {
        for (forward, check) in std::mem::take(&mut self.pending_steps) {
            if check {
                self.toggle_focused_checks(ctx);
            }
            self.move_page(ctx, forward);
        }
    }

    pub(super) fn toggle_focused_checks(&mut self, ctx: &egui::Context) {
        if self.random {
            self.show_toast(ctx, tr("checks-are-not-available-in-random-mode"));
            return;
        }
        if self.files.is_empty() {
            return;
        }
        let (first, last) = self.focus_span();
        let paths = &self.files[first..=last];
        let uncheck = paths.iter().all(|path| self.checked.contains(path));
        for path in paths {
            if uncheck {
                self.checked.remove(path);
            } else {
                self.checked.insert(path.clone());
            }
        }
        ctx.request_repaint();
    }

    pub(super) fn toggle_random(&mut self) {
        self.toast = None;
        self.span_cache.get_mut().take();
        let focused_path = self.files.get(self.focus_index).cloned();
        self.random = !self.random;
        if self.random {
            self.checked.clear();
            shuffle_paths(&mut self.files);
            self.focus(0);
        } else {
            let files = std::mem::take(&mut self.files);
            self.files = self.sorted_files(files);
            self.focus(
                self.files
                    .iter()
                    .position(|p| Some(p) == focused_path.as_ref())
                    .unwrap_or(0),
            );
        }
    }

    pub(super) fn random_confirmation(&mut self, ctx: &egui::Context) {
        if !self.random_prompt {
            return;
        }
        let response = egui::Modal::new(egui::Id::unique("random-clears-checks")).show(ctx, |ui| {
            ui.heading(tr("clear-checks-and-enable-random-order-confirm"));
            ui.label(tr("enabling-random-order-will-clear-all-checks"));
            ui.horizontal(|ui| {
                if ui.button(tr("cancel")).clicked() {
                    self.random_prompt = false;
                }
                if ui.button(tr("clear-checks-and-continue")).clicked() {
                    self.random_prompt = false;
                    self.toggle_random();
                    self.show_action_toast("random", ctx);
                }
            });
        });
        if response.should_close() {
            self.random_prompt = false;
        }
    }

    pub(super) fn paint_check(&self, ui: &mut egui::Ui, thumbnail: Rect, path: &Path) {
        let badge = Rect::from_min_size(
            thumbnail.right_top() + egui::vec2(-20., 2.),
            egui::vec2(18., 18.),
        );
        ui.painter().rect_filled(badge, 4., ACCENT);
        ui.painter().add(egui::Shape::line(
            vec![
                badge.min + egui::vec2(4., 9.),
                badge.min + egui::vec2(8., 13.),
                badge.min + egui::vec2(14., 5.),
            ],
            egui::Stroke::new(2_f32, BAR),
        ));
        ui.interact(
            badge,
            egui::Id::unique(("thumbnail-check", path)),
            egui::Sense::hover(),
        )
        .hover_text(tr("checked"));
    }

    pub(super) fn file_targets(&self) -> Vec<PathBuf> {
        let paths = if self.checked.is_empty() {
            if self.files.is_empty() {
                Vec::new()
            } else {
                let (first, last) = self.focus_span();
                self.files[first..=last].to_vec()
            }
        } else {
            // Stable list order makes the undo batch deterministic, including spreads.
            self.files
                .iter()
                .filter(|p| self.checked.contains(*p))
                .cloned()
                .collect()
        };
        #[cfg(any(target_os = "ios", target_os = "android", test))]
        let paths = paths
            .into_iter()
            .filter(|path| crate::core::documents::source_record(path).is_none())
            .collect();
        paths
    }

    /// Runs file work off the UI thread; `poll_file_job` collects its result.
    fn spawn_file_job(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        undo_backup: Option<Vec<TrashEntry>>,
        work: impl FnOnce() -> FileResult + Send + 'static,
    ) -> Result<(), String> {
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _ = sender.send(work());
                repaint.request_repaint();
            })
            .map_err(|error| error.to_string())?;
        self.file_job = Some(FileJob {
            receiver,
            undo_backup,
        });
        Ok(())
    }

    pub(super) fn trash_files(&mut self, ctx: &egui::Context) {
        if self.file_job.is_some() {
            self.show_toast(ctx, tr("a-file-operation-is-still-in-progress"));
            return;
        }
        let paths = self.file_targets();
        if paths.is_empty() {
            return;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        let Some(scopes) = self.authorize_file_folders(&paths, None) else {
            return;
        };
        match self.spawn_file_job(ctx, "trash-images", None, move || {
            #[cfg(all(target_os = "macos", not(test)))]
            let _scopes = scopes;
            FileResult::Trashed(crate::platform::trash::move_to_trash(paths))
        }) {
            Ok(()) if !REMOVES_FROM_LIST => {
                self.show_toast(ctx, tr("moving-images-to-trash-dialog"));
            }
            Ok(()) => {}
            Err(error) => self.last_error = error,
        }
    }

    pub(super) fn start_move(
        &mut self,
        paths: Vec<PathBuf>,
        destination: PathBuf,
        ctx: &egui::Context,
    ) {
        if self.file_job.is_some() || paths.is_empty() {
            return;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        let Some(scopes) = self.authorize_file_folders(&paths, Some(&destination)) else {
            return;
        };
        match self.spawn_file_job(ctx, "move-images", None, move || {
            #[cfg(all(target_os = "macos", not(test)))]
            let _scopes = scopes;
            FileResult::Moved(crate::core::file_move::move_files(paths, &destination))
        }) {
            Ok(()) => self.show_toast(ctx, tr("moving-images")),
            Err(error) => {
                if let Some(dialog) = &mut self.move_dialog {
                    dialog.error.clone_from(&error);
                }
                self.last_error = error;
            }
        }
    }

    pub(super) fn undo_trash(&mut self, ctx: &egui::Context) {
        if self.file_job.is_some() {
            self.show_toast(ctx, tr("a-file-operation-is-still-in-progress"));
            return;
        }
        // The batch leaves the history only once its worker has started.
        let Some(entries) = self.trash_history.last().cloned() else {
            self.show_toast(ctx, tr("there-are-no-images-to-restore"));
            return;
        };
        let backup = self.trash_history.last().cloned();
        #[cfg(all(target_os = "macos", not(test)))]
        let Some(scopes) = self.authorize_file_folders(
            &entries
                .iter()
                .map(|entry| entry.original_path.clone())
                .collect::<Vec<_>>(),
            None,
        ) else {
            return;
        };
        match self.spawn_file_job(ctx, "restore-images", backup, move || {
            #[cfg(all(target_os = "macos", not(test)))]
            let _scopes = scopes;
            FileResult::Restored(crate::platform::trash::restore_batch(entries))
        }) {
            Ok(()) => {
                self.trash_history.pop();
                if !REMOVES_FROM_LIST {
                    self.show_toast(ctx, tr("restoring-images-from-trash-dialog"));
                }
            }
            Err(error) => self.last_error = error,
        }
    }

    pub(super) fn poll_file_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.file_job else { return };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(TryRecvError::Disconnected) => {
                if let Some(entries) = self.file_job.take().and_then(|job| job.undo_backup) {
                    self.trash_history.push(entries);
                }
                self.close_after_file_job = false;
                self.closing_all = false;
                self.last_error = tr("the-file-operation-could-not-be-completed").into();
                if let Some(dialog) = &mut self.move_dialog {
                    dialog.error.clone_from(&self.last_error);
                }
                self.show_toast(ctx, self.last_error.clone());
                return;
            }
        };
        self.file_job = None;
        let (failed, message) = match result {
            FileResult::Trashed(result) => {
                let removed: HashSet<_> = result
                    .trashed
                    .iter()
                    .map(|entry| entry.original_path.clone())
                    .collect();
                let files = self
                    .files
                    .iter()
                    .filter(|p| !removed.contains(*p))
                    .cloned()
                    .collect();
                self.refresh_folder(files, &removed.into_iter().collect::<Vec<_>>());
                if !result.trashed.is_empty() {
                    self.trash_history.push(result.trashed);
                }
                (
                    result.failed,
                    if REMOVES_FROM_LIST {
                        "images-removed-from-list"
                    } else {
                        "images-moved-to-trash"
                    },
                )
            }
            FileResult::Moved(result) => {
                let removed: HashSet<_> = result.moved.iter().cloned().collect();
                let files = self
                    .files
                    .iter()
                    .filter(|p| !removed.contains(*p))
                    .cloned()
                    .collect();
                self.refresh_folder(files, &result.moved);
                if !result.moved.is_empty() {
                    self.record_move_destination(&result.destination);
                }
                if result.failed.is_empty() {
                    self.move_dialog = None;
                    ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::unique("move-path")));
                } else if let Some(dialog) = &mut self.move_dialog {
                    dialog.paths = result.failed.iter().map(|(path, _)| path.clone()).collect();
                    dialog.error = result
                        .failed
                        .iter()
                        .map(|(path, error)| format!("{}: {error}", path.display()))
                        .collect::<Vec<_>>()
                        .join("\n");
                }
                (result.failed, "images-moved")
            }
            FileResult::Restored(result) => {
                let mut files = self.files.clone();
                for path in &result.restored {
                    if path.parent() == self.folder.as_deref() && !files.contains(path) {
                        files.push(path.clone());
                    }
                }
                files.sort_by(|a, b| media::compare_file_names(a, b));
                self.refresh_folder(files, &result.restored);
                if !result.remaining.is_empty() {
                    self.trash_history.push(result.remaining);
                }
                (
                    result.failed,
                    if REMOVES_FROM_LIST {
                        "images-restored-to-list"
                    } else {
                        "images-restored-from-trash"
                    },
                )
            }
        };
        // Retire stale watcher snapshots taken before the operation completed.
        // Preserve the current checks/focus rather than reopening the folder.
        if let Some(folder) = &self.folder {
            self.listing = Some(self.media.request_folder(folder.clone()));
            self.folder_watch = self.listing;
        }
        if failed.is_empty() {
            self.show_toast(ctx, tr(message));
        } else {
            self.close_after_file_job = false;
            self.closing_all = false;
            self.last_error = failed
                .iter()
                .map(|(path, error)| format!("{}: {error}", path.display()))
                .collect::<Vec<_>>()
                .join("\n");
            self.show_toast(
                ctx,
                tr("some-images-could-not-be-processed-check-the-error-details"),
            );
        }
        if self.close_after_file_job {
            self.close_after_file_job = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hidden_window_defers_native_close_until_file_work_succeeds() {
        use eframe::App;

        for fail in [false, true] {
            for complete_before_close in [false, true] {
                let (mut app, ctx) = viewer();
                let (sender, receiver) = std::sync::mpsc::channel();
                app.file_job = Some(FileJob {
                    receiver,
                    undo_backup: None,
                });
                let mut result = TrashBatchResult::default();
                if fail {
                    result
                        .failed
                        .push(("image.png".into(), "test error".into()));
                }
                let mut result = Some(result);
                if complete_before_close {
                    sender
                        .send(FileResult::Trashed(result.take().unwrap()))
                        .unwrap();
                }
                let mut input = egui::RawInput {
                    focused: false,
                    ..Default::default()
                };
                let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
                viewport.minimized = Some(true);
                viewport.events.push(egui::ViewportEvent::Close);
                let mut output = ctx.run_logic(&input, |ctx| {
                    app.logic(ctx, &mut eframe::Frame::_new_kittest());
                });
                assert!(
                    output
                        .viewport_commands
                        .values()
                        .any(|commands| { commands.contains(&egui::ViewportCommand::CancelClose) })
                );
                if !complete_before_close {
                    assert!(app.close_after_file_job);
                    assert!(
                        !output
                            .viewport_commands
                            .values()
                            .any(|commands| { commands.contains(&egui::ViewportCommand::Close) })
                    );
                    sender
                        .send(FileResult::Trashed(result.take().unwrap()))
                        .unwrap();
                    input
                        .viewports
                        .get_mut(&egui::ViewportId::ROOT)
                        .unwrap()
                        .events
                        .clear();
                    output = ctx.run_logic(&input, |ctx| {
                        app.logic(ctx, &mut eframe::Frame::_new_kittest());
                    });
                }
                let closed = output
                    .viewport_commands
                    .values()
                    .any(|commands| commands.contains(&egui::ViewportCommand::Close));
                assert_eq!(closed, !fail);
                assert!(!app.close_after_file_job);
                assert!(app.file_job.is_none());
                assert!(!app.close_request_accepted);
                if fail {
                    assert!(app.last_error.contains("test error"));
                    assert!(!app.closing_all);
                }
            }
        }
    }

    #[test]
    fn close_all_waits_for_file_work_and_does_not_hide_failures() {
        for fail in [false, true] {
            let (mut app, ctx) = viewer();
            let (sender, receiver) = std::sync::mpsc::channel();
            app.file_job = Some(FileJob {
                receiver,
                undo_backup: None,
            });
            let output = ctx.run_pass(egui::RawInput::default(), |ctx| {
                app.request_graceful_close(ctx)
            });
            assert!(app.close_after_file_job);
            assert!(
                !output
                    .viewport_output
                    .values()
                    .any(|v| v.commands.contains(&egui::ViewportCommand::Close))
            );
            let mut result = TrashBatchResult::default();
            if fail {
                result
                    .failed
                    .push(("image.png".into(), "test error".into()));
            }
            sender.send(FileResult::Trashed(result)).unwrap();
            let output = ctx.run_pass(egui::RawInput::default(), |ctx| app.poll_file_job(ctx));
            let closed = output
                .viewport_output
                .values()
                .any(|v| v.commands.contains(&egui::ViewportCommand::Close));
            assert_eq!(closed, !fail);
            assert!(!app.close_after_file_job);
        }
    }

    use super::*;

    fn viewer() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            None,
            Err(String::new()),
        );
        viewer.files = vec!["1.png".into(), "2.png".into(), "3.png".into()];
        (viewer, ctx)
    }

    #[test]
    fn trash_targets_checked_files_or_current_spread_without_mutating_checks() {
        let (mut app, ctx) = viewer();
        assert_eq!(app.file_targets(), vec![PathBuf::from("1.png")]);
        app.settings.spread = true;
        assert_eq!(app.file_targets(), app.files[..2]);
        app.checked.insert(app.files[2].clone());
        assert_eq!(app.file_targets(), vec![app.files[2].clone()]);
        app.toggle_focused_checks(&ctx);
        assert_eq!(app.file_targets(), app.files);
        app.toggle_focused_checks(&ctx);
        assert_eq!(app.file_targets(), vec![app.files[2].clone()]);
        app.refresh_folder(vec!["1.png".into(), "2.png".into()], &[]);
        assert!(app.checked.is_empty());
        assert_eq!(app.file_targets(), app.files);
    }

    #[test]
    fn unlisted_check_shortcuts_are_not_implicitly_enabled() {
        for key in ["Space", "Shift+Left", "Shift+Right", "Delete", "Primary+Z"] {
            let settings = Settings {
                keyboard_shortcuts: Some(vec![KeyboardShortcut {
                    action: "fit".into(),
                    key: key.into(),
                }]),
                ..Default::default()
            };
            assert!(validate_keys(&settings).is_ok(), "{key}");
            for action in ["check", "check-prev", "check-next", "trash", "undo-trash"] {
                assert!(effective_keys(&settings, action).is_empty());
            }
        }
    }

    #[test]
    fn checks_before_deferred_navigation_keep_order_and_include_intermediate_pages() {
        let (mut app, ctx) = viewer();
        app.listing = Some(1);
        for command in ["check-next", "check-next", "check-prev"] {
            app.run(command, &ctx);
        }
        assert_eq!(app.focus_index, 0);
        app.listing = None;
        app.replay_pending_navigation(&ctx);
        assert_eq!(app.focus_index, 1);
        assert_eq!(app.checked.len(), 3);
    }

    #[test]
    fn deferred_check_toggles_match_immediate_navigation_in_both_directions() {
        for spread in [false, true] {
            let (mut immediate, ctx) = viewer();
            let (mut deferred, _) = viewer();
            for app in [&mut immediate, &mut deferred] {
                app.settings.spread = spread;
                app.checked.extend(app.files[..2].iter().cloned());
            }
            deferred.listing = Some(1);
            let before = deferred.checked.clone();
            for command in [
                "check-next",
                "check-prev",
                "check-next",
                "next",
                "check-prev",
            ] {
                immediate.run(command, &ctx);
                deferred.run(command, &ctx);
            }
            assert_eq!(
                deferred.checked, before,
                "queued input must not toggle early"
            );
            deferred.listing = None;
            deferred.replay_pending_navigation(&ctx);
            assert_eq!(deferred.focus_index, immediate.focus_index);
            assert_eq!(deferred.checked, immediate.checked);
            let after = deferred.checked.clone();
            deferred.replay_pending_navigation(&ctx);
            assert_eq!(
                deferred.checked, after,
                "queued input must run exactly once"
            );
        }
    }
}
