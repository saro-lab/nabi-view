//! Permission and recovery state; OS authorization never follows from a saved path.
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PhotoStatus {
    #[default]
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
    Limited,
}

impl PhotoStatus {
    pub(crate) fn from_native(value: i32) -> Self {
        match value {
            0 => Self::NotDetermined,
            2 => Self::Denied,
            3 => Self::Authorized,
            4 => Self::Limited,
            _ => Self::Restricted,
        }
    }
    pub(crate) fn readable(self) -> bool {
        matches!(self, Self::Authorized | Self::Limited)
    }
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::NotDetermined => "photo-access-request",
            Self::Restricted => "photo-access-restricted",
            Self::Denied => "ios-photo-access-denied",
            Self::Authorized => "photo-access-authorized",
            Self::Limited => "ios-photo-access-limited",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccessIssue {
    Denied,
    Empty,
    Restricted,
    OriginalUnavailable,
    PhotoRead,
    Reconnect,
    ProviderRead,
}

impl AccessIssue {
    pub(crate) fn from_native(reason: i32) -> Self {
        match reason {
            1 => Self::Denied,
            2 => Self::Empty,
            3 => Self::Restricted,
            4 => Self::OriginalUnavailable,
            6 => Self::Reconnect,
            7 => Self::ProviderRead,
            _ => Self::PhotoRead,
        }
    }
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Denied => "ios-photo-access-denied",
            Self::Empty => "ios-photo-collection-empty",
            Self::Restricted => "photo-access-restricted",
            Self::OriginalUnavailable => "photo-original-unavailable",
            Self::PhotoRead => "photo-read-failed",
            Self::Reconnect => "source-reselect-needed",
            Self::ProviderRead => "source-read-failed",
        }
    }
}

#[derive(Default)]
pub(crate) struct ImportGate {
    generation: u64,
    active: Option<(u64, Option<PathBuf>)>,
    pub(crate) issue: Option<AccessIssue>,
}

impl ImportGate {
    #[cfg(target_os = "ios")]
    pub(crate) const fn new() -> Self {
        Self {
            generation: 0,
            active: None,
            issue: None,
        }
    }
    pub(crate) fn start(&mut self, source: Option<PathBuf>) -> bool {
        if self.active.is_some() {
            return false;
        }
        self.generation = self.generation.wrapping_add(1);
        self.active = Some((self.generation, source));
        self.issue = None;
        true
    }
    pub(crate) fn busy(&self) -> bool {
        self.active.is_some()
    }
    #[cfg(target_os = "ios")]
    pub(crate) fn source(&self) -> Option<PathBuf> {
        self.active.as_ref().and_then(|(_, source)| source.clone())
    }
    #[cfg(target_os = "ios")]
    pub(crate) fn identify(&mut self, path: PathBuf) {
        if self.current()
            && let Some((_, source)) = &mut self.active
        {
            if source.is_none() {
                *source = Some(path);
            }
        }
    }
    pub(crate) fn current(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|(generation, _)| *generation == self.generation)
    }
    pub(crate) fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
    pub(crate) fn finish(&mut self) -> Option<(Option<PathBuf>, Option<AccessIssue>)> {
        let current = self.current();
        let (_, source) = self.active.take()?;
        let issue = self.issue.take();
        current.then_some((source, issue))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canceled_import_cannot_publish_or_capture_a_later_request() {
        let mut gate = ImportGate::default();
        assert!(gate.start(Some("old".into())));
        gate.cancel();
        assert!(!gate.current());
        assert!(!gate.start(Some("new".into())));
        assert_eq!(gate.finish(), None);
        assert!(gate.start(Some("new".into())));
        gate.issue = Some(AccessIssue::Reconnect);
        assert_eq!(
            gate.finish(),
            Some((Some("new".into()), Some(AccessIssue::Reconnect)))
        );
        assert!(!gate.busy());
    }

    #[test]
    fn limited_is_readable_and_restricted_is_not_denied() {
        assert!(PhotoStatus::from_native(4).readable());
        assert!(!PhotoStatus::from_native(1).readable());
        assert_ne!(
            PhotoStatus::from_native(1).message(),
            PhotoStatus::from_native(2).message()
        );
        assert_ne!(
            AccessIssue::from_native(2).message(),
            AccessIssue::from_native(4).message()
        );
        assert_ne!(
            AccessIssue::from_native(6).message(),
            AccessIssue::from_native(1).message()
        );
    }
}
