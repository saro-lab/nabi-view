//! Platform-independent phone/tablet layout and gesture decisions.
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
    pan_only: bool,
}

impl TouchGesture {
    pub fn start(&mut self, position: Pos2) {
        self.start = Some(position);
        self.multiple = false;
        self.pan_only = false;
    }
    pub fn multiple(&mut self) {
        self.multiple = true;
        self.pan_only = true;
    }
    /// Once a touch belongs to the enlarged image, changing its scale or
    /// lifting a second finger cannot turn the same touch into page navigation.
    pub fn lock_pan(&mut self) {
        self.pan_only = true;
    }
    pub fn pan_only(&self) -> bool {
        self.pan_only
    }
    pub fn cancel(&mut self) {
        self.start = None;
        self.multiple = false;
        self.pan_only = false;
    }
    pub fn finish(&mut self, end: Pos2, rtl: bool) -> Option<bool> {
        let delta = end - self.start.take()?;
        if self.pan_only
            || self.multiple
            || delta.x.abs() < 48.
            || delta.x.abs() < delta.y.abs() * 1.5
        {
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
    pub fn swipe_offset(&self, position: Pos2) -> Option<f32> {
        let delta = position - self.start?;
        (!self.pan_only
            && !self.multiple
            && delta.x.abs() >= 8.
            && delta.x.abs() >= delta.y.abs() * 1.5)
            .then_some(delta.x)
    }
    pub fn horizontal_offset(&self, position: Pos2) -> Option<f32> {
        (!self.pan_only && !self.multiple).then_some(position.x - self.start?.x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swipe_respects_reading_direction_and_multitouch() {
        let mut gesture = TouchGesture::default();
        gesture.start(Pos2::new(200., 100.));
        assert_eq!(gesture.finish(Pos2::new(120., 105.), false), Some(true));
        gesture.start(Pos2::new(200., 100.));
        assert_eq!(gesture.finish(Pos2::new(120., 105.), true), Some(false));
        gesture.start(Pos2::new(200., 100.));
        gesture.multiple();
        assert_eq!(gesture.finish(Pos2::new(120., 105.), false), None);
        gesture.start(Pos2::new(200., 100.));
        assert_eq!(gesture.finish(Pos2::new(170., 105.), false), None);
        gesture.start(Pos2::new(200., 100.));
        assert_eq!(gesture.finish(Pos2::new(120., 220.), false), None);
    }
    #[test]
    fn pan_lock_survives_motion_until_a_new_touch_starts() {
        let mut gesture = TouchGesture::default();
        gesture.start(Pos2::new(200., 100.));
        gesture.lock_pan();
        assert!(gesture.pan_only());
        assert_eq!(gesture.swipe_offset(Pos2::new(80., 100.)), None);
        assert_eq!(gesture.horizontal_offset(Pos2::new(80., 100.)), None);
        assert_eq!(gesture.finish(Pos2::new(80., 100.), false), None);
        gesture.start(Pos2::new(200., 100.));
        assert!(!gesture.pan_only());
        assert_eq!(gesture.finish(Pos2::new(80., 100.), false), Some(true));
    }
    #[test]
    fn converts_safe_area_from_physical_pixels() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(390., 844.));
        let safe = safe_rect(screen, [177., 0., 102., 0.], 3.);
        assert_eq!(safe.min.y, 59.);
        assert_eq!(safe.max.y, 810.);
        assert!(supported("trash") && supported("fullscreen") && supported("import"));
        assert!(!supported("reveal") && !supported("move") && !supported("new-window"));
        assert!(supported("open"));
    }
}
