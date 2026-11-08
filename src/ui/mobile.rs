//! Platform-independent phone layout and gesture decisions.
use eframe::egui::Pos2;
#[cfg(any(target_os = "ios", target_os = "android", test))]
use eframe::egui::{Rect, Vec2};

thread_local! { static PREVIEW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub(crate) fn preview_enabled() -> bool {
    PREVIEW.get()
}
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub(crate) fn set_preview(enabled: bool) {
    PREVIEW.set(enabled);
}

pub(crate) const BAR_HEIGHT: f32 = 52.;

pub(crate) use crate::core::commands::mobile_supported as supported;

#[cfg(any(target_os = "ios", target_os = "android", test))]
pub(crate) fn safe_rect(screen: Rect, insets: [f32; 4], pixels_per_point: f32) -> Rect {
    let ppp = pixels_per_point.max(0.1);
    let [top, right, bottom, left] =
        insets.map(|v| if v.is_finite() { v.max(0.) / ppp } else { 0. });
    Rect::from_min_max(
        screen.min
            + Vec2::new(
                left.min(screen.width() * 0.4),
                top.min(screen.height() * 0.4),
            ),
        screen.max
            - Vec2::new(
                right.min(screen.width() * 0.4),
                bottom.min(screen.height() * 0.4),
            ),
    )
}

#[derive(Default)]
pub(crate) struct TouchGesture {
    start: Option<Pos2>,
    multiple: bool,
}

impl TouchGesture {
    pub fn start(&mut self, position: Pos2) {
        self.start = Some(position);
        self.multiple = false;
    }
    pub fn multiple(&mut self) {
        self.multiple = true;
    }
    pub fn cancel(&mut self) {
        self.start = None;
        self.multiple = false;
    }
    pub fn finish(&mut self, end: Pos2, zoomed: bool, rtl: bool) -> Option<bool> {
        let delta = end - self.start.take()?;
        if self.multiple || zoomed || delta.x.abs() < 48. || delta.x.abs() < delta.y.abs() * 1.5 {
            return None;
        }
        Some((delta.x < 0.) != rtl)
    }
    pub fn active(&self) -> bool {
        self.start.is_some()
    }
    pub fn was_multiple(&self) -> bool {
        self.multiple
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swipe_respects_zoom_direction_and_multitouch() {
        let mut gesture = TouchGesture::default();
        gesture.start(Pos2::new(200., 100.));
        assert_eq!(
            gesture.finish(Pos2::new(120., 105.), false, false),
            Some(true)
        );
        gesture.start(Pos2::new(200., 100.));
        assert_eq!(
            gesture.finish(Pos2::new(120., 105.), false, true),
            Some(false)
        );
        gesture.start(Pos2::new(200., 100.));
        gesture.multiple();
        assert_eq!(gesture.finish(Pos2::new(120., 105.), false, false), None);
        gesture.start(Pos2::new(200., 100.));
        assert_eq!(gesture.finish(Pos2::new(120., 105.), true, false), None);
        gesture.start(Pos2::new(200., 100.));
        assert_eq!(gesture.finish(Pos2::new(120., 220.), false, false), None);
    }
    #[test]
    fn converts_safe_area_from_physical_pixels() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(390., 844.));
        let safe = safe_rect(screen, [177., 0., 102., 0.], 3.);
        assert_eq!(safe.min.y, 59.);
        assert_eq!(safe.max.y, 810.);
        assert!(!supported("trash"));
        assert!(supported("open"));
    }
}
