//! The inspector starts at most one reader. Navigation supersedes its target and
//! never makes the frame wait for that read to finish. Spread pages have
//! independent reports and controls, sharing the same single-reader budget.
use crate::core::metadata::{self, Metadata};
use eframe::egui::{self, Color32, FontId, Rect, Response, Sense, Ui};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::core::metadata::tree::{Row, Tree};

struct PreparedMetadata {
    metadata: Metadata,
    tree: Tree,
    rows: Vec<Row>,
}

impl PreparedMetadata {
    fn new(metadata: Metadata) -> Self {
        let tree = Tree::new(&metadata);
        let rows = tree.rows();
        Self {
            metadata,
            tree,
            rows,
        }
    }
}

type Report = Result<PreparedMetadata, String>;
const COPY_FADE_SECONDS: f64 = 0.55;

struct Pending {
    generation: u64,
    receiver: Receiver<Report>,
}

#[derive(Default)]
pub(super) struct Inspector {
    pub open: bool,
    pages: [PageInspector; 2],
}

impl Inspector {
    pub fn invalidate(&mut self) {
        for page in &mut self.pages {
            page.set_target(None);
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.invalidate();
    }

    pub(super) fn poll(&mut self) {
        for page in &mut self.pages {
            page.poll();
        }
    }

    pub fn show(&mut self, parent: &mut Ui, rects: &[Rect], paths: &[&Path]) -> bool {
        if !self.open {
            return false;
        }
        for (index, page) in self.pages.iter_mut().enumerate() {
            page.set_target(paths.get(index).copied());
        }
        // A superseded read must finish before another parser is started.
        if self.pages.iter().all(|page| page.pending.is_none())
            && let Some(page) = self
                .pages
                .iter_mut()
                .find(|page| page.target.is_some() && page.report.is_none())
        {
            page.request(parent.ctx());
        }
        let mut copied = false;
        for (index, (page, &rect)) in self.pages.iter_mut().zip(rects).enumerate() {
            copied |= page.show(parent, rect, index);
        }
        copied
    }
}

#[derive(Default)]
struct PageInspector {
    target: Option<PathBuf>,
    generation: u64,
    pending: Option<Pending>,
    report: Option<Report>,
    row_heights: HashMap<(usize, usize), f32>,
    layout_width: f32,
    layout_scale: f32,
    copied_at: HashMap<egui::Id, f64>,
}

impl PageInspector {
    fn set_target(&mut self, path: Option<&Path>) {
        if self.target.as_deref() != path {
            self.target = path.map(Path::to_owned);
            self.generation = self.generation.wrapping_add(1);
            self.report = None;
            self.row_heights.clear();
            self.copied_at.clear();
        }
    }

