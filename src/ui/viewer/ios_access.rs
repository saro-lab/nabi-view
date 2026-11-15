//! Recover a selected source without changing the user's authorization choices.
use super::*;
use crate::core::ios_access::{AccessIssue, PhotoStatus};

#[derive(Clone)]
pub(super) struct Dialog {
    source: Option<PathBuf>,
    photo: bool,
    issue: Option<AccessIssue>,
    action_failed: bool,
}

#[derive(Clone)]
struct Waiting {
    source: Option<PathBuf>,
    status: PhotoStatus,
    action: i32,
    photo: bool,
}

#[derive(Default)]
pub(super) struct AccessUi {
    pub(super) dialog: Option<Dialog>,
    waiting: Option<Waiting>,
    deferred: Option<PathBuf>,
    refresh_pending: bool,
    status: PhotoStatus,
}

impl Waiting {
    fn retry(&self, status: PhotoStatus, selection_changed: bool) -> bool {
        if !self.photo {
            return self.source.is_some() && self.action == 1;
        }
        self.source.is_some()
            && status.readable()
            && (status != self.status || (self.action == 2 && selection_changed))
    }
}

fn actions(photo: bool, status: PhotoStatus, has_source: bool) -> Vec<(i32, &'static str)> {
    if !photo {
        if !has_source {
            return vec![(5, "open"), (6, "open-folder")];
        }
        return vec![
            (0, "access-retry"),
            (3, "source-reselect"),
            (1, "access-open-settings"),
        ];
    }
    match status {
        PhotoStatus::NotDetermined => vec![(4, "photo-access-allow")],
        PhotoStatus::Denied => vec![(1, "access-open-settings")],
        PhotoStatus::Restricted => Vec::new(),
        PhotoStatus::Limited => {
            let mut result = vec![(2, "photo-selection-change"), (1, "access-open-settings")];
            if has_source {
                result.push((0, "access-retry"));
            }
            result
        }
        PhotoStatus::Authorized => {
            let mut result = vec![(1, "access-open-settings")];
            if has_source {
                result.insert(0, (0, "access-retry"));
            }
            result
        }
    }
}

fn photo_access_controls(ui: &mut egui::Ui, status: PhotoStatus) -> Option<i32> {
    ui.heading(tr("ios-photo-access"));
    ui.label(tr(status.message()));
    let mut selected = None;
    let actions = actions(true, status, false);
    if !actions.is_empty() {
        // Settings lives in a vertical scroll area. Reserve only a button row,
        // allowing wrapped rows to grow instead of centering in the remaining height.
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), forms::height()),
            crate::ui::direction::flow().with_main_wrap(true),
            |ui| {
                for (action, key) in actions {
                    if forms::button(ui, tr(key)).clicked() {
                        selected = Some(action);
                    }
                }
            },
        );
    }
    selected
}

impl Viewer {
    pub(super) fn ios_access_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.ios_access.dialog.clone() else {
            return;
        };
        let status = self.ios_access.status;
        let mut selected = None;
        let mut close = false;
        let max = (ctx.content_rect().size() - Vec2::splat(32.)).max(Vec2::splat(1.));
        let modal = egui::Modal::new(egui::Id::unique("ios-access-recovery"))
            .frame(
                egui::Frame::NONE
                    .fill(overlay::background())
                    .inner_margin(16)
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                forms::style(ui);
                ui.set_width(max.x.min(420.));
                egui::ScrollArea::vertical()
                    .max_height(max.y)
                    .show(ui, |ui| {
                        ui.heading(tr(if dialog.photo {
                            "ios-photo-access"
                        } else {
                            "access-reconnect-title"
                        }));
                        ui.add_space(12.);
                        let message = dialog.issue.map(AccessIssue::message).unwrap_or_else(|| {
                            if dialog.photo {
                                status.message()
                            } else {
                                "source-read-failed"
                            }
                        });
                        ui.label(tr(message));
                        if dialog.action_failed {
                            ui.colored_label(WARNING, tr("access-action-failed"));
                        }
                        ui.add_space(12.);
                        for (action, key) in actions(dialog.photo, status, dialog.source.is_some())
                        {
                            if ui
                                .add_sized(
                                    [ui.available_width(), forms::height()],
                                    egui::Button::new(tr(key)),
                                )
                                .clicked()
                            {
                                selected = Some(action);
                            }
                        }
                        close = ui
                            .add_sized(
                                [ui.available_width(), forms::height()],
                                egui::Button::new(tr("access-close")),
                            )
                            .clicked();
                    });
            });
        if close || modal.should_close() {
            #[cfg(target_os = "ios")]
            self.cancel_ios_recovery();
            #[cfg(not(target_os = "ios"))]
            {
                self.ios_access = Default::default();
            }
        } else if let Some(action) = selected {
            #[cfg(target_os = "ios")]
            self.perform_ios_access(action, dialog.source);
            #[cfg(not(target_os = "ios"))]
            let _ = action;
        }
        self.release_held_input();
    }
}

