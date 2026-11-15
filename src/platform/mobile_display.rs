//! Runtime display requests, separate from the user's saved preferences.

#[derive(Default)]
pub(crate) struct DisplayState {
    windowed: Option<bool>,
    sent_orientation: Option<i32>,
    sent_immersive: Option<bool>,
}

impl DisplayState {
    pub const fn new() -> Self {
        Self {
            windowed: None,
            sent_orientation: None,
            sent_immersive: None,
        }
    }

    pub fn windowed(&self) -> Option<bool> {
        self.windowed
    }

    pub fn set_windowed(&mut self, windowed: Option<bool>) -> bool {
        if self.windowed == windowed {
            return false;
        }
        self.windowed = windowed;
        // A native transition may already have released the orientation lock.
        // Invalidate even if both transitions happen between two UI frames.
        self.sent_orientation = None;
        true
    }

    /// Returns only changed requests. Call the host after releasing the lock.
    /// A sent request does not imply that the OS accepted it.
    pub fn requests(&mut self, preferred: &str, immersive: bool) -> (Option<i32>, Option<bool>) {
        let orientation = self.windowed.and_then(|windowed| {
            let mode = match (windowed, preferred) {
                (false, "portrait") => 1,
                (false, "landscape") => 2,
                _ => 0,
            };
            (self.sent_orientation.replace(mode) != Some(mode)).then_some(mode)
        });
        let immersive =
            (self.sent_immersive.replace(immersive) != Some(immersive)).then_some(immersive);
        (orientation, immersive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_mode_uses_system_then_restores_preference() {
        let mut state = DisplayState::new();
        assert_eq!(state.windowed(), None);
        assert_eq!(state.requests("portrait", false), (None, Some(false)));
        state.set_windowed(Some(false));
        assert_eq!(state.requests("portrait", false), (Some(1), None));
        assert_eq!(state.requests("portrait", false), (None, None));

        state.set_windowed(Some(true));
        assert_eq!(state.requests("portrait", false), (Some(0), None));
        // Changing the preference while windowed must not lock the window.
        assert_eq!(state.requests("landscape", true), (None, Some(true)));
        state.set_windowed(Some(false));
        assert_eq!(state.requests("landscape", true), (Some(2), None));
    }

    #[test]
    fn rapid_transitions_and_new_hosts_resend_without_a_request_loop() {
        let mut state = DisplayState::new();
        state.set_windowed(Some(false));
        state.requests("landscape", false);
        state.set_windowed(Some(true));
        state.set_windowed(Some(false));
        assert_eq!(state.requests("landscape", false), (Some(2), None));
        assert!(!state.set_windowed(Some(false)));
        assert_eq!(state.requests("landscape", false), (None, None));

        state.set_windowed(None);
        assert_eq!(state.requests("portrait", false), (None, None));
        state.set_windowed(Some(false));
        assert_eq!(state.requests("portrait", false), (Some(1), None));

        state = DisplayState::new();
        state.set_windowed(Some(false));
        assert_eq!(state.requests("portrait", false), (Some(1), Some(false)));
    }
}
