//! Portable settings schema; no UI or persistence handles.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

const VERSION: u32 = 1;
pub(crate) const MAX_RECENT: usize = 500;
/// Allowed values, shared by validation, the settings screen and commands.
pub(crate) const RECENT_LIMIT: RangeInclusive<usize> = 1..=MAX_RECENT;
/// Every duration setting is in milliseconds and moves in 0.1 s steps.
pub(crate) const TIME_MS: RangeInclusive<u32> = 100..=100_000;
pub(crate) const TIME_STEP_MS: u32 = 100;
pub(crate) const SMALL_IMAGE_PERCENT: RangeInclusive<u32> = 50..=2000;
pub(crate) const CACHE_IMAGE_LIMIT: RangeInclusive<usize> = 2..=10_000;
pub(crate) const CACHE_MEMORY_MB: RangeInclusive<u32> = 32..=4096;
pub(crate) const DISK_CACHE_MB: RangeInclusive<u32> = 32..=16_384;
pub(crate) const METADATA_FONT_SIZE: RangeInclusive<u32> = 10..=32;
pub(crate) const SORT_BY: &[&str] = &["natural", "name", "modified", "size"];
pub(crate) const VIEW_MODES: &[&str] = &["fit", "fill", "actual", "width", "height"];
/// Shortcut rows, and the length of each command ID and chord.
pub(crate) const MAX_SHORTCUTS: usize = 256;
const MAX_SHORTCUT_TEXT: usize = 128;

fn default_sort_by() -> String {
    "natural".into()
}
fn default_checker() -> bool {
    true
}
fn default_metadata_font_size() -> u32 {
    12
}
fn default_disk_cache_enabled() -> bool {
    true
}
fn default_disk_cache_mb() -> u32 {
    1024
}
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
fn default_orientation() -> String {
    "auto".into()
}
pub(crate) const ORIENTATIONS: &[&str] = &["auto", "portrait", "landscape"];
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

/// Wheel gestures either turn pages or zoom.
pub(crate) const WHEEL_ACTIONS: &[&str] = &["page", "zoom", "none"];
/// Primary-button drags either move the window or pan the image. A pan that
/// starts on the background instead of an image moves the window.
pub(crate) const DRAG_ACTIONS: &[&str] = &["window", "pan", "none"];
/// Clicks open the menu, do nothing, or run any command ID.
pub(crate) const CLICK_MENU: &str = "menu";
pub(crate) const MOUSE_NONE: &str = "none";

/// Desktop mouse gestures. "Primary" is Ctrl on Windows/Linux and ⌘ on macOS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MouseSettings {
    pub wheel: String,
    pub primary_wheel: String,
    pub reverse_wheel: bool,
    pub right_click: String,
    pub middle_click: String,
    /// Left-button double click.
    pub double_click: String,
    pub back_button: String,
    pub forward_button: String,
    pub drag: String,
    pub primary_drag: String,
}

impl Default for MouseSettings {
    fn default() -> Self {
        Self {
            wheel: "zoom".into(),
            primary_wheel: MOUSE_NONE.into(),
            reverse_wheel: false,
            right_click: CLICK_MENU.into(),
            middle_click: "recent".into(),
            double_click: "fullscreen".into(),
            back_button: "prev".into(),
            forward_button: "next".into(),
            drag: "pan".into(),
            primary_drag: MOUSE_NONE.into(),
        }
    }
}

pub(crate) fn valid_click_action(action: &str) -> bool {
    matches!(action, CLICK_MENU | MOUSE_NONE)
        || crate::core::commands::COMMANDS
            .iter()
            .any(|command| command.0 == action)
}

impl MouseSettings {
    /// Unknown values (from a newer or hand-edited file) fall back per gesture.
    pub(crate) fn normalize(&mut self) {
        let defaults = Self::default();
        for (value, default, allowed) in [
            (&mut self.wheel, defaults.wheel, WHEEL_ACTIONS),
            (
                &mut self.primary_wheel,
                defaults.primary_wheel,
                WHEEL_ACTIONS,
            ),
            (&mut self.drag, defaults.drag, DRAG_ACTIONS),
            (&mut self.primary_drag, defaults.primary_drag, DRAG_ACTIONS),
        ] {
            if !allowed.contains(&value.as_str()) {
                *value = default;
            }
        }
        for (value, default) in [
            (&mut self.right_click, defaults.right_click),
            (&mut self.middle_click, defaults.middle_click),
            (&mut self.double_click, defaults.double_click),
            (&mut self.back_button, defaults.back_button),
            (&mut self.forward_button, defaults.forward_button),
        ] {
            if !valid_click_action(value) {
                *value = default;
            }
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        let mut normalized = self.clone();
        normalized.normalize();
        normalized == *self
    }
}

/// Mobile gestures are stored separately from desktop mouse bindings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TouchSettings {
    pub swipe_navigation: bool,
    pub pinch_zoom: bool,
    pub one_finger_pan: bool,
    pub two_finger_pan: bool,
    pub double_tap: String,
    pub long_press: String,
}