#[cfg(target_os = "ios")]
impl Viewer {
    pub(super) fn open_ios_source(&mut self, path: &Path) -> bool {
        use crate::platform::mobile;
        let Some((_, id)) = crate::core::documents::source_identity(path) else {
            return false;
        };
        let Some(resolved) = mobile::recovery_source(path) else {
            return false;
        };
        let path = resolved.as_path();
        self.ios_access.waiting = None;
        self.ios_access.dialog = None;
        self.ios_access.deferred = None;
        if mobile::access_import_busy() {
            mobile::cancel_access_import();
            self.ios_access.deferred = Some(path.to_owned());
            return true;
        }
        mobile::cancel_access_import();
        let photo = crate::core::photo_library::is_ios(&id);
        self.ios_access.status = mobile::photo_status();
        let missing = crate::core::documents::source_record(path).is_none();
        // The first explicit photo tap goes straight to PhotoKit's permission
        // sheet. Only a previous denial or system restriction needs recovery UI.
        if (photo
            && matches!(
                self.ios_access.status,
                PhotoStatus::Denied | PhotoStatus::Restricted
            ))
            || missing
        {
            self.ios_access.dialog = Some(Dialog {
                source: Some(path.to_owned()),
                photo,
                issue: missing.then_some(AccessIssue::Reconnect),
                action_failed: false,
            });
        } else {
            self.start_ios_source(path.to_owned());
        }
        true
    }

    fn start_ios_source(&mut self, path: PathBuf) {
        self.ios_access.dialog = None;
        self.last_error.clear();
        if crate::platform::mobile::reopen(&path) {
            self.library.open = false;
        } else {
            self.ios_access.dialog = Some(Dialog {
                photo: crate::core::photo_library::title(&path).is_some(),
                source: Some(path),
                issue: Some(AccessIssue::Reconnect),
                action_failed: false,
            });
        }
    }

    pub(super) fn cancel_ios_recovery(&mut self) {
        self.ios_access.waiting = None;
        self.ios_access.dialog = None;
        self.ios_access.deferred = None;
        crate::platform::mobile::cancel_access_import();
    }

    pub(super) fn ios_import_error(
        &mut self,
        source: Option<PathBuf>,
        issue: Option<AccessIssue>,
        error: &str,
    ) {
        self.ios_access.status = crate::platform::mobile::photo_status();
        if source.is_some() || issue.is_some() {
            let photo = source
                .as_ref()
                .is_some_and(|p| crate::core::photo_library::title(p).is_some())
                || matches!(
                    issue,
                    Some(
                        AccessIssue::Denied
                            | AccessIssue::Empty
                            | AccessIssue::Restricted
                            | AccessIssue::OriginalUnavailable
                            | AccessIssue::PhotoRead
                    )
                );
            self.ios_access.dialog = Some(Dialog {
                source,
                photo,
                issue,
                action_failed: false,
            });
            // Unknown native errors retain their useful details below the recovery UI.
            self.last_error = if issue.is_none() {
                crate::i18n::tr(error).into()
            } else {
                String::new()
            };
        } else {
            self.last_error = crate::i18n::tr(error).into();
        }
        if self.files.is_empty() && self.listing.is_none() {
            self.library.open = true;
        }
    }

