//! Hide only this window's cursor; repaint once when the idle deadline expires.
use eframe::egui::{self, CursorIcon, Event};
use std::time::Duration;

#[derive(Default)]
pub struct CursorIdle {
    last_activity: Option<f64>,
    inside: bool,
}

impl CursorIdle {
    pub fn observe(&mut self, ctx: &egui::Context) {
        ctx.input(|input| {
            let inside = input.focused && input.pointer.hover_pos().is_some();
            let activity = input.events.iter().any(|event| {
                matches!(
                    event,
                    Event::PointerMoved(_) | Event::PointerButton { .. } | Event::MouseWheel { .. }
                )
            });
            if !inside || !self.inside || activity || input.pointer.any_down() {
                self.last_activity = Some(input.time);
            }
            self.inside = inside;
        });
    }

    /// Apply after all widgets so their hover cursors cannot override hiding.
    pub fn apply(&self, ctx: &egui::Context, delay_ms: u32) {
        if !self.inside {
            return;
        }
        let now = ctx.input(|input| input.time);
        let remaining = self.last_activity.unwrap_or(now) + delay_ms as f64 / 1000. - now;
        if remaining <= 0. {
            ctx.set_cursor_icon(CursorIcon::None);
        } else {
            ctx.request_repaint_after(Duration::from_secs_f64(remaining));
        }
    }
}