impl Default for TouchSettings {
    fn default() -> Self {
        Self {
            swipe_navigation: true,
            pinch_zoom: true,
            one_finger_pan: true,
            two_finger_pan: true,
            double_tap: "fit".into(),
            long_press: CLICK_MENU.into(),
        }
    }
}

pub(crate) fn valid_touch_action(action: &str) -> bool {
    valid_click_action(action) && crate::core::commands::mobile_supported(action)
}

impl TouchSettings {
    pub(crate) fn normalize(&mut self) {
        let defaults = Self::default();
        for (value, default) in [
            (&mut self.double_tap, defaults.double_tap),
            (&mut self.long_press, defaults.long_press),
        ] {
            if !valid_touch_action(value) {
                *value = default;
            }
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        valid_touch_action(&self.double_tap) && valid_touch_action(&self.long_press)
    }
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
    #[serde(default = "default_sort_by")]
    pub sort_by: String,
    #[serde(default)]
    pub sort_descending: bool,
    #[serde(default)]
    pub background_rgb: [u8; 3],
    #[serde(default = "default_checker")]
    pub transparency_checker: bool,
    #[serde(default = "default_metadata_font_size")]
    pub metadata_font_size: u32,
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default = "default_disk_cache_enabled")]
    pub disk_cache_enabled: bool,
    #[serde(default = "default_disk_cache_mb")]
    pub disk_cache_mb: u32,
    /// Phone screen orientation lock: auto, portrait or landscape.
    #[serde(default = "default_orientation")]
    pub orientation: String,
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
    #[serde(default)]
    pub mouse: MouseSettings,
    #[serde(default)]
    pub touch: TouchSettings,
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
    #[serde(default)]
    pub settings_shortcut_migrated: bool,
    #[serde(default)]
    pub bookmark_shortcut_migrated: bool,
    #[serde(default)]
    pub single_step_shortcuts_migrated: bool,
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
            sort_by: default_sort_by(),
            sort_descending: false,
            background_rgb: [0, 0, 0],
            transparency_checker: default_checker(),
            metadata_font_size: default_metadata_font_size(),
            reduce_motion: false,
            disk_cache_enabled: default_disk_cache_enabled(),
            disk_cache_mb: default_disk_cache_mb(),
            orientation: default_orientation(),
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
            mouse: MouseSettings::default(),
            touch: TouchSettings::default(),
            shortcuts: BTreeMap::new(),
            keyboard_shortcuts: Some(crate::core::commands::default_shortcut_rows()),
            close_all_shortcut_migrated: true,
            help_shortcut_migrated: true,
            slideshow_shortcuts_migrated: true,
            trash_shortcut_migrated: true,
            move_shortcut_migrated: true,
            settings_shortcut_migrated: true,
            bookmark_shortcut_migrated: true,
            single_step_shortcuts_migrated: true,
        }
    }
}

const SETTINGS_CTRL_S: &str = "Ctrl+S";

fn is_settings_ctrl_s(row: &KeyboardShortcut) -> bool {
    row.action == "settings" && row.key == SETTINGS_CTRL_S
}

