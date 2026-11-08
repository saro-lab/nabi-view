//! Portable command IDs, default bindings and chord validation.
use crate::{
    core::settings::{KeyboardShortcut, Settings},
    i18n::tr,
};
use std::collections::HashMap;
pub(crate) const COMMANDS: &[(&str, &str, &str)] = &[
    ("metadata", "image-metadata", "Tab"),
    ("new-window", "new-window", ""),
    ("close-window", "close-window", "Primary+W"),
    ("close-all", "close-all-windows", "Primary+Q"),
    ("minimize", "minimize", ""),
    ("maximize", "maximize-restore", ""),
    ("open", "open-image", "Primary+O"),
    ("folder", "open-folder", "Primary+Shift+O"),
    ("recent", "recent", "Primary+Alt+Shift+O"),
    ("reveal", "reveal-in-file-manager", "L"),
    ("prev", "previous-page-or-spread", "Left"),
    ("next", "next-page-or-spread", "Right"),
    ("check", "toggle-checks", "Space"),
    ("check-prev", "check-and-go-to-previous", "Shift+Left"),
    ("check-next", "check-and-go-to-next", "Shift+Right"),
    ("move", "move-images", "M"),
    ("trash", "move-to-trash", "Delete"),
    ("undo-trash", "restore-from-trash", "Primary+Z"),
    ("first", "first-page", "Home"),
    ("last", "last-page", "End"),
    ("spread", "toggle-two-page-view", ""),
    ("single", "single-page-view", "P"),
    ("spread-left", "spread-left", "OpenBracket"),
    ("spread-right", "spread-right", "CloseBracket"),
    ("flip-horizontal", "flip-horizontally", "R"),
    ("flip-vertical", "flip-vertically", "T"),
    ("slideshow", "slideshow", "Q"),
    (
        "slideshow-decrease",
        "decrease-slideshow-interval-by-0-1-s",
        "Comma",
    ),
    (
        "slideshow-increase",
        "increase-slideshow-interval-by-0-1-s",
        "Period",
    ),
    ("random", "random-order", ""),
    ("width", "fit-width", ""),
    ("height", "fit-height", ""),
    ("rtl", "reverse-reading-direction", ""),
    ("cover", "show-first-page-alone", ""),
    ("fit", "fit-entire-image", "0"),
    ("fill", "fill-viewer", ""),
    ("actual", "actual-size-100", "1"),
    ("in", "zoom-in", "Equals"),
    ("out", "zoom-out", "Minus"),
    ("fullscreen", "toggle-full-screen", "Primary+Enter"),
    ("chrome", "hide-or-show-controls", "H"),
    ("settings", "settings", "Primary+Comma"),
    ("about", "about", "F1"),
    ("remember", "remember-recent-items", ""),
    ("clear", "clear-recent-items", ""),
    ("export", "export-settings", ""),
    ("import", "import-settings", ""),
    ("licenses", "open-source-licenses", ""),
];

pub(crate) const DEFAULT_ALTERNATE_SHORTCUTS: &[(&str, &str)] = &[
    ("prev", "A"),
    ("next", "D"),
    ("first", "W"),
    ("last", "S"),
    ("check-prev", "Shift+A"),
    ("check-next", "Shift+D"),
    ("trash", "Backspace"),
    ("trash", "Primary+E"),
    ("close-window", "Escape"),
    ("fullscreen", "F"),
    ("metadata", "I"),
];

