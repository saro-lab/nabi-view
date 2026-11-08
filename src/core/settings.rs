//! Portable settings schema; no UI or persistence handles.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
const VERSION: u32 = 1;
pub(crate) const MAX_RECENT: usize = 500;
fn default_recent_moves() -> usize {
    10
}
fn default_slide() -> u32 {
    1000
}
fn default_delay() -> u32 {
    300
}
fn default_small_image() -> u32 {
    400
}
fn default_cache_image_limit() -> usize {
    1000
}
fn default_cache_memory_mb() -> u32 {
    2048
}
fn default_toast() -> u32 {
    1000
}
fn default_repeat() -> u32 {
    100
}
fn default_prevent_display_sleep() -> bool {
    true
}
fn default_cursor_hide() -> u32 {
    2000
}
pub const MIN_WINDOW_SIZE: [f32; 2] = [560., 420.];
pub const DEFAULT_WINDOW_SIZE: [f32; 2] = [1080., 760.];

pub fn valid_window_size(size: [f32; 2]) -> bool {
    size.into_iter()
        .zip(MIN_WINDOW_SIZE)
        .all(|(value, min)| value.is_finite() && (min..=16_384.).contains(&value))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardShortcut {
    pub action: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    #[serde(default)]
    pub language: crate::i18n::Language,
    #[serde(default = "default_prevent_display_sleep")]
    pub prevent_display_sleep: bool,
    #[serde(default = "default_cursor_hide")]
    pub cursor_hide_ms: u32,
    pub spread: bool,
    pub rtl: bool,
    pub cover: bool,
    #[serde(default)]
    pub blackout_other_monitors: bool,
    #[serde(default)]
    pub flip_horizontal: bool,
    #[serde(default)]
    pub flip_vertical: bool,
    #[serde(default)]
    pub show_thumbnails: bool,
    pub view_mode: String,
    #[serde(default = "default_slide")]
    pub slide_ms: u32,
    #[serde(default = "default_delay")]
    pub key_delay_ms: u32,
    #[serde(default = "default_repeat")]
    pub key_repeat_ms: u32,
    #[serde(default = "default_toast")]
    pub toast_ms: u32,
    #[serde(default = "default_small_image")]
    pub small_image_percent: u32,
    #[serde(default = "default_cache_image_limit")]
    pub cache_image_limit: usize,
    #[serde(default = "default_cache_memory_mb")]
    pub cache_memory_mb: u32,
    pub remember_recent: bool,
    pub recent_limit: usize,
    #[serde(default = "default_recent_moves")]
    pub recent_move_limit: usize,
    pub shortcuts: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_shortcuts: Option<Vec<KeyboardShortcut>>,
    #[serde(default)]
    pub close_all_shortcut_migrated: bool,
    #[serde(default)]
    pub help_shortcut_migrated: bool,
    #[serde(default)]
    pub slideshow_shortcuts_migrated: bool,
    #[serde(default)]
    pub trash_shortcut_migrated: bool,
    #[serde(default)]
    pub move_shortcut_migrated: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: VERSION,
            language: Default::default(),
            prevent_display_sleep: default_prevent_display_sleep(),
            cursor_hide_ms: default_cursor_hide(),
            spread: false,
            rtl: false,
            cover: false,
            blackout_other_monitors: false,
            flip_horizontal: false,
            flip_vertical: false,
            show_thumbnails: false,
            view_mode: "fit".into(),
            slide_ms: default_slide(),
            key_delay_ms: default_delay(),
            key_repeat_ms: default_repeat(),
            toast_ms: default_toast(),
            small_image_percent: default_small_image(),
            cache_image_limit: default_cache_image_limit(),
            cache_memory_mb: default_cache_memory_mb(),
            remember_recent: true,
            recent_limit: 50,
            recent_move_limit: default_recent_moves(),
            shortcuts: BTreeMap::new(),
            keyboard_shortcuts: Some(crate::core::commands::default_shortcut_rows()),
            close_all_shortcut_migrated: true,
            help_shortcut_migrated: true,
            slideshow_shortcuts_migrated: true,
            trash_shortcut_migrated: true,
            move_shortcut_migrated: true,
        }
    }
}

