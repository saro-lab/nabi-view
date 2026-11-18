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

struct PhotoSnapshot {
    source: PathBuf,
    status: PhotoStatus,
    revision: u64,
}

struct PhotoValidation {
    request: u64,
    source: PathBuf,
    directory: PathBuf,
    revision: u64,
}

#[derive(Default)]
pub(super) struct AccessUi {
    pub(super) dialog: Option<Dialog>,
    waiting: Option<Waiting>,
    #[cfg(target_os = "ios")]
    deferred: Option<PathBuf>,
    #[cfg(target_os = "ios")]
    refresh_pending: bool,
    status: PhotoStatus,
    snapshot: Option<PhotoSnapshot>,
    resume_pending: bool,
    validation: Option<PhotoValidation>,
    refreshing: bool,
    restore_library_after_refresh: bool,
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
    pub(super) fn accept_ios_seed(
        &mut self,
        selection: crate::core::documents::ImportBatch,
        status: PhotoStatus,
        revision: u64,
    ) -> bool {
        if !selection.ios_access_current(revision, status) {
            self.discard_mobile_seed();
            self.library.open = true;
            return false;
        }
        // Automatic revalidation already retains the user's current screen.
        // Its early page must not open the library's backdrop or reset a photo
        // the user is still viewing; let the normal full refresh finish.
        if self.ios_access.refreshing {
            return false;
        }
        self.accept_mobile_seed(selection);
        true
    }

    /// Native publication and UI consumption are separate moments. Recheck the
    /// exact authorization/content stamp before retaining any seed texture.
    pub(super) fn accept_ios_selection(
        &mut self,
        selection: crate::core::documents::ImportBatch,
        completion: Option<(Option<PathBuf>, Option<AccessIssue>, u64)>,
        status: PhotoStatus,
        revision: u64,
    ) -> bool {
        let photo = selection
            .source()
            .is_some_and(|source| crate::core::photo_library::is_ios(&source.id));
        let current = !photo
            || (selection.ios_access_current(revision, status)
                && completion
                    .as_ref()
                    .is_some_and(|(request, issue, started)| {
                        issue.is_none()
                            && *started == revision
                            && request
                                .as_ref()
                                .and_then(|path| crate::core::documents::source_identity(path))
                                .is_some_and(|(root, _)| {
                                    selection.source().is_some_and(|source| source.root == root)
                                })
                    }));
        if !current {
            self.discard_mobile_seed();
            self.cancel_library_preview();
            self.library.open = true;
            return false;
        }
        self.accept_mobile_selection(selection);
        self.remember_ios_selection(completion, status);
        true
    }

    pub(super) fn remember_ios_selection(
        &mut self,
        completion: Option<(Option<PathBuf>, Option<AccessIssue>, u64)>,
        status: PhotoStatus,
    ) {
        if self.ios_access.refreshing && self.ios_access.restore_library_after_refresh {
            self.library.open = true;
        }
        self.ios_access.refreshing = false;
        self.ios_access.restore_library_after_refresh = false;
        self.ios_access.snapshot = None;
        let Some((Some(request), None, revision)) = completion else {
            return;
        };
        let Some(source) = self
            .mobile_selection
            .as_ref()
            .and_then(|batch| batch.source())
        else {
            return;
        };
        if status.readable()
            && crate::core::photo_library::is_ios(&source.id)
            && crate::core::documents::source_identity(&request)
                .is_some_and(|(root, _)| root == source.root)
        {
            self.ios_access.snapshot = Some(PhotoSnapshot {
                source: source.root.clone(),
                status,
                revision,
            });
        }
    }