pub(crate) fn default_shortcut_rows() -> Vec<KeyboardShortcut> {
    let mut rows = Vec::new();
    for &(action, _, primary) in COMMANDS {
        if cfg!(any(target_os = "ios", target_os = "android")) && !mobile_supported(action) {
            continue;
        }
        if !primary.is_empty() {
            rows.push(KeyboardShortcut {
                action: action.into(),
                key: primary.into(),
            });
        }
        for &(id, key) in DEFAULT_ALTERNATE_SHORTCUTS {
            if id == action {
                rows.push(KeyboardShortcut {
                    action: action.into(),
                    key: key.into(),
                });
            }
        }
    }
    rows
}
pub(crate) fn canonical_chord(chord: &str) -> String {
    canonical_chord_for_platform(chord, cfg!(target_os = "macos"))
}
pub(crate) fn canonical_chord_for_platform(chord: &str, macos: bool) -> String {
    if !macos {
        let rest = chord.strip_prefix("Primary+").unwrap_or(chord);
        if let Some(rest) = rest.strip_prefix("Ctrl+") {
            return format!("Primary+{rest}");
        }
    }
    chord.to_owned()
}
pub(crate) fn shortcut_label(chord: &str) -> String {
    shortcut_label_for_platform(chord, cfg!(target_os = "macos"))
}
pub(crate) fn shortcut_label_for_platform(chord: &str, macos: bool) -> String {
    let canonical = canonical_chord_for_platform(chord, macos);
    let parts: Vec<_> = canonical.split('+').collect();
    let key_label: fn(&str) -> &str = |part| match part {
        "Primary" => "Ctrl",
        "OpenBracket" => "[",
        "CloseBracket" => "]",
        "Comma" => ",",
        "Period" => ".",
        "Minus" => "−",
        "Plus" => "+",
        "Equals" => "=",
        "Left" => "←",
        "Right" => "→",
        "Up" => "↑",
        "Down" => "↓",
        "Escape" => "Esc",
        _ => part,
    };
    if macos {
        let modifiers: String = [
            ("Ctrl", "⌃"),
            ("Alt", "⌥"),
            ("Shift", "⇧"),
            ("Primary", "⌘"),
        ]
        .into_iter()
        .filter(|(name, _)| parts.contains(name))
        .map(|(_, symbol)| symbol)
        .collect();
        let key = parts
            .into_iter()
            .filter(|part| !matches!(*part, "Primary" | "Ctrl" | "Alt" | "Shift"))
            .map(key_label)
            .collect::<Vec<_>>()
            .join("+");
        if modifiers.is_empty() {
            key
        } else if key.is_empty() {
            modifiers
        } else {
            format!("{modifiers}{key}")
        }
    } else {
        parts
            .into_iter()
            .map(key_label)
            .collect::<Vec<_>>()
            .join("+")
    }
}
pub(crate) fn validate_keys(s: &Settings) -> Result<(), String> {
    validate_keys_for_platform(s, cfg!(target_os = "macos"))
}
pub(crate) fn validate_keys_for_platform(s: &Settings, macos: bool) -> Result<(), String> {
    let mut normalized = s.clone();
    normalized.normalize_shortcuts();
    let rows = normalized.keyboard_shortcuts.as_deref().unwrap_or_default();
    if rows.len() > 256 {
        return Err(tr("a-maximum-of-256-shortcuts-can-be-registered").into());
    }
    let mut assigned = HashMap::new();
    for row in rows {
        let Some(&(_, label, _)) = COMMANDS.iter().find(|c| c.0 == row.action) else {
            return Err(format!("Unknown shortcut command: {}", row.action));
        };
        validate_chord(&row.action, &row.key)?;
        if let Some(other) = assigned.insert(canonical_chord_for_platform(&row.key, macos), label) {
            return Err(format!(
                "{} conflicts: {} / {}.",
                shortcut_label(&row.key),
                tr(other),
                tr(label)
            ));
        }
    }
    Ok(())
}
pub(crate) fn validate_chord(id: &str, chord: &str) -> Result<(), String> {
    // Accept only the canonical order and key names produced by key capture.
    let mut rest = chord;
    for modifier in ["Primary+", "Ctrl+", "Alt+", "Shift+"] {
        rest = rest.strip_prefix(modifier).unwrap_or(rest);
    }
    if !KEY_NAMES.contains(&rest)
        || (rest == "Escape" && (id != "close-window" || chord != "Escape"))
    {
        return Err(format!(
            "Invalid or reserved shortcut: {}",
            shortcut_label(chord)
        ));
    }
    Ok(())
}
const KEY_NAMES: &[&str] = &[
    "Down",
    "Left",
    "Right",
    "Up",
    "Escape",
    "Tab",
    "Backspace",
    "Enter",
    "Insert",
    "Delete",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Copy",
    "Cut",
    "Paste",
    "Space",
    "Colon",
    "Comma",
    "Minus",
    "Period",
    "Plus",
    "Equals",
    "Semicolon",
    "Backslash",
    "Slash",
    "Pipe",
    "Questionmark",
    "Exclamationmark",
    "OpenBracket",
    "CloseBracket",
    "OpenCurlyBracket",
    "CloseCurlyBracket",
    "Backtick",
    "Quote",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "F13",
    "F14",
    "F15",
    "F16",
    "F17",
    "F18",
    "F19",
    "F20",
    "F21",
    "F22",
    "F23",
    "F24",
    "F25",
    "F26",
    "F27",
    "F28",
    "F29",
    "F30",
    "F31",
    "F32",
    "F33",
    "F34",
    "F35",
];

pub(crate) fn mobile_supported(command: &str) -> bool {
    // Phone layout does not remove the desktop host's folder access. Native
    // mobile pickers currently stage individual documents without saved access.
    if cfg!(any(target_os = "ios", target_os = "android")) && matches!(command, "folder" | "recent")
    {
        return false;
    }
    !matches!(
        command,
        "new-window"
            | "reveal"
            | "move"
            | "trash"
            | "undo-trash"
            | "close-window"
            | "close-all"
            | "minimize"
            | "maximize"
            | "fullscreen"
            | "export"
            | "import"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_keys_match_keys_exposed_by_the_input_adapter() {
        let expected: std::collections::BTreeSet<_> = eframe::egui::Key::ALL
            .iter()
            .map(|key| key.name())
            .collect();
        assert_eq!(
            KEY_NAMES
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            expected
        );
    }
}