impl Settings {
    pub(crate) fn normalize_shortcuts(&mut self) {
        if self.keyboard_shortcuts.is_none() {
            self.keyboard_shortcuts = Some(
                self.shortcuts
                    .iter()
                    .filter(|(action, key)| {
                        !key.is_empty() && !matches!(action.as_str(), "sample" | "strip")
                    })
                    .map(|(action, key)| KeyboardShortcut {
                        action: action.clone(),
                        key: key.clone(),
                    })
                    .collect(),
            );
        }
        if let Some(rows) = &mut self.keyboard_shortcuts {
            for row in rows {
                if row.action == "help" {
                    row.action = "about".into();
                }
            }
        }
        self.shortcuts.clear();
        let mut current_defaults = crate::core::commands::default_shortcut_rows();
        if !self.move_shortcut_migrated {
            current_defaults.retain(|row| row.action != "move");
        }
        let mut defaults = current_defaults.clone();
        // Run earlier upgrades against the historical defaults first.
        if !self.trash_shortcut_migrated {
            for row in &mut defaults {
                if row.action == "trash" && row.key == "Primary+E" {
                    row.key = "Primary+S".into();
                }
            }
        }
        // Earlier migrations must compare against the defaults that existed
        // before the slideshow interval commands were added.
        let previous_defaults: Vec<_> = defaults
            .iter()
            .filter(|r| {
                self.slideshow_shortcuts_migrated
                    || !matches!(
                        r.action.as_str(),
                        "slideshow-decrease" | "slideshow-increase"
                    )
            })
            .cloned()
            .collect();
        if !self.help_shortcut_migrated {
            // Preserve custom/empty lists and never reclaim a deleted help binding.
            for without_close_all in [false, true] {
                if without_close_all && self.close_all_shortcut_migrated {
                    continue;
                }
                let upgraded: Vec<_> = previous_defaults
                    .iter()
                    .filter(|r| !without_close_all || r.action != "close-all")
                    .cloned()
                    .collect();
                let previous: Vec<_> = upgraded
                    .iter()
                    .filter(|r| r.action != "about")
                    .cloned()
                    .collect();
                if self.keyboard_shortcuts.as_ref() == Some(&previous) {
                    self.keyboard_shortcuts = Some(upgraded);
                    break;
                }
            }
            self.help_shortcut_migrated = true;
        }
        if !self.close_all_shortcut_migrated {
            // Upgrade only an untouched previous default list. Custom/empty
            // bindings keep their meaning, and later deletion stays deleted.
            let previous: Vec<_> = previous_defaults
                .iter()
                .filter(|r| r.action != "close-all")
                .cloned()
                .collect();
            if self.keyboard_shortcuts.as_ref() == Some(&previous) {
                self.keyboard_shortcuts = Some(previous_defaults.clone());
            }
            self.close_all_shortcut_migrated = true;
        }
        if !self.slideshow_shortcuts_migrated {
            if self.keyboard_shortcuts.as_ref() == Some(&previous_defaults) {
                self.keyboard_shortcuts = Some(defaults.clone());
            }
            self.slideshow_shortcuts_migrated = true;
        }
        if !self.trash_shortcut_migrated {
            // Only untouched defaults are upgraded; preserve edits, conflicts,
            // removed bindings and deliberately empty lists.
            if self.keyboard_shortcuts.as_ref() == Some(&defaults) {
                self.keyboard_shortcuts = Some(current_defaults);
            }
            self.trash_shortcut_migrated = true;
        }
        if !self.move_shortcut_migrated {
            if !cfg!(any(target_os = "ios", target_os = "android"))
                && let Some(rows) = &mut self.keyboard_shortcuts
                && !rows.is_empty()
                && !rows
                    .iter()
                    .any(|row| row.action == "move" || row.key == "M")
            {
                rows.push(KeyboardShortcut {
                    action: "move".into(),
                    key: "M".into(),
                });
            }
            self.move_shortcut_migrated = true;
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(50..=500).contains(&self.small_image_percent) {
            return Err("Small image maximum size must be 50–500%.".into());
        }
        if !(2..=10_000).contains(&self.cache_image_limit) {
            return Err("Cache image limit must be 2–10000.".into());
        }
        if !(32..=4096).contains(&self.cache_memory_mb) {
            return Err("Cache memory limit must be 32–4096 MiB.".into());
        }
        if self.schema_version != VERSION {
            return Err(format!(
                "Unsupported settings version ({}).",
                self.schema_version
            ));
        }
        if !matches!(
            self.view_mode.as_str(),
            "fit" | "fill" | "actual" | "width" | "height"
        ) {
            return Err("Invalid image fit mode.".into());
        }
        if [
            self.slide_ms,
            self.key_delay_ms,
            self.key_repeat_ms,
            self.toast_ms,
            self.cursor_hide_ms,
        ]
        .iter()
        .any(|ms| !(100..=100_000).contains(ms))
        {
            return Err("Time settings must be 0.1–100.0 seconds.".into());
        }
        if !(1..=MAX_RECENT).contains(&self.recent_move_limit) {
            return Err(format!("Recent move limit must be 1–{MAX_RECENT}."));
        }
        if !(1..=MAX_RECENT).contains(&self.recent_limit) {
            return Err(format!("Recent item limit must be 1–{MAX_RECENT}."));
        }
        if self.shortcuts.len() > 256
            || self.shortcuts.iter().any(|(id, chord)| {
                id.is_empty()
                    || id.len() > 128
                    || chord.len() > 128
                    || id.chars().any(char::is_control)
                    || chord.chars().any(char::is_control)
            })
        {
            return Err("Invalid shortcut settings format.".into());
        }
        if self.keyboard_shortcuts.as_ref().is_some_and(|rows| {
            rows.len() > 256
                || rows.iter().any(|row| {
                    row.action.is_empty()
                        || row.action.len() > 128
                        || row.key.is_empty()
                        || row.key.len() > 128
                        || row.action.chars().any(char::is_control)
                        || row.key.chars().any(char::is_control)
                })
        }) {
            return Err("Invalid shortcut settings format.".into());
        }
        crate::core::commands::validate_keys(self)
    }

    pub(crate) fn merge_changes(&mut self, before: &Self, after: &Self) {
        macro_rules! merge {
            ($($field:ident),+) => {
                $(if before.$field != after.$field {
                    self.$field.clone_from(&after.$field);
                })+
            };
        }
        merge!(
            language,
            prevent_display_sleep,
            cursor_hide_ms,
            spread,
            rtl,
            cover,
            blackout_other_monitors,
            flip_horizontal,
            flip_vertical,
            show_thumbnails,
            view_mode,
            slide_ms,
            key_delay_ms,
            key_repeat_ms,
            toast_ms,
            small_image_percent,
            cache_image_limit,
            cache_memory_mb,
            remember_recent,
            recent_limit,
            recent_move_limit,
            keyboard_shortcuts
        );
        for (command, chord) in &after.shortcuts {
            if before.shortcuts.get(command) != Some(chord) {
                self.shortcuts.insert(command.clone(), chord.clone());
            }
        }
        for command in before.shortcuts.keys() {
            if !after.shortcuts.contains_key(command) {
                self.shortcuts.remove(command);
            }
        }
    }
}