    fn reuse_ios_selection(&mut self, path: &Path, status: PhotoStatus, revision: u64) -> bool {
        let Some(snapshot) = &self.ios_access.snapshot else {
            return false;
        };
        if ((self.ios_access.resume_pending || self.ios_access.validation.is_some())
            && status != PhotoStatus::Authorized)
            || !status.readable()
            || snapshot.status != status
            || snapshot.revision != revision
            || self.listing.is_some()
            || self.file_job.is_some()
            || self.files.is_empty()
        {
            return false;
        }
        let Some(selection) = &self.mobile_selection else {
            return false;
        };
        let Some(source) = selection.source() else {
            return false;
        };
        if source.root != snapshot.source || self.folder.as_deref() != Some(selection.directory()) {
            return false;
        }
        let focus = if path == source.root {
            self.focus_index
        } else {
            let Some(local) = selection.local_path(path) else {
                return false;
            };
            let Some(index) = self.files.iter().position(|candidate| candidate == &local) else {
                return false;
            };
            index
        };
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

    fn prepare_ios_validation(&mut self, request: u64, revision: u64) -> bool {
        let Some(snapshot) = &self.ios_access.snapshot else {
            return false;
        };
        let Some(selection) = &self.mobile_selection else {
            return false;
        };
        if snapshot.status != PhotoStatus::Authorized
            || snapshot.revision != revision
            || selection
                .source()
                .is_none_or(|source| source.root != snapshot.source)
            || self.folder.as_deref() != Some(selection.directory())
        {
            return false;
        }
        self.ios_access.validation = Some(PhotoValidation {
            request,
            source: snapshot.source.clone(),
            directory: selection.directory().to_owned(),
            revision,
        });
        self.ios_access.resume_pending = false;
        true
    }

    /// A delayed validation cannot approve a replacement batch of the same source.
    fn finish_ios_validation(
        &mut self,
        request: u64,
        same: bool,
        status: PhotoStatus,
        revision: u64,
    ) -> Option<bool> {
        if self.ios_access.validation.as_ref()?.request != request {
            return None;
        }
        let validation = self.ios_access.validation.take().unwrap();
        let selection = self.mobile_selection.as_ref()?;
        if selection.directory() != validation.directory
            || selection
                .source()
                .is_none_or(|source| source.root != validation.source)
        {
            return None;
        }
        Some(same && status == PhotoStatus::Authorized && revision == validation.revision)
    }

    pub(super) fn ios_photos_refreshing(&self) -> bool {
        self.ios_access.refreshing
            && !self
                .mobile_selection
                .as_ref()
                .is_some_and(|batch| batch.is_seed())
    }

    fn hold_ios_photos_for_refresh(&mut self) {
        if !self.ios_access.refreshing {
            self.ios_access.restore_library_after_refresh = self.library.open;
        }
        if !self.library.open
            && let Some(path) = self
                .files
                .get(self.focus_index)
                .map(|file| self.page_location(file))
        {
            self.preserve_current_photo_preview(&path);
        }
        self.ios_access.refreshing = true;
        // Retain only completed display state from the old authorized batch.
        // Its provider guards have expired; fresh work belongs to the new list.
        self.media.clear_slot(0);
        self.media.clear_slot(1);
        self.tickets = [None, None];
        self.cancel_prefetch();
        self.media.clear_thumbnail();
        self.thumb_ticket = None;
        self.details.clear();
    }

    fn ios_refresh_keeps_library(&self) -> bool {
        if self.ios_access.refreshing {
            self.ios_access.restore_library_after_refresh
        } else {
            self.library.open
        }
    }

    fn prepare_ios_automatic_refresh(&mut self, keep_library: bool) {
        self.ios_access.refreshing = true;
        self.ios_access.restore_library_after_refresh = keep_library;
    }

    fn ios_refresh_retry(
        &mut self,
        current: Option<PathBuf>,
        invalidated: bool,
        status: PhotoStatus,
        selection_changed: bool,
        keep_library: bool,
    ) -> Option<PathBuf> {
        let requested = self
            .ios_access
            .waiting
            .take()
            .filter(|waiting| waiting.retry(status, selection_changed))
            .and_then(|waiting| waiting.source);
        if requested.is_some() {
            // A permission action initiated by the user still opens its target.
            self.ios_access.restore_library_after_refresh = false;
            return requested;
        }
        if invalidated && status.readable() && current.is_some() {
            self.prepare_ios_automatic_refresh(keep_library);
            current
        } else {
            None
        }
    }

    fn invalidate_ios_photos(&mut self) {
        self.cancel_library_preview();
        self.ios_access.refreshing = false;
        self.ios_access.restore_library_after_refresh = false;
        self.ios_access.snapshot = None;
        self.remember_page();
        self.reset_view();
        self.folder = None;
        self.listing = None;
        self.folder_watch = None;
        self.mobile_selection = None;
        self.library.open = true;
        self.library.remember = None;
    }

    fn ios_import_invalidates_current(
        &self,
        source: Option<&Path>,
        issue: Option<AccessIssue>,
        status: PhotoStatus,
    ) -> bool {
        let Some(current) = self
            .mobile_selection
            .as_ref()
            .and_then(|batch| batch.source())
            .filter(|source| crate::core::photo_library::is_ios(&source.id))
        else {
            return false;
        };
        // Global revoked access hides every PhotoKit source. A source-specific
        // empty/error result must not discard a different currently owned batch.
        !status.readable()
            || (source
                .and_then(crate::core::documents::source_identity)
                .is_some_and(|(root, _)| root == current.root)
                && (status != PhotoStatus::Authorized
                    || matches!(
                        issue,
                        Some(AccessIssue::Empty | AccessIssue::Denied | AccessIssue::Restricted)
                    )))
    }

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
        self.discard_mobile_seed();
        // This entry point represents a requested open. Automatic refreshes
        // use start_ios_source directly and preserve an already open library.
        self.ios_access.restore_library_after_refresh = false;
        self.clear_library_preview_for_source(path);
        // With unchanged full authorization, a warm tap can show the retained
        // batch immediately while its metadata is revalidated in the background.
        if (self.ios_access.resume_pending || self.ios_access.validation.is_some())
            && !mobile::access_import_busy()
            && mobile::photo_status() == PhotoStatus::Authorized
            && self.reuse_ios_selection(path, PhotoStatus::Authorized, mobile::access_revision())
        {
            return true;
        }
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
            if !self.reuse_ios_selection(path, self.ios_access.status, mobile::access_revision()) {
                self.start_ios_source(path.to_owned());
            }
        }
        true
    }

    fn start_ios_source(&mut self, path: PathBuf) {
        self.ios_access.validation = None;
        self.ios_access.resume_pending = false;
        crate::platform::mobile::cancel_photo_validation();
        self.ios_access.dialog = None;
        self.last_error.clear();
        // Keep the originating screen until the asynchronous selection arrives.
        // accept_mobile_selection() closes the library through reset_view().
        if !crate::platform::mobile::reopen(&path) {
            self.cancel_library_preview();
            self.ios_access.refreshing = false;
            self.ios_access.restore_library_after_refresh = false;
            self.ios_access.dialog = Some(Dialog {
                photo: crate::core::photo_library::title(&path).is_some(),
                source: Some(path),
                issue: Some(AccessIssue::Reconnect),
                action_failed: false,
            });
        }
    }

    pub(super) fn cancel_ios_recovery(&mut self) {
        self.discard_mobile_seed();
        self.cancel_library_preview();
        self.ios_access.validation = None;
        self.ios_access.resume_pending = false;
        self.ios_access.refreshing = false;
        self.ios_access.restore_library_after_refresh = false;
        crate::platform::mobile::cancel_photo_validation();
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
        self.discard_mobile_seed();
        self.cancel_library_preview();
        self.ios_access.status = crate::platform::mobile::photo_status();
        self.ios_access.refreshing = false;
        self.ios_access.restore_library_after_refresh = false;
        if self.ios_import_invalidates_current(source.as_deref(), issue, self.ios_access.status) {
            self.invalidate_ios_photos();
        }
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
        let mut access_changed = mobile::take_access_changed();
        if mobile::take_access_resumed() {
            self.ios_access.resume_pending = true;
            if self
                .ios_access
                .waiting
                .as_ref()
                .is_some_and(|wait| !wait.photo)
            {
                access_changed = true;
            }
        }
        if let Some((request, same)) = mobile::take_photo_validation() {
            let status = mobile::photo_status();
            match self.finish_ios_validation(request, same, status, mobile::access_revision()) {
                Some(true) => mobile::cancel_photo_validation(),
                Some(false) => access_changed = true,
                None if self.ios_access.validation.is_none() => mobile::cancel_photo_validation(),
                None => {}
            }
        }
        if self.ios_access.resume_pending && !access_changed && !selection_changed {
            let status = mobile::photo_status();
            if status == PhotoStatus::Authorized && status == self.ios_access.status {
                if mobile::access_import_busy() {
                    return;
                }
                if let Some(source) = self.ios_access.snapshot.as_ref().map(|s| s.source.clone())
                    && let Some(request) = mobile::validate_photo_source(&source)
                    && self.prepare_ios_validation(request, mobile::access_revision())
                {
                    return;
                }
                // No retained PhotoKit batch needs validation (for example at
                // launch or while viewing a Files document).
                if self
                    .mobile_selection
                    .as_ref()
                    .and_then(|s| s.source())
                    .is_some_and(|source| crate::core::photo_library::is_ios(&source.id))
                {
                    access_changed = true;
                } else {
                    self.ios_access.resume_pending = false;
                    mobile::cancel_photo_validation();
                }
            } else {
                // Limited -> Limited can remove permission for individual
                // assets. Keep its existing invalidate-and-refetch behavior.
                access_changed = true;
            }
        }
        let changed = access_changed
            || selection_changed
            || events.iter().any(|(action, _)| *action == 4)
            || self.ios_access.refresh_pending;
        if changed {
            self.ios_access.validation = None;
            self.ios_access.resume_pending = false;
            mobile::cancel_photo_validation();
            self.ios_access.snapshot = None;
            let status = mobile::photo_status();
            let previous = self.ios_access.status;
            // Invalidation may temporarily open the library and clear refresh
            // state. Capture the user's destination before changing either.
            let keep_library = self.ios_refresh_keeps_library();
            let automatic_refresh = self.ios_access.refreshing;
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
            self.discard_mobile_seed();
            if mobile::access_import_busy()
                && !(previous == PhotoStatus::NotDetermined && status.readable())
            {
                let active_photo = mobile::access_import_source()
                    .filter(|path| crate::core::photo_library::title(path).is_some());
                if access_changed
                    || status != previous
                    || selection_changed
                    || status == PhotoStatus::Limited
                {
                    if let Some(active) = active_photo {
                        mobile::cancel_access_import();
                        self.ios_access.deferred = Some(active);
                    }
                    if current.is_some() {
                        if previous == PhotoStatus::Authorized && status == PhotoStatus::Authorized
                        {
                            self.hold_ios_photos_for_refresh();
                        } else {
                            self.invalidate_ios_photos();
                            if automatic_refresh && status.readable() {
                                self.prepare_ios_automatic_refresh(keep_library);
                            }
                        }
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
                let invalidate = current.is_some()
                    && (access_changed
                        || status != previous
                        || selection_changed
                        || status == PhotoStatus::Limited);
                if invalidate {
                    if previous == PhotoStatus::Authorized && status == PhotoStatus::Authorized {
                        self.hold_ios_photos_for_refresh();
                    } else {
                        self.invalidate_ios_photos();
                    }
                }
                let retry = self.ios_refresh_retry(
                    current.clone(),
                    invalidate,
                    status,
                    selection_changed,
                    keep_library,
                );
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
            if self.ios_access.refreshing {
                self.start_ios_source(path);
            } else {
                self.open_ios_source(&path);
            }
        }
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
    fn limited_refresh_invalidates_old_photos_and_preserves_the_home_destination() {
        use crate::core::documents::ImportBatch;
        use std::time::{Duration, Instant};
        for home in [false, true] {
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
            let png = std::fs::read(crate::app::sample_path().unwrap()).unwrap();
            let make_batch = |revision| {
                let mut batch = ImportBatch::new_folder(root.path()).unwrap();
                batch
                    .set_source(
                        &root.path().join("document-sources"),
                        "nabi-ios-library",
                        "Photos",
                    )
                    .unwrap();
                batch.add("selected.png", png.as_slice()).unwrap();
                batch.request_page(Some(batch.source().unwrap().root.join("selected.png")));
                batch.set_ios_access_stamp(revision, PhotoStatus::Limited);
                batch.commit_source().unwrap();
                batch
            };
            app.accept_mobile_selection(make_batch(7));
            let deadline = Instant::now() + Duration::from_secs(5);
            while app.listing.is_some() || app.pictures.is_empty() {
                assert!(Instant::now() < deadline);
                app.collect(&ctx);
                std::thread::sleep(Duration::from_millis(1));
            }
            let request = app.page_location(&app.files[app.focus_index]);
            if home {
                app.open_mobile_home(&ctx);
                assert!(
                    !app.pictures.is_empty(),
                    "home retains the completed photo before a permission change"
                );
            }
            let keep_library = app.ios_refresh_keeps_library();
            assert_eq!(keep_library, home);
            // These are the same invalidation and retry-policy methods used by
            // poll_ios_access after a Limited resume or deferred access change.
            app.invalidate_ios_photos();
            assert!(app.pictures.is_empty() && app.mobile_selection.is_none());
            assert_eq!(
                app.ios_refresh_retry(
                    Some(request.clone()),
                    true,
                    PhotoStatus::Limited,
                    false,
                    keep_library
                ),
                Some(request.clone())
            );
            let full = make_batch(8);
            assert!(!app.accept_ios_seed(full.snapshot_seed().unwrap(), PhotoStatus::Limited, 8));
            assert!(
                app.library.open,
                "partial refresh must not interrupt home/loading"
            );
            assert!(app.accept_ios_selection(
                full,
                Some((Some(request.clone()), None, 8)),
                PhotoStatus::Limited,
                8
            ));
            assert_eq!(
                app.library.open, home,
                "full completion keeps the original destination"
            );
            assert!(!app.ios_access.refreshing);
            // Revocation still clears capabilities/pixels and never reopens.
            app.invalidate_ios_photos();
            assert!(
                app.ios_refresh_retry(Some(request), true, PhotoStatus::Denied, false, home)
                    .is_none()
            );
            assert!(app.library.open && app.pictures.is_empty() && app.mobile_selection.is_none());
        }
    }

    #[test]
    fn automatic_refresh_seed_preserves_the_library_or_current_photo_until_full_completion() {
        use crate::core::documents::ImportBatch;
        use std::time::{Duration, Instant};
        for library_open in [false, true] {
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
            let png = std::fs::read(crate::app::sample_path().unwrap()).unwrap();
            let make_batch = || {
                let mut batch = ImportBatch::new_folder(root.path()).unwrap();
                batch
                    .set_source(
                        &root.path().join("document-sources"),
                        "nabi-ios-library",
                        "Photos",
                    )
                    .unwrap();
                batch.add("selected.png", png.as_slice()).unwrap();
                batch.request_page(Some(batch.source().unwrap().root.join("selected.png")));
                batch.set_ios_access_stamp(7, PhotoStatus::Authorized);
                batch
            };
            let mut previous = make_batch();
            previous.commit_source().unwrap();
            app.accept_mobile_selection(previous);
            let deadline = Instant::now() + Duration::from_secs(5);
            while app.listing.is_some() || app.pictures.is_empty() {
                assert!(Instant::now() < deadline);
                app.collect(&ctx);
                std::thread::sleep(Duration::from_millis(1));
            }
            let files = app.files.clone();
            let texture = app.pictures[&files[0]].handle.id();
            app.library.open = library_open;
            app.hold_ios_photos_for_refresh();
            let mut full = make_batch();
            assert!(!app.accept_ios_seed(
                full.snapshot_seed().unwrap(),
                PhotoStatus::Authorized,
                7
            ));
            assert_eq!(app.library.open, library_open);
            assert_eq!(app.files, files);
            assert_eq!(app.pictures[&files[0]].handle.id(), texture);
            assert!(!app.source_list_pending());
            assert!(app.ios_photos_refreshing());
            full.commit_source().unwrap();
            let request = full.source().unwrap().root.clone();
            assert!(app.accept_ios_selection(
                full,
                Some((Some(request), None, 7)),
                PhotoStatus::Authorized,
                7
            ));
            assert_eq!(app.library.open, library_open);
            assert!(!app.ios_photos_refreshing());
        }
    }

    #[test]
    fn same_photo_source_reuses_pixels_only_with_current_access_snapshot() {
        use crate::core::documents::ImportBatch;
        use std::time::{Duration, Instant};

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
                "nabi-ios-library",
                "Photos",
            )
            .unwrap();
        selection.commit_source().unwrap();
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
        app.focus(1);
        app.zoom = 2.;
        app.pan = egui::vec2(11., 19.);
        app.checked.insert(directory.join("second.png"));
        app.library.open = true;
        app.remember_ios_selection(
            Some((Some(source.clone()), None, 7)),
            PhotoStatus::Authorized,
        );
        assert!(app.reuse_ios_selection(&source, PhotoStatus::Authorized, 7));
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
        assert!(app.listing.is_none());

        // Foreground revalidation preserves the real owned files and decoded
        // texture. Full access allows a warm tap before the result arrives.
        app.ios_access.resume_pending = true;
        assert!(app.prepare_ios_validation(11, 7));
        app.library.open = true;
        assert!(app.reuse_ios_selection(&source, PhotoStatus::Authorized, 7));
        assert_eq!(
            app.pictures[&directory.join("first.png")].handle.id(),
            texture
        );
        assert_eq!(
            app.finish_ios_validation(10, true, PhotoStatus::Authorized, 7),
            None
        );
        assert!(
            app.ios_access.validation.is_some(),
            "older results must not consume the current request"
        );
        assert_eq!(
            app.finish_ios_validation(11, true, PhotoStatus::Authorized, 7),
            Some(true)
        );
        assert_eq!(
            app.mobile_selection.as_ref().unwrap().directory(),
            directory
        );
        assert!(app.listing.is_none());
        for (same, status, revision) in [
            (false, PhotoStatus::Authorized, 7),
            (true, PhotoStatus::Denied, 7),
            (true, PhotoStatus::Limited, 7),
            (true, PhotoStatus::Authorized, 8),
        ] {
            assert!(app.prepare_ios_validation(12, 7));
            assert_eq!(
                app.finish_ios_validation(12, same, status, revision),
                Some(false)
            );
        }
        app.focus(0);
        app.library.open = false;
        app.hold_ios_photos_for_refresh();
        assert!(app.ios_photos_refreshing());
        assert_eq!(
            app.pictures[&directory.join("first.png")].handle.id(),
            texture
        );
        app.queue_images();
        assert!(app.tickets.iter().all(Option::is_none));
        app.remember_ios_selection(
            Some((Some(source.clone()), None, 7)),
            PhotoStatus::Authorized,
        );
        assert!(!app.ios_photos_refreshing());
        assert!(!app.library.open);
        app.cancel_library_preview();
        app.library.open = true;
        app.hold_ios_photos_for_refresh();
        // A background refresh must not navigate away from the recent list.
        assert!(app.library.open);
        app.library.open = false; // accept_mobile_selection's reset_view.
        app.remember_ios_selection(
            Some((Some(source.clone()), None, 7)),
            PhotoStatus::Authorized,
        );
        assert!(app.library.open);

        assert!(app.ios_import_invalidates_current(
            Some(&source),
            Some(AccessIssue::Empty),
            PhotoStatus::Authorized
        ));
        assert!(!app.ios_import_invalidates_current(
            Some(&source),
            Some(AccessIssue::PhotoRead),
            PhotoStatus::Authorized
        ));
        assert!(!app.ios_import_invalidates_current(
            Some(&source.with_file_name("nabi-ios-favorites")),
            Some(AccessIssue::Empty),
            PhotoStatus::Authorized
        ));
        assert!(app.ios_import_invalidates_current(
            Some(&source),
            Some(AccessIssue::Denied),
            PhotoStatus::Denied
        ));

        // A saved explicit page still wins over the active source's position.
        assert!(app.reuse_ios_selection(&source.join("first.png"), PhotoStatus::Authorized, 7,));
        assert_eq!(app.focus_index, 0);
        for (path, status, revision) in [
            (source.clone(), PhotoStatus::Denied, 7),
            (source.clone(), PhotoStatus::Restricted, 7),
            (source.clone(), PhotoStatus::NotDetermined, 7),
            (source.clone(), PhotoStatus::Limited, 7),
            (source.clone(), PhotoStatus::Authorized, 8),
            (source.join("deleted.png"), PhotoStatus::Authorized, 7),
            (
                source.with_file_name("nabi-ios-screenshots"),
                PhotoStatus::Authorized,
                7,
            ),
        ] {
            app.library.open = true;
            assert!(!app.reuse_ios_selection(&path, status, revision));
            assert!(app.library.open);
        }
        // Matching limited access may be reused; a selection change increments
        // the revision even when the enum remains Limited.
        app.remember_ios_selection(Some((Some(source.clone()), None, 9)), PhotoStatus::Limited);
        app.ios_access.resume_pending = true;
        assert!(!app.prepare_ios_validation(13, 9));
        assert!(!app.reuse_ios_selection(&source, PhotoStatus::Limited, 9));
        app.ios_access.resume_pending = false;
        assert!(app.reuse_ios_selection(&source, PhotoStatus::Limited, 9));
        assert!(!app.reuse_ios_selection(&source, PhotoStatus::Limited, 10));
        // Simulate an import spanning a content/permission change: completion
        // retains request revision 9 instead of becoming fresh at revision 10.
        app.remember_ios_selection(
            Some((Some(source.clone()), None, 9)),
            PhotoStatus::Authorized,
        );
        assert!(!app.reuse_ios_selection(&source, PhotoStatus::Authorized, 10));

        // A completion for a different source cannot bless the active batch.
        app.remember_ios_selection(
            Some((
                Some(source.with_file_name("nabi-ios-screenshots")),
                None,
                10,
            )),
            PhotoStatus::Authorized,
        );
        assert!(!app.reuse_ios_selection(&source, PhotoStatus::Authorized, 10));
        let mut files = ImportBatch::new_folder(root.path()).unwrap();
        files.add("page.png", image.as_slice()).unwrap();
        files
            .set_source(
                &root.path().join("document-sources"),
                "12345678-files-folder",
                "Files",
            )
            .unwrap();
        files.commit_source().unwrap();
        let source = files.source().unwrap().root.clone();
        app.accept_mobile_selection(files);
        app.remember_ios_selection(Some((Some(source), None, 10)), PhotoStatus::Authorized);
        assert!(app.ios_access.snapshot.is_none());
    }

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