    pub(super) fn poll(&mut self) {
        let Some(job) = &self.pending else { return };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("The metadata reader stopped unexpectedly.".into())
            }
        };
        if job.generation == self.generation {
            self.row_heights.clear();
            self.report = Some(result);
        }
        self.pending = None;
    }

    fn request(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() || self.report.is_some() {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let context = ctx.clone();
        match std::thread::Builder::new()
            .name("metadata-inspector".into())
            .spawn(move || {
                let _ = sender.send(metadata::read(&target).map(PreparedMetadata::new));
                context.request_repaint();
            }) {
            Ok(_) => {
                self.pending = Some(Pending {
                    generation: self.generation,
                    receiver,
                })
            }
            Err(error) => self.report = Some(Err(error.to_string())),
        }
    }

    fn show(&mut self, parent: &mut Ui, rect: Rect, index: usize) -> bool {
        let mut ui = parent.new_child(
            egui::UiBuilder::new()
                .id_salt(("metadata-on-image", index))
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        ui.set_clip_rect(parent.clip_rect().intersect(rect));
        ui.spacing_mut().item_spacing = egui::vec2(6., 1.);
        ui.spacing_mut().interact_size.y = 16.;
        if self.target.is_none() {
            return false;
        }
        let Some(report) = &mut self.report else {
            metadata_label(
                &mut ui,
                crate::i18n::tr("reading-metadata-dialog"),
                12.,
                Sense::hover(),
            );
            return false;
        };
        let prepared = match report {
            Ok(report) => report,
            Err(error) => {
                metadata_label(&mut ui, error, 12., Sense::hover());
                return false;
            }
        };
        let PreparedMetadata {
            metadata,
            tree,
            rows,
        } = prepared;
        let now = ui.input(|i| i.time);
        self.copied_at
            .retain(|_, started| now - *started < COPY_FADE_SECONDS);
        let mut copied = false;
        ui.horizontal_wrapped(|ui| {
            for (n, (index, text)) in summary(metadata).iter().enumerate() {
                if n > 0 {
                    metadata_label(ui, "/", 12., Sense::hover());
                }
                copied |= copyable(
                    ui,
                    text,
                    || text.clone(),
                    egui::Id::new(("summary", index)),
                    &mut self.copied_at,
                );
            }
        });
        let available = ui.available_height();
        egui::ScrollArea::vertical()
            .id_salt(("on-image-text", self.generation))
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .max_height(available)
            .auto_shrink([false, true])
            .show_viewport(&mut ui, |ui, viewport| {
                let width = ui.available_width();
                let scale = ui.ctx().pixels_per_point();
                if self.layout_width != width || self.layout_scale != scale {
                    self.row_heights.clear();
                    self.layout_width = width;
                    self.layout_scale = scale;
                }
                let origin = ui.cursor().min;
                let mut y = 0.;
                for row in rows.iter() {
                    let key = (row.node, row.chunk);
                    let mut height = self.row_heights.get(&key).copied().unwrap_or(22.);
                    // Only lay out and paint visible text. Unvisited rows use a
                    // small height estimate until scrolled into view, so even a
                    // multi-megabyte workflow remains a single scrollable list.
                    if y + height >= viewport.min.y && y <= viewport.max.y {
                        let node = &tree.nodes[row.node];
                        let mut row_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt(key)
                                .max_rect(Rect::from_min_size(
                                    origin + egui::vec2(0., y),
                                    egui::vec2(width, height),
                                ))
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        row_ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.;
                            let indent = (row.depth as f32 * 18.).min(width * 0.45);
                            ui.add_space(indent);
                            let mut label = node.label(row.chunk, row.depth > 0);
                            if row.depth > 0 && row.chunk == 0 {
                                label.insert_str(0, "- ");
                            }
                            copied |= copyable(
                                ui,
                                &label,
                                || tree.copy_text(row.node),
                                egui::Id::new(("tree", row.node)),
                                &mut self.copied_at,
                            );
                        });
                        height = row_ui.min_rect().height() + 3.;
                        self.row_heights.insert(key, height);
                    }
                    y += height;
                }
                ui.allocate_space(egui::vec2(width, y));
                if tree.limited {
                    metadata_label(
                        ui,
                        crate::i18n::tr("large-or-deeply-nested-json-is-shown-as-text"),
                        12.,
                        Sense::hover(),
                    );
                }
                for warning in &metadata.warnings {
                    metadata_label(ui, warning, 12., Sense::hover());
                }
            });
        copied
    }
}

fn summary(metadata: &Metadata) -> Vec<(usize, String)> {
    let field = |name: &str| {
        metadata
            .fields
            .iter()
            .enumerate()
            .find(|(_, (key, _))| key == name)
    };
    let mut parts = Vec::new();
    if let Some((index, (_, extension))) = field("Extension").or_else(|| field("Format")) {
        parts.push((index, extension.to_uppercase()));
    }
    if let Some((index, (_, dimensions))) = field("Dimensions") {
        let text = match field("Frame count") {
            Some((_, (_, count))) => {
                format!(
                    "{dimensions} ({})",
                    crate::i18n::format(
                        if count == "1" {
                            "metadata-frames-one"
                        } else {
                            "metadata-frames-other"
                        },
                        &[("count", count)]
                    )
                )
            }
            None => dimensions.clone(),
        };
        parts.push((index, text));
    }
    if let Some((index, (_, size))) = field("File size") {
        parts.push((index, size.clone()));
    }
    parts
}