    fn perform_ios_access(&mut self, action: i32, source: Option<PathBuf>) {
        use crate::platform::mobile;
        if action == 5 || action == 6 {
            self.cancel_ios_recovery();
            if action == 5 {
                mobile::pick();
            } else {
                mobile::pick_folder();
            }
            return;
        }
        if action == 0 || (action == 4 && source.is_some()) {
            self.ios_access.waiting = None;
            if let Some(source) = source {
                self.start_ios_source(source);
            }
            return;
        }
        let wait = Waiting {
            source: source.clone(),
            status: mobile::photo_status(),
            action,
            photo: source.as_ref().is_none_or(|p| {
                crate::core::documents::source_identity(p)
                    .is_some_and(|(_, id)| crate::core::photo_library::is_ios(&id))
            }),
        };
        if mobile::access_action(action, source.as_deref()) {
            self.ios_access.waiting = Some(wait);
            if let Some(dialog) = &mut self.ios_access.dialog {
                dialog.action_failed = false;
            }
        } else if let Some(dialog) = &mut self.ios_access.dialog {
            dialog.action_failed = true;
        } else {
            self.last_error = tr("access-action-failed").into();
        }
    }

    pub(super) fn poll_ios_access(&mut self) {
        use crate::platform::mobile;
        let events = mobile::take_access_actions();
        let selection_changed = events
            .iter()
            .any(|&(action, success)| action == 2 && success);
        for (action, success) in &events {
            if !success && *action != 3 {
                if let Some(dialog) = &mut self.ios_access.dialog {
                    dialog.action_failed = true;
                } else {
                    self.last_error = tr("access-action-failed").into();
                }
                self.ios_access.waiting = None;
            }
            if *action == 3 {
                self.ios_access.waiting = None;
            }
        }
        let changed = mobile::take_access_changed()
            || selection_changed
            || events.iter().any(|(action, _)| *action == 4)
            || self.ios_access.refresh_pending;
        if changed {
            let status = mobile::photo_status();
            let previous = self.ios_access.status;
            self.ios_access.refresh_pending = false;
            let current = self
                .mobile_selection
                .as_ref()
                .and_then(|batch| batch.source())
                .filter(|source| crate::core::photo_library::is_ios(&source.id))
                .map(|source| {
                    self.files
                        .get(self.focus_index)
                        .map(|p| self.page_location(p))
                        .unwrap_or_else(|| source.root.clone())
                });
            if mobile::access_import_busy()
                && !(previous == PhotoStatus::NotDetermined && status.readable())
            {
                let active_photo = mobile::access_import_source()
                    .filter(|path| crate::core::photo_library::title(path).is_some());
                if status != previous || selection_changed || status == PhotoStatus::Limited {
                    if let Some(active) = active_photo {
                        mobile::cancel_access_import();
                        self.ios_access.deferred = Some(active);
                    }
                    if current.is_some() {
                        self.invalidate_ios_photos();
                    }
                }
                self.ios_access.refresh_pending = true;
                self.ios_access.status = status;
                return;
            }
            self.ios_access.status = status;
            self.reload_library();
            // The permission sheet itself also activates the scene. Let its
            // existing worker finish; it already uses the newly granted access.
            if !mobile::access_import_busy() {
                let invalidate =
                    current.is_some() && (status != previous || status == PhotoStatus::Limited);
                if invalidate {
                    self.invalidate_ios_photos();
                }
                let waiting = self.ios_access.waiting.take();
                let retry = waiting
                    .as_ref()
                    .filter(|w| w.retry(status, selection_changed))
                    .and_then(|w| w.source.clone())
                    .or_else(|| {
                        (invalidate && status.readable())
                            .then_some(current.clone())
                            .flatten()
                    });
                if let Some(source) = retry {
                    self.start_ios_source(source);
                } else if let Some(dialog) = &mut self.ios_access.dialog {
                    if dialog.photo && status != previous {
                        dialog.issue = None;
                    }
                } else if invalidate && !status.readable() {
                    self.ios_access.dialog = Some(Dialog {
                        source: current,
                        photo: true,
                        issue: None,
                        action_failed: false,
                    });
                }
            }
        }
        if !mobile::access_import_busy()
            && let Some(path) = self.ios_access.deferred.take()
        {
            self.open_ios_source(&path);
        }
    }

    fn invalidate_ios_photos(&mut self) {
        self.remember_page();
        self.reset_view();
        self.folder = None;
        self.listing = None;
        self.folder_watch = None;
        self.mobile_selection = None;
        self.library.open = true;
        self.library.remember = None;
    }