impl Settings {
    pub(crate) fn normalize(&mut self) {
        // Every load, save and import path normalizes here first.
        self.mouse.normalize();
        self.touch.normalize();
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
        if !self.single_step_shortcuts_migrated {
            current_defaults
                .retain(|row| !matches!(row.action.as_str(), "page-prev" | "page-next"));
        }
        if !self.move_shortcut_migrated {
            current_defaults.retain(|row| row.action != "move");
        }
        if !self.settings_shortcut_migrated {
            current_defaults.retain(|row| !is_settings_ctrl_s(row));
        }
        if !self.bookmark_shortcut_migrated {
            current_defaults.retain(|row| row.action != "bookmark");
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
            if !crate::core::NATIVE_MOBILE
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
        if !self.settings_shortcut_migrated {
            // Add Ctrl+S beside the existing settings binding only while that
            // chord is still free; empty lists stay deliberately empty.
            let chord = crate::core::commands::canonical_chord(SETTINGS_CTRL_S);
            if let Some(rows) = &mut self.keyboard_shortcuts
                && !rows.is_empty()
                && !rows
                    .iter()
                    .any(|row| crate::core::commands::canonical_chord(&row.key) == chord)
            {
                // Keep the default order: right after the existing settings key.
                let at = rows
                    .iter()
                    .rposition(|row| row.action == "settings")
                    .map_or(rows.len(), |index| index + 1);
                rows.insert(
                    at,
                    KeyboardShortcut {
                        action: "settings".into(),
                        key: SETTINGS_CTRL_S.into(),
                    },
                );
            }
            self.settings_shortcut_migrated = true;
        }
        if !self.bookmark_shortcut_migrated {
            // Add B for bookmarks only while that key is free; empty lists stay
            // deliberately empty and a removed binding is never restored.
            if !crate::core::NATIVE_MOBILE
                && let Some(rows) = &mut self.keyboard_shortcuts
                && !rows.is_empty()
                && !rows
                    .iter()
                    .any(|row| row.action == "bookmark" || row.key == "B")
            {
                // Keep the default order: right after the recent-folders key.
                let at = rows
                    .iter()
                    .rposition(|row| row.action == "recent")
                    .map_or(rows.len(), |index| index + 1);
                rows.insert(
                    at,
                    KeyboardShortcut {
                        action: "bookmark".into(),
                        key: "B".into(),
                    },
                );
            }
            self.bookmark_shortcut_migrated = true;
        }
        if !self.single_step_shortcuts_migrated {
            if let Some(rows) = &mut self.keyboard_shortcuts
                && !rows.is_empty()
            {
                for action in ["page-prev", "page-next"] {
                    // Preserve an existing assignment for the command, and add
                    // only unclaimed chords. Empty/deleted lists stay empty.
                    if rows.iter().any(|row| row.action == action) {
                        continue;
                    }
                    for row in crate::core::commands::default_shortcut_rows()
                        .into_iter()
                        .filter(|row| row.action == action)
                    {
                        let chord = crate::core::commands::canonical_chord(&row.key);
                        if rows.len() < MAX_SHORTCUTS
                            && !rows.iter().any(|existing| {
                                crate::core::commands::canonical_chord(&existing.key) == chord
                            })
                        {
                            let at = rows
                                .iter()
                                .rposition(|row| {
                                    matches!(
                                        row.action.as_str(),
                                        "next" | "page-prev" | "page-next"
                                    )
                                })
                                .map_or(rows.len(), |index| index + 1);
                            rows.insert(at, row);
                        }
                    }
                }
            }
            self.single_step_shortcuts_migrated = true;
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if !SMALL_IMAGE_PERCENT.contains(&self.small_image_percent) {
            return Err("Small image maximum size must be 50–2000%.".into());
        }
        if !CACHE_IMAGE_LIMIT.contains(&self.cache_image_limit) {
            return Err("Cache image limit must be 2–10000.".into());
        }
        if !CACHE_MEMORY_MB.contains(&self.cache_memory_mb) {
            return Err("Cache memory limit must be 32–4096 MiB.".into());
        }
        if self.schema_version != VERSION {
            return Err(format!(
                "Unsupported settings version ({}).",
                self.schema_version
            ));
        }
        if !SORT_BY.contains(&self.sort_by.as_str()) {
            return Err("Invalid image sort order.".into());
        }
        if !METADATA_FONT_SIZE.contains(&self.metadata_font_size) {
            return Err("Metadata font size must be 10–32 points.".into());
        }
        if !DISK_CACHE_MB.contains(&self.disk_cache_mb) {
            return Err("Disk cache limit must be 32–16384 MiB.".into());
        }
        if !VIEW_MODES.contains(&self.view_mode.as_str()) {
            return Err("Invalid image fit mode.".into());
        }
        if !ORIENTATIONS.contains(&self.orientation.as_str()) {
            return Err("Invalid screen orientation.".into());
        }
        if [
            self.slide_ms,
            self.key_delay_ms,
            self.key_repeat_ms,
            self.toast_ms,
            self.cursor_hide_ms,
        ]
        .iter()
        .any(|ms| !TIME_MS.contains(ms))
        {
            return Err("Time settings must be 0.1–100.0 seconds.".into());
        }
        if !RECENT_LIMIT.contains(&self.recent_move_limit) {
            return Err(format!("Recent move limit must be 1–{MAX_RECENT}."));
        }
        if !RECENT_LIMIT.contains(&self.recent_limit) {
            return Err(format!("Recent item limit must be 1–{MAX_RECENT}."));
        }
        if !self.mouse.is_valid() {
            return Err("Invalid mouse settings.".into());
        }
        if !self.touch.is_valid() {
            return Err("Invalid touch settings.".into());
        }
        let text_ok =
            |text: &str| text.len() <= MAX_SHORTCUT_TEXT && !text.chars().any(char::is_control);
        // The legacy map may hold an empty chord for an unbound command.
        let legacy_ok = self.shortcuts.len() <= MAX_SHORTCUTS
            && self
                .shortcuts
                .iter()
                .all(|(id, chord)| !id.is_empty() && text_ok(id) && text_ok(chord));
        let rows_ok = self.keyboard_shortcuts.as_ref().is_none_or(|rows| {
            rows.len() <= MAX_SHORTCUTS
                && rows.iter().all(|row| {
                    !row.action.is_empty()
                        && !row.key.is_empty()
                        && text_ok(&row.action)
                        && text_ok(&row.key)
                })
        });
        if !legacy_ok || !rows_ok {
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
            sort_by,
            sort_descending,
            background_rgb,
            transparency_checker,
            metadata_font_size,
            reduce_motion,
            disk_cache_enabled,
            disk_cache_mb,
            orientation,
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
            mouse,
            touch,
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