fn copyable(
    ui: &mut Ui,
    text: &str,
    value: impl FnOnce() -> String,
    index: egui::Id,
    copied_at: &mut HashMap<egui::Id, f64>,
) -> bool {
    let now = ui.input(|i| i.time);
    let ctx = ui.ctx().clone();
    let response = metadata_label_color(ui, text, 12., Sense::click(), |response| {
        if response.clicked() {
            ctx.copy_text(value());
            copied_at.insert(index, now);
        }
        let Some(started) = copied_at.get(&index) else {
            return Color32::WHITE;
        };
        let progress = ((now - started - 0.1) / (COPY_FADE_SECONDS - 0.1)).clamp(0., 1.) as f32;
        if progress < 1. {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let channel =
            |start: u8| (f32::from(start) + (255. - f32::from(start)) * progress).round() as u8;
        Color32::from_rgb(channel(150), channel(205), 255)
    });
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// Keep the regular-weight fill legible over photos with a black outer outline.
fn metadata_label(ui: &mut Ui, text: &str, size: f32, sense: Sense) -> Response {
    metadata_label_color(ui, text, size, sense, |_| Color32::WHITE)
}

fn metadata_label_color(
    ui: &mut Ui,
    text: &str,
    size: f32,
    sense: Sense,
    color: impl FnOnce(&Response) -> Color32,
) -> Response {
    let galley = ui.painter().layout(
        text.to_owned(),
        FontId::proportional(size),
        Color32::WHITE,
        (ui.available_width() - 4.).max(1.),
    );
    let padding = egui::Vec2::splat(1.4);
    let (rect, response) = ui.allocate_exact_size(galley.size() + padding * 2., sense);
    let color = color(&response);
    if ui.is_rect_visible(rect) {
        let position = rect.min + padding;
        // Reuse the same layout for a one-point outline in all directions.
        // Paint the regular fill last so the outline does not thicken the text.
        for (x, y) in [
            (-1., -1.),
            (0., -1.),
            (1., -1.),
            (-1., 0.),
            (1., 0.),
            (-1., 1.),
            (0., 1.),
            (1., 1.),
        ] {
            ui.painter().galley_with_override_text_color(
                position + egui::vec2(x, y),
                galley.clone(),
                Color32::BLACK,
            );
        }
        ui.painter()
            .galley_with_override_text_color(position, galley, color);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::metadata::tree::{VALUE_CHUNK_BYTES, value_chunk};

    #[test]
    fn metadata_scroll_reaches_and_copies_the_end_without_pagination_or_laying_out_every_row() {
        let mut metadata = Metadata::default();
        let graph: serde_json::Map<String, serde_json::Value> = (0..500)
            .map(|i| (format!("node{i:03}"), serde_json::json!({"seed": i})))
            .collect();
        metadata.add("workflow", serde_json::Value::Object(graph).to_string());
        let long_value = format!("{}\nEND_OF_METADATA", "한글 prompt ".repeat(1500));
        metadata.add("note", &long_value);
        let prepared = PreparedMetadata::new(metadata);
        let row_count = prepared.rows.len();
        assert!(row_count > 1_000);
        let mut page = PageInspector {
            target: Some(PathBuf::from("metadata.png")),
            report: Some(Ok(prepared)),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut time = 0.;
        let mut draw = |page: &mut PageInspector, events: Vec<egui::Event>| {
            time += 0.1;
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(360., 300.),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        page.show(ui, ui.max_rect(), 0);
                    });
                },
            )
        };
        let labels = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text.galley.job.text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let first = draw(&mut page, Vec::new());
        assert!(
            labels(&first).contains("seed: 0"),
            "JSON children are expanded immediately"
        );
        assert!(
            page.row_heights.len() < 30,
            "Offscreen rows must not be laid out eagerly"
        );
        let pointer = egui::pos2(180., 200.);
        let mut tail = None;
        for _ in 0..20 {
            let output = draw(
                &mut page,
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., -100_000.),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            let text = labels(&output);
            assert!(
                !text.contains("‹ Back")
                    && !text.contains("More ›")
                    && !text.contains("‹ 이전")
                    && !text.contains("다음 ›")
            );
            tail = output.shapes.iter().find_map(|shape| {
                let egui::Shape::Text(text) = &shape.shape else {
                    return None;
                };
                let bounds = text.visual_bounding_rect().intersect(shape.clip_rect);
                (text.galley.job.text.contains("END_OF_METADATA") && bounds.is_positive())
                    .then_some(bounds.center())
            });
            if tail.is_some() {
                break;
            }
        }
        let position = tail.expect("The last value must be reachable by scrolling alone");
        assert!(
            page.row_heights.len() < row_count / 2,
            "Jumping to the end must keep distant rows virtualized"
        );
        // Clear the smoothed wheel delta before clicking the final visible chunk.
        ctx.input_mut(|input| {
            input.smooth_scroll_delta = egui::Vec2::ZERO;
            input.raw_scroll_delta = egui::Vec2::ZERO;
        });
        draw(
            &mut page,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let copied = draw(
            &mut page,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            copied.platform_output.commands.iter().any(|command| {
                matches!(command, egui::OutputCommand::CopyText(value) if value == &long_value)
            }),
            "Clicking the last layout chunk must copy the complete value"
        );
    }

    #[test]
    fn unicode_workflow_is_recoverable_across_every_layout_chunk() {
        let original = format!(
            "{}한글🙂{}",
            "x".repeat(VALUE_CHUNK_BYTES - 1),
            "그림".repeat(3000)
        );
        let mut joined = String::new();
        for n in 0..original.len().div_ceil(VALUE_CHUNK_BYTES) {
            joined.push_str(value_chunk(&original, n));
        }
        assert_eq!(joined, original);
    }

    #[test]
    fn stale_read_after_navigation_cannot_replace_new_file() {
        let mut inspector = PageInspector::default();
        inspector.set_target(Some(Path::new("page-1.png")));
        let (sender, receiver) = mpsc::channel();
        inspector.pending = Some(Pending {
            generation: inspector.generation,
            receiver,
        });
        inspector.set_target(Some(Path::new("page-2.png")));
        sender.send(Err("old result".into())).ok().unwrap();
        inspector.poll();
        assert!(inspector.report.is_none() && inspector.pending.is_none());
        assert_eq!(inspector.target.as_deref(), Some(Path::new("page-2.png")));
    }
}
