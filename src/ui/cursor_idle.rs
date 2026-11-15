//! Hide only this window's cursor; repaint once when the idle deadline expires.
use eframe::egui::{self, CursorIcon, Event};
use std::time::Duration;

#[derive(Default)]
pub struct CursorIdle {
    last_activity: Option<f64>,
    last_position: Option<egui::Pos2>,
    inside: bool,
    was_fullscreen: bool,
}

impl CursorIdle {
    pub fn observe(&mut self, ctx: &egui::Context, fullscreen: bool) {
        ctx.input(|input| {
            let inside = input.focused && input.pointer.hover_pos().is_some();
            let mut moved = false;
            for event in &input.events {
                match event {
                    Event::PointerMoved(position) => {
                        moved |= self.last_position != Some(*position);
                        self.last_position = Some(*position);
                    }
                    Event::MouseMoved(delta) => moved |= *delta != egui::Vec2::ZERO,
                    _ => {}
                }
            }
            let activity = input.events.iter().any(|event| {
                matches!(
                    event,
                    Event::PointerMoved(_) | Event::PointerButton { .. } | Event::MouseWheel { .. }
                )
            });
            // Focus changes can discard egui's hover position until the next
            // movement. Keep the fullscreen idle deadline independently, and
            // do not count synthetic button releases or repeated positions.
            let reset = if fullscreen {
                input.focused && moved
            } else {
                !inside || !self.inside || activity || input.pointer.any_down()
            };
            if reset || self.last_activity.is_none() {
                self.last_activity = Some(input.time);
            }
            self.inside = inside;
        });
    }

    /// Apply after all widgets so their hover cursors cannot override hiding.
    pub fn apply(&mut self, ctx: &egui::Context, delay_ms: u32, fullscreen: bool) {
        let (now, focused) = ctx.input(|input| (input.time, input.focused));
        let mut hidden = false;
        if focused && (fullscreen || self.inside) {
            let remaining = self.last_activity.unwrap_or(now) + delay_ms as f64 / 1000. - now;
            hidden = remaining <= 0.;
            if hidden {
                ctx.set_cursor_icon(CursorIcon::None);
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(remaining));
            }
        }
        // egui-winit ignores cursor icons while its pointer position is absent.
        // A window-local command also restores hiding after keyboard refocus,
        // without waiting for a new mouse movement to establish that position.
        if fullscreen || self.was_fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(!hidden));
        }
        self.was_fullscreen = fullscreen;
    }
}
