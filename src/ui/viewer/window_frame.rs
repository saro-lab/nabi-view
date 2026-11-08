//! Native borderless resize handles and device-local normal-window geometry.
use crate::core::settings::valid_window_size;
use crate::storage::Store;
use eframe::egui::{self, CursorIcon, Pos2, Rect, ResizeDirection};
use std::time::Duration;

const EDGE: f32 = 6.;
const CORNER: f32 = 12.;
const SAVE_DELAY: f64 = 0.5;

fn drag_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::new(("native-window-drag-started", ctx.viewport_id()))
}

pub(super) fn reset_drag(ctx: &egui::Context) {
    let id = drag_id(ctx);
    ctx.data_mut(|data| data.remove::<bool>(id));
}

/// Start native movement once, only after movement from a live primary press.
pub(super) fn drag(ctx: &egui::Context, response: &egui::Response) {
    if !response.is_pointer_button_down_on()
        || !ctx.input(|i| {
            i.focused
                && i.pointer.primary_down()
                && i.pointer
                    .press_origin()
                    .zip(i.pointer.latest_pos())
                    .is_some_and(|(origin, position)| origin.distance_sq(position) > 0.)
        })
    {
        return;
    }
    let id = drag_id(ctx);
    let started = ctx.data_mut(|data| {
        let started = data.get_temp::<bool>(id).unwrap_or(false);
        data.insert_temp(id, true);
        started
    });
    if !started {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

fn handles(rect: Rect) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let region = |x1, y1, x2, y2| Rect::from_min_max(Pos2::new(x1, y1), Pos2::new(x2, y2));
    [
        (
            region(l, t, l + CORNER, t + CORNER),
            ResizeDirection::NorthWest,
            CursorIcon::ResizeNwSe,
        ),
        (
            region(r - CORNER, t, r, t + CORNER),
            ResizeDirection::NorthEast,
            CursorIcon::ResizeNeSw,
        ),
        (
            region(l, b - CORNER, l + CORNER, b),
            ResizeDirection::SouthWest,
            CursorIcon::ResizeNeSw,
        ),
        (
            region(r - CORNER, b - CORNER, r, b),
            ResizeDirection::SouthEast,
            CursorIcon::ResizeNwSe,
        ),
        (
            region(l + CORNER, t, r - CORNER, t + EDGE),
            ResizeDirection::North,
            CursorIcon::ResizeVertical,
        ),
        (
            region(l + CORNER, b - EDGE, r - CORNER, b),
            ResizeDirection::South,
            CursorIcon::ResizeVertical,
        ),
        (
            region(l, t + CORNER, l + EDGE, b - CORNER),
            ResizeDirection::West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            region(r - EDGE, t + CORNER, r, b - CORNER),
            ResizeDirection::East,
            CursorIcon::ResizeHorizontal,
        ),
    ]
}

/// Call after the viewer's windows/overlays, so these narrow areas own edge input.
pub fn resize(ctx: &egui::Context) {
    if ctx.input(|i| {
        let viewport = i.viewport();
        viewport.fullscreen.unwrap_or(false)
            || viewport.maximized.unwrap_or(false)
            || viewport.minimized.unwrap_or(false)
    }) {
        return;
    }
    for (index, (rect, direction, cursor)) in handles(ctx.screen_rect()).into_iter().enumerate() {
        let area = egui::Area::new(egui::Id::new(("native-resize", index)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .default_size(rect.size())
            .constrain(false)
            .fade_in(false)
            .sense(egui::Sense::drag())
            .movable(false)
            .show(ctx, |ui| {
                // Use the Area's own response: child widgets are disabled in its
                // initial sizing pass, whereas its root hit target is live immediately.
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
            });
        ctx.move_to_top(area.response.layer_id);
        let response = area.response.on_hover_and_drag_cursor(cursor);
        if response.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

#[derive(Default)]
pub struct WindowSizeState {
    latest: Option<[f32; 2]>,
    saved: Option<[f32; 2]>,
    save_at: f64,
}

impl WindowSizeState {
    /// Observe only native, normal-window dimensions in logical points.
    pub fn observe(&mut self, ctx: &egui::Context, store: Option<&Store>) -> Result<(), String> {
        let (size, now) = ctx.input(|i| {
            let viewport = i.viewport();
            let normal = !viewport.minimized.unwrap_or(false)
                && !viewport.maximized.unwrap_or(false)
                && !viewport.fullscreen.unwrap_or(false);
            (
                normal
                    .then_some(viewport.inner_rect)
                    .flatten()
                    .map(|rect| rect.size()),
                i.time,
            )
        });
        if let Some(size) = size {
            self.observe_size([size.x.round(), size.y.round()], now);
        }
        if self.latest != self.saved {
            if now >= self.save_at {
                // Back off after a failure as well; don't retry at the render frame rate.
                self.save_at = now + SAVE_DELAY;
                ctx.request_repaint_after(Duration::from_secs_f64(SAVE_DELAY));
                self.flush(store)?;
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(self.save_at - now));
            }
        }
        Ok(())
    }

    fn observe_size(&mut self, size: [f32; 2], now: f64) {
        if valid_window_size(size) && self.latest != Some(size) {
            self.latest = Some(size);
            self.save_at = now + SAVE_DELAY;
        }
    }

    /// Flush the final pending normal size on close, even during the debounce period.
    pub fn flush(&mut self, store: Option<&Store>) -> Result<(), String> {
        if self.latest == self.saved {
            return Ok(());
        }
        if let (Some(size), Some(store)) = (self.latest, store) {
            store.save_window_size(size)?;
        }
        self.saved = self.latest;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Vec2;

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.))),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn every_edge_and_corner_sets_cursor_and_requests_native_resize() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
        for (rect, direction, cursor) in handles(screen) {
            let ctx = egui::Context::default();
            let point = rect.center();
            // First frame registers areas; subsequent native input uses those rectangles.
            let _ = ctx.run(input(Vec::new()), resize);
            let hovered = ctx.run(input(vec![egui::Event::PointerMoved(point)]), resize);
            assert_eq!(hovered.platform_output.cursor_icon, cursor, "{direction:?}");
            let pressed = ctx.run(
                input(vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }]),
                resize,
            );
            assert!(
                pressed
                    .viewport_output
                    .values()
                    .flat_map(|output| &output.commands)
                    .any(|command| *command == egui::ViewportCommand::BeginResize(direction)),
                "{direction:?}"
            );
        }
    }

    #[test]
    fn maximized_or_fullscreen_windows_have_no_resize_handles() {
        for fullscreen in [false, true] {
            let ctx = egui::Context::default();
            let mut raw = input(vec![egui::Event::PointerMoved(Pos2::new(1., 300.))]);
            let viewport = raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
            viewport.fullscreen = Some(fullscreen);
            viewport.maximized = Some(!fullscreen);
            let output = ctx.run(raw, resize);
            assert_eq!(output.platform_output.cursor_icon, CursorIcon::Default);
            assert!(
                !output
                    .viewport_output
                    .values()
                    .flat_map(|o| &o.commands)
                    .any(|command| matches!(command, egui::ViewportCommand::BeginResize(_)))
            );
        }
    }

    #[test]
    fn size_changes_debounce_and_close_flushes_latest_valid_value() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::in_directories(root.path().join("config"), root.path().join("state"));
        let mut state = WindowSizeState::default();
        state.observe_size([900., 600.], 0.);
        state.observe_size([920., 640.], 0.2);
        assert_eq!(state.save_at, 0.7);
        assert_eq!(store.load_window_size().unwrap(), None);
        state.observe_size([0., 0.], 0.3);
        assert_eq!(state.latest, Some([920., 640.]));
        state.flush(Some(&store)).unwrap();
        assert_eq!(store.load_window_size().unwrap(), Some([920., 640.]));
        assert_eq!(state.latest, state.saved);
    }

    #[test]
    fn observation_ignores_maximized_minimized_and_fullscreen_geometry() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::in_directories(root.path().join("config"), root.path().join("state"));
        let mut state = WindowSizeState::default();
        let ctx = egui::Context::default();
        for (time, size, mode) in [
            (0., [900., 600.], 0),
            (0.2, [940., 680.], 0),
            (1., [1920., 1080.], 1),
            (2., [0., 0.], 2),
            (3., [3840., 2160.], 3),
        ] {
            let mut raw = input(Vec::new());
            raw.time = Some(time);
            let viewport = raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
            viewport.inner_rect =
                Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(size[0], size[1])));
            viewport.maximized = Some(mode == 1);
            viewport.minimized = Some(mode == 2);
            viewport.fullscreen = Some(mode == 3);
            let _ = ctx.run(raw, |ctx| state.observe(ctx, Some(&store)).unwrap());
            if time < 0.7 {
                assert_eq!(store.load_window_size().unwrap(), None);
            }
        }
        assert_eq!(store.load_window_size().unwrap(), Some([940., 680.]));
    }
}
