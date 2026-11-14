//! Read-only filename preview; only the bottom Apply button mutates files.
use super::*;
use crate::core::file_rename::{self, CheckedPlan, Plan};
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) struct RenameDialog {
    plan: Plan,
    checked: Option<CheckedPlan>,
    validation: Option<Receiver<Result<CheckedPlan, file_rename::Error>>>,
    replace_tab: bool,
    digits: usize,
    from: String,
    to: String,
    pub error: String,
    pub failed: bool,
}
impl RenameDialog {
    fn validate(&mut self, candidate: Result<Plan, file_rename::Error>, ctx: &egui::Context) {
        let candidate = match candidate {
            Ok(plan) => plan,
            Err(error) => {
                self.error = error.display();
                return;
            }
        };
        let (send, receive) = mpsc::channel();
        let baseline = self.checked.clone();
        let repaint = ctx.clone();
        match std::thread::Builder::new()
            .name("check-image-names".into())
            .spawn(move || {
                let _ = send.send(file_rename::check(candidate, baseline.as_ref()));
                repaint.request_repaint();
            }) {
            Ok(_) => {
                self.validation = Some(receive);
                self.error.clear();
            }
            Err(error) => self.error = error.to_string(),
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.validation else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(checked)) => {
                self.plan = checked.plan.clone();
                self.checked = Some(checked);
                self.validation = None;
                self.error.clear();
            }
            Ok(Err(error)) => {
                self.validation = None;
                self.error = error.display();
            }
            Err(TryRecvError::Disconnected) => {
                self.validation = None;
                self.error = tr("the-file-operation-could-not-be-completed").into();
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100))
            }
        }
    }
}

impl Viewer {
    pub(super) fn renaming(&self) -> bool {
        self.rename_dialog.is_some() && self.file_job.is_some()
    }

    pub(super) fn open_rename_dialog_for_files(&mut self, ctx: &egui::Context) {
        if self.file_job.is_some() {
            self.show_toast(ctx, tr("a-file-operation-is-still-in-progress"));
            return;
        }
        if self.listing.is_some() {
            self.show_toast(ctx, tr("rename-wait-for-list"));
            return;
        }
        if self.files.is_empty() {
            return;
        }
        let plan = Plan::new(&self.files);
        let mut dialog = RenameDialog {
            digits: plan.digits(),
            plan: plan.clone(),
            checked: None,
            validation: None,
            replace_tab: false,
            from: String::new(),
            to: String::new(),
            error: String::new(),
            failed: false,
        };
        dialog.validate(Ok(plan), ctx);
        self.rename_dialog = Some(dialog);
        self.menu_open = false;
        self.library.open = false;
        self.zoom_input = None;
        self.release_held_input();
        self.canvas_drag = None;
    }