    pub(super) fn ios_photo_access_controls(&mut self, ui: &mut egui::Ui) {
        let status = crate::platform::mobile::photo_status();
        self.ios_access.status = status;
        if let Some(action) = photo_access_controls(ui, status) {
            let source = self
                .mobile_selection
                .as_ref()
                .and_then(|batch| batch.source())
                .filter(|source| crate::core::photo_library::is_ios(&source.id))
                .map(|source| source.root.clone());
            self.perform_ios_access(action, source);
        }
    }

    pub(super) fn ios_photos_limited(&self) -> bool {
        self.ios_access.status == PhotoStatus::Limited
            && self
                .mobile_selection
                .as_ref()
                .and_then(|batch| batch.source())
                .is_some_and(|source| crate::core::photo_library::is_ios(&source.id))
    }

    pub(super) fn show_ios_photo_access(&mut self) {
        let source = self
            .mobile_selection
            .as_ref()
            .and_then(|batch| batch.source())
            .map(|source| {
                self.files
                    .get(self.focus_index)
                    .map(|p| self.page_location(p))
                    .unwrap_or_else(|| source.root.clone())
            });
        self.ios_access.dialog = Some(Dialog {
            source,
            photo: true,
            issue: None,
            action_failed: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn photo_access_settings_stay_compact_in_scrolling_phone_and_tablet_views() {
        use crate::i18n::Language;
        crate::ui::mobile::set_preview(true);
        for language in [
            Language::English,
            Language::German,
            Language::Korean,
            Language::Arabic,
        ] {
            crate::i18n::set_language(language);
            for size in [
                egui::vec2(320., 640.),
                egui::vec2(390., 844.),
                egui::vec2(844., 390.),
                egui::vec2(768., 1024.),
                egui::vec2(1024., 768.),
            ] {
                for status in [
                    PhotoStatus::NotDetermined,
                    PhotoStatus::Denied,
                    PhotoStatus::Restricted,
                    PhotoStatus::Authorized,
                    PhotoStatus::Limited,
                ] {
                    let ctx = egui::Context::default();
                    let _ = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ui| {
                            forms::style(ui);
                            let scroll = egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    photo_access_controls(ui, status);
                                    ui.separator();
                                    let next = ui.label("Next setting");
                                    assert!(
                                        ctx.viewport_rect().contains_rect(next.rect),
                                        "Next setting pushed off screen at {size}: {status:?}"
                                    );
                                });
                            assert!(
                                scroll.content_size.y < 240.,
                                "Access section uses excess height at {size}: {status:?}, {:?}",
                                scroll.content_size
                            );
                        },
                    );
                }
            }
        }
        crate::ui::mobile::set_preview(false);
    }

    #[test]
    fn recovery_actions_follow_source_type_and_permission() {
        assert_eq!(
            actions(true, PhotoStatus::Denied, true),
            vec![(1, "access-open-settings")]
        );
        assert!(actions(true, PhotoStatus::Restricted, true).is_empty());
        assert!(actions(true, PhotoStatus::Limited, true).contains(&(2, "photo-selection-change")));
        assert!(actions(false, PhotoStatus::Denied, true).contains(&(3, "source-reselect")));
        assert!(!actions(false, PhotoStatus::Denied, true).contains(&(4, "photo-access-allow")));
    }
    #[test]
    fn settings_return_does_not_loop_without_a_permission_change() {
        let waiting = Waiting {
            source: Some("photo".into()),
            status: PhotoStatus::Denied,
            action: 1,
            photo: true,
        };
        assert!(!waiting.retry(PhotoStatus::Denied, false));
        assert!(waiting.retry(PhotoStatus::Limited, false));
        let waiting = Waiting {
            source: Some("photo".into()),
            status: PhotoStatus::Limited,
            action: 2,
            photo: true,
        };
        assert!(!waiting.retry(PhotoStatus::Limited, false));
        assert!(waiting.retry(PhotoStatus::Limited, true));
        let document = Waiting {
            source: Some("document".into()),
            status: PhotoStatus::Denied,
            action: 1,
            photo: false,
        };
        assert!(document.retry(PhotoStatus::Denied, false));
        assert_eq!(
            actions(false, PhotoStatus::Denied, false),
            vec![(5, "open"), (6, "open-folder")]
        );
    }
}