    pub(super) fn rename_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.rename_dialog.take() else {
            return;
        };
        dialog.poll(ctx);
        let busy = self.file_job.is_some();
        let validating = dialog.validation.is_some();
        let editable = !busy && !validating && !dialog.failed;
        let mut close = false;
        let mut submit = false;
        let mut preview = false;
        let mut reset = false;
        let modal = overlay::show(ctx, "normalize-names-window", self.fullscreen, |ui| {
            close = overlay::header(ui, "normalize-names", "normalize-names-close");
            if !overlay::compact(ctx) {
                ui.heading(tr("normalize-names"));
            }
            ui.add_enabled_ui(editable, |ui| {
                crate::ui::direction::horizontal(ui, |ui| {
                    let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.;
                    for (replace, label) in
                        [(false, "rename-folder-tab"), (true, "rename-replace-tab")]
                    {
                        if ui
                            .add_sized(
                                [width, forms::height()],
                                egui::Button::new(tr(label))
                                    .selected(dialog.replace_tab == replace)
                                    .wrap(),
                            )
                            .clicked()
                        {
                            dialog.replace_tab = replace;
                        }
                    }
                });
                ui.add_space(6.);
                if dialog.replace_tab {
                    // Stack the inputs on narrow windows, retaining full labels.
                    let narrow = ui.available_width() < 430.;
                    if narrow {
                        ui.add_sized(
                            [ui.available_width(), forms::height()],
                            forms::text_edit(&mut dialog.from)
                                .id(egui::Id::unique("rename-from"))
                                .hint_text(tr("rename-from")),
                        );
                        ui.add_sized(
                            [ui.available_width(), forms::height()],
                            forms::text_edit(&mut dialog.to)
                                .id(egui::Id::unique("rename-to"))
                                .hint_text(tr("rename-to")),
                        );
                        preview = ui
                            .add_enabled(
                                !dialog.from.is_empty(),
                                egui::Button::new(tr("rename-preview")),
                            )
                            .clicked();
                    } else {
                        ui.horizontal(|ui| {
                            let button_width = ui
                                .painter()
                                .layout_no_wrap(
                                    tr("rename-preview").into(),
                                    egui::TextStyle::Button.resolve(ui.style()),
                                    INK,
                                )
                                .size()
                                .x
                                + 2. * ui.spacing().button_padding.x;
                            let width = (ui.available_width()
                                - button_width
                                - 2. * ui.spacing().item_spacing.x)
                                / 2.;
                            ui.add_sized(
                                [width, forms::height()],
                                forms::text_edit(&mut dialog.from)
                                    .id(egui::Id::unique("rename-from"))
                                    .hint_text(tr("rename-from")),
                            );
                            ui.add_sized(
                                [width, forms::height()],
                                forms::text_edit(&mut dialog.to)
                                    .id(egui::Id::unique("rename-to"))
                                    .hint_text(tr("rename-to")),
                            );
                            preview = ui
                                .add_enabled(
                                    !dialog.from.is_empty(),
                                    egui::Button::new(tr("rename-preview")),
                                )
                                .clicked();
                        });
                    }
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr("rename-digits"));
                        forms::number(ui, egui::DragValue::new(&mut dialog.digits).range(1..=20));
                        preview = ui.button(tr("rename-preview")).clicked();
                    });
                }
            });
            ui.add_space(8.);
            ui.label(
                RichText::new(tr("rename-preview-hint"))
                    .small()
                    .color(MUTED),
            );
            // Reserve the bottom controls and warning area before laying out the list.
            let warning_height = if dialog.error.is_empty() { 0. } else { 78. };
            let status_height = if busy || validating {
                forms::height() + forms::GAP
            } else {
                0.
            };
            let list_height =
                (ui.available_height() - forms::height() - warning_height - status_height - 48.)
                    .max(36.);
            egui::Frame::new()
                .fill(Color32::from_rgb(13, 15, 17))
                .stroke(egui::Stroke::new(1., Color32::from_gray(65)))
                .corner_radius(4.)
                .inner_margin(8.)
                .show(ui, |ui| {
                    let row_height = ui.text_style_height(&egui::TextStyle::Body).max(20.);
                    egui::ScrollArea::vertical()
                        .id_salt("rename-preview-list")
                        .max_height(list_height)
                        .min_scrolled_height(list_height)
                        .auto_shrink([false, false])
                        .show_rows(ui, row_height, dialog.plan.entries.len(), |ui, rows| {
                            // Filenames retain their own text direction, including RTL UI.
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                for index in rows {
                                    let entry = &dialog.plan.entries[index];
                                    let name = entry
                                        .target
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy();
                                    // add_sized centers the label itself even with
                                    // halign(Min); give each row an explicit LTR layout.
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(ui.available_width(), row_height),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.set_min_height(row_height);
                                            ui.add(
                                                egui::Label::new(name.as_ref())
                                                    .halign(egui::Align::Min)
                                                    .truncate()
                                                    .selectable(false),
                                            )
                                        },
                                    )
                                    .inner
                                    .hover_text(name.as_ref());
                                }
                            });
                        });
                });
            if !dialog.error.is_empty() {
                egui::ScrollArea::vertical()
                    .id_salt("rename-errors")
                    .max_height(70.)
                    .show(ui, |ui| {
                        ui.colored_label(WARNING, &dialog.error);
                    });
            }
            ui.add_space(6.);
            if busy || validating {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr(if busy {
                        "rename-working"
                    } else {
                        "rename-checking"
                    }));
                });
            }
            crate::ui::direction::horizontal(ui, |ui| {
                let width = (ui.available_width() - 2. * ui.spacing().item_spacing.x) / 3.;
                let mut button = |enabled, label| {
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.add_sized(
                            [width, forms::height()],
                            egui::Button::new(tr(label)).truncate(),
                        )
                    })
                    .inner
                    .hover_text(tr(label))
                    .clicked()
                };
                reset = button(editable, "reset");
                close |= button(!busy, "cancel");
                submit = button(
                    editable
                        && dialog.checked.is_some()
                        && dialog.error.is_empty()
                        && dialog.plan.changes() > 0,
                    "rename-commit",
                );
            });
        });
        if (close || modal.should_close()) && !busy {
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
            self.release_held_input();
            return;
        }
        if reset {
            let plan = Plan::new(
                &dialog
                    .plan
                    .entries
                    .iter()
                    .map(|e| e.source.clone())
                    .collect::<Vec<_>>(),
            );
            dialog.validate(Ok(plan), ctx);
        } else if preview {
            let candidate = if dialog.replace_tab {
                dialog.plan.replace(&dialog.from, &dialog.to)
            } else {
                dialog.plan.folder_names(dialog.digits)
            };
            dialog.validate(candidate, ctx);
        }
        let checked = submit.then(|| dialog.checked.clone()).flatten();
        self.rename_dialog = Some(dialog);
        if let Some(checked) = checked {
            self.start_rename(checked, ctx);
        }
    }

    pub(super) fn reflect_renamed_files(&mut self, renamed: &[(PathBuf, PathBuf)]) {
        let mapping: HashMap<_, _> = renamed.iter().cloned().collect();
        let remap = |path: PathBuf| mapping.get(&path).cloned().unwrap_or(path);
        for path in &mut self.files {
            *path = remap(std::mem::take(path));
        }
        self.checked = self.checked.drain().map(remap).collect();
        self.sorting.stamps = self
            .sorting
            .stamps
            .drain()
            .map(|(path, stamp)| (remap(path), stamp))
            .collect();
        let changed: Vec<_> = renamed
            .iter()
            .flat_map(|(a, b)| [a.clone(), b.clone()])
            .collect();
        self.refresh_folder(self.files.clone(), &changed);
        self.inspector.invalidate();
        self.check_navigation = None;
        if let Some(store) = &self.store
            && let Err(error) = store.rename_library_images(renamed)
        {
            self.last_error = error;
        }
        self.remember_page();
        self.reload_library();
    }
}
