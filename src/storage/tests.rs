#[test]
fn trash_default_upgrade_preserves_custom_deleted_and_previously_migrated_bindings() {
    let defaults = crate::core::commands::default_shortcut_rows();
    assert!(
        defaults
            .iter()
            .any(|row| row.action == "trash" && row.key == "Primary+E")
    );
    assert!(
        !defaults
            .iter()
            .any(|row| row.action == "trash" && row.key == "Primary+S")
    );
    // Releases with Primary+S for trash predate the settings Ctrl+S default.
    let mut old: Vec<_> = defaults
        .iter()
        .filter(|row| !(row.action == "settings" && row.key == "Ctrl+S"))
        .cloned()
        .collect();
    for row in &mut old {
        if row.action == "trash" && row.key == "Primary+E" {
            row.key = "Primary+S".into();
        }
    }
    for older_release in [false, true] {
        let mut json = serde_json::to_value(Settings {
            keyboard_shortcuts: Some(
                old.iter()
                    .filter(|row| {
                        !older_release
                            || !matches!(
                                row.action.as_str(),
                                "about" | "close-all" | "slideshow-increase" | "slideshow-decrease"
                            )
                    })
                    .cloned()
                    .collect(),
            ),
            help_shortcut_migrated: !older_release,
            close_all_shortcut_migrated: !older_release,
            slideshow_shortcuts_migrated: !older_release,
            ..Default::default()
        })
        .unwrap();
        for field in ["trash_shortcut_migrated", "settings_shortcut_migrated"] {
            json.as_object_mut().unwrap().remove(field);
        }
        let mut settings: Settings = serde_json::from_value(json).unwrap();
        settings.normalize();
        assert_eq!(settings.keyboard_shortcuts, Some(defaults.clone()));
        settings.validate().unwrap();
        settings.keyboard_shortcuts = Some(old.clone());
        let mut reloaded: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        reloaded.normalize();
        assert_eq!(reloaded.keyboard_shortcuts, Some(old.clone()));
    }
    let mut conflicting = old.clone();
    conflicting.push(KeyboardShortcut {
        action: "next".into(),
        key: "Primary+E".into(),
    });
    let deleted = old
        .iter()
        .filter(|row| row.key != "Primary+S")
        .cloned()
        .collect();
    for rows in [vec![], conflicting, deleted] {
        let mut settings = Settings {
            keyboard_shortcuts: Some(rows.clone()),
            trash_shortcut_migrated: false,
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.keyboard_shortcuts, Some(rows));
        settings.validate().unwrap();
    }
}

#[test]
fn slideshow_defaults_upgrade_old_versions_and_preserve_custom_or_deleted_keys() {
    let defaults = crate::core::commands::default_shortcut_rows();
    let old: Vec<_> = defaults
        .iter()
        .filter(|r| {
            !matches!(
                r.action.as_str(),
                "slideshow-decrease" | "slideshow-increase"
            )
        })
        .cloned()
        .collect();
    for without_help in [false, true] {
        for without_close_all in [false, true] {
            let mut json = serde_json::to_value(Settings::default()).unwrap();
            json.as_object_mut()
                .unwrap()
                .remove("slideshow_shortcuts_migrated");
            json["help_shortcut_migrated"] = (!without_help).into();
            json["close_all_shortcut_migrated"] = (!without_close_all).into();
            json["keyboard_shortcuts"] = serde_json::to_value(
                old.iter()
                    .filter(|r| {
                        (!without_help || r.action != "about")
                            && (!without_close_all || r.action != "close-all")
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let mut settings: Settings = serde_json::from_value(json).unwrap();
            settings.normalize();
            assert_eq!(settings.keyboard_shortcuts.as_ref(), Some(&defaults));
            settings.keyboard_shortcuts = Some(old.clone());
            let json = serde_json::to_vec(&settings).unwrap();
            let mut reloaded: Settings = serde_json::from_slice(&json).unwrap();
            reloaded.normalize();
            assert_eq!(
                reloaded.keyboard_shortcuts,
                Some(old.clone()),
                "Deleted keys must stay deleted"
            );
        }
    }
    for rows in [
        vec![],
        vec![KeyboardShortcut {
            action: "next".into(),
            key: "Comma".into(),
        }],
    ] {
        let mut settings = Settings {
            keyboard_shortcuts: Some(rows.clone()),
            slideshow_shortcuts_migrated: false,
            ..Settings::default()
        };
        settings.normalize();
        assert_eq!(settings.keyboard_shortcuts, Some(rows));
        settings.validate().unwrap();
    }
}

#[test]
fn legacy_help_binding_moves_to_about_without_changing_its_key() {
    let mut settings = Settings {
        keyboard_shortcuts: Some(vec![KeyboardShortcut {
            action: "help".into(),
            key: "F2".into(),
        }]),
        ..Settings::default()
    };
    settings.normalize();
    assert_eq!(
        settings.keyboard_shortcuts,
        Some(vec![KeyboardShortcut {
            action: "about".into(),
            key: "F2".into()
        },])
    );
    settings.normalize();
    assert_eq!(settings.keyboard_shortcuts.as_ref().unwrap().len(), 1);
}

#[test]
fn help_default_upgrade_handles_old_defaults_and_preserves_custom_bindings() {
    let defaults = crate::core::commands::default_shortcut_rows();
    for without_close_all in [false, true] {
        let old: Vec<_> = defaults
            .iter()
            .filter(|r| r.action != "about" && (!without_close_all || r.action != "close-all"))
            .cloned()
            .collect();
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json.as_object_mut()
            .unwrap()
            .remove("help_shortcut_migrated");
        json["keyboard_shortcuts"] = serde_json::to_value(old).unwrap();
        json["close_all_shortcut_migrated"] = (!without_close_all).into();
        let mut settings: Settings = serde_json::from_value(json).unwrap();
        settings.normalize();
        assert_eq!(settings.keyboard_shortcuts.as_ref(), Some(&defaults));
        settings
            .keyboard_shortcuts
            .as_mut()
            .unwrap()
            .retain(|r| r.action != "about");
        let deleted = settings.keyboard_shortcuts.clone();
        settings.normalize();
        assert_eq!(settings.keyboard_shortcuts, deleted);
    }
    for rows in [
        vec![],
        vec![KeyboardShortcut {
            action: "next".into(),
            key: "F1".into(),
        }],
    ] {
        let mut settings = Settings {
            keyboard_shortcuts: Some(rows.clone()),
            help_shortcut_migrated: false,
            ..Settings::default()
        };
        settings.normalize();
        assert_eq!(settings.keyboard_shortcuts, Some(rows));
    }
}

#[test]
fn close_all_default_upgrade_runs_once_and_preserves_custom_bindings() {
    let defaults = crate::core::commands::default_shortcut_rows();
    let old: Vec<_> = defaults
        .iter()
        .filter(|r| r.action != "close-all")
        .cloned()
        .collect();
    let mut settings = Settings {
        keyboard_shortcuts: Some(old.clone()),
        close_all_shortcut_migrated: false,
        ..Settings::default()
    };
    settings.normalize();
    assert_eq!(settings.keyboard_shortcuts, Some(defaults));
    settings.keyboard_shortcuts = Some(old.clone());
    settings.normalize();
    assert_eq!(settings.keyboard_shortcuts, Some(old));
    for rows in [
        vec![],
        vec![KeyboardShortcut {
            action: "next".into(),
            key: "Primary+Q".into(),
        }],
    ] {
        settings.close_all_shortcut_migrated = false;
        settings.keyboard_shortcuts = Some(rows.clone());
        settings.normalize();
        assert_eq!(settings.keyboard_shortcuts, Some(rows));
    }
}

#[test]
fn monitor_blackout_migrates_and_round_trips_and_merges() {
    let before = Settings::default();
    let mut json = serde_json::to_value(&before).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("blackout_other_monitors");
    let old: Settings = serde_json::from_value(json).unwrap();
    assert!(!old.blackout_other_monitors);
    let mut after = before.clone();
    after.blackout_other_monitors = true;
    let imported: Settings = serde_json::from_str(&serde_json::to_string(&after).unwrap()).unwrap();
    assert!(imported.blackout_other_monitors);
    let mut merged = before.clone();
    merged.merge_changes(&before, &imported);
    assert!(merged.blackout_other_monitors);
}

#[test]
fn display_sleep_defaults_on_and_preserves_an_explicit_off_setting() {
    let before = Settings::default();
    assert!(before.prevent_display_sleep);
    let mut json = serde_json::to_value(&before).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("prevent_display_sleep");
    let old: Settings = serde_json::from_value(json).unwrap();
    assert!(old.prevent_display_sleep);
    let mut after = before.clone();
    after.prevent_display_sleep = false;
    let imported: Settings = serde_json::from_str(&serde_json::to_string(&after).unwrap()).unwrap();
    assert!(!imported.prevent_display_sleep);
    let mut merged = before.clone();
    merged.language = crate::i18n::Language::English;
    merged.merge_changes(&before, &imported);
    assert!(!merged.prevent_display_sleep);
    assert_eq!(merged.language, crate::i18n::Language::English);
}

#[test]
fn cursor_delay_migrates_validates_and_round_trips() {
    let before = Settings::default();
    assert_eq!(before.cursor_hide_ms, 2000);
    let mut json = serde_json::to_value(&before).unwrap();
    json.as_object_mut().unwrap().remove("cursor_hide_ms");
    let old: Settings = serde_json::from_value(json).unwrap();
    assert_eq!(old.cursor_hide_ms, 2000);
    let mut after = before.clone();
    for value in [100, 2000, 100_000] {
        after.cursor_hide_ms = value;
        after.validate().unwrap();
    }
    for value in [0, 99, 100_001] {
        after.cursor_hide_ms = value;
        assert!(after.validate().is_err());
    }
    after.cursor_hide_ms = 300;
    let imported: Settings = serde_json::from_str(&serde_json::to_string(&after).unwrap()).unwrap();
    let mut merged = before.clone();
    merged.merge_changes(&before, &imported);
    assert_eq!(merged.cursor_hide_ms, 300);
}

#[test]
fn settings_validate_and_legacy_files_get_defaults() {
    let mut settings = Settings::default();
    assert_eq!(
        (
            settings.slide_ms,
            settings.key_delay_ms,
            settings.key_repeat_ms
        ),
        (1000, 300, 100)
    );
    for value in [100, 100_000] {
        settings.slide_ms = value;
        settings.key_delay_ms = value;
        settings.key_repeat_ms = value;
        assert!(settings.validate().is_ok());
    }
    for value in [0, 99, 100_001] {
        settings.slide_ms = value;
        assert!(settings.validate().is_err());
    }
    let mut old = serde_json::to_value(Settings::default()).unwrap();
    assert!(old["keyboard_shortcuts"].is_array());
    for key in [
        "slide_ms",
        "key_delay_ms",
        "key_repeat_ms",
        "flip_horizontal",
        "flip_vertical",
        "cache_image_limit",
        "cache_memory_mb",
        "keyboard_shortcuts",
    ] {
        old.as_object_mut().unwrap().remove(key);
    }
    let mut migrated = serde_json::from_value::<Settings>(old).unwrap();
    assert!(migrated.keyboard_shortcuts.is_none());
    migrated.normalize();
    assert_eq!(
        migrated,
        Settings {
            keyboard_shortcuts: Some(Vec::new()),
            ..Settings::default()
        }
    );
}
use super::*;

fn isolated(root: &Path) -> Store {
    Store::in_directories(root.join("config"), root.join("data"))
}

#[test]
fn legacy_shortcuts_migrate_only_registered_keys_without_filling_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let store = isolated(dir.path());
    assert_eq!(store.load().unwrap(), Settings::default());
    let mut legacy = serde_json::to_value(Settings::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("keyboard_shortcuts");
    legacy["shortcuts"] = serde_json::json!({
        "new-window": "D", "next": "", "sample": "Q", "strip": "T"
    });
    fs::create_dir_all(store.settings.parent().unwrap()).unwrap();
    fs::write(&store.settings, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let migrated = store.load().unwrap();
    assert_eq!(
        migrated.keyboard_shortcuts,
        Some(vec![KeyboardShortcut {
            action: "new-window".into(),
            key: "D".into(),
        }])
    );
    assert!(migrated.shortcuts.is_empty());
    let exported = dir.path().join("migrated.json");
    store.export(&exported, &migrated).unwrap();
    assert_eq!(store.import(&exported).unwrap(), migrated);

    // Existing empty configuration is not a fresh installation: no keys
    // should appear merely because the current app has default assignments.
    legacy["shortcuts"] = serde_json::json!({});
    fs::write(&store.settings, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(store.load().unwrap().keyboard_shortcuts, Some(Vec::new()));

    // An explicit list, including an empty list, overrides all legacy values.
    legacy["keyboard_shortcuts"] = serde_json::json!([]);
    legacy["shortcuts"] = serde_json::json!({ "new-window": "D" });
    fs::write(&store.settings, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let empty = store.load().unwrap();
    assert_eq!(empty.keyboard_shortcuts, Some(Vec::new()));
    assert!(empty.shortcuts.is_empty());

    legacy.as_object_mut().unwrap().remove("keyboard_shortcuts");
    legacy["shortcuts"] = serde_json::json!({ "unknown-action": "J" });
    fs::write(&store.settings, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(store.load().is_err());
}

#[test]
fn window_size_is_device_local_and_survives_store_recreation() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    assert_eq!(store.load_window_size().unwrap(), None);
    assert!(
        !store.database.exists(),
        "Initial geometry read must not create a database"
    );
    store.save_window_size([1280., 800.]).unwrap();
    assert_eq!(
        isolated(root.path()).load_window_size().unwrap(),
        Some([1280., 800.])
    );
    store.clear_recent().unwrap();
    assert_eq!(store.load_window_size().unwrap(), Some([1280., 800.]));
    let exported = root.path().join("portable.json");
    store.export(&exported, &Settings::default()).unwrap();
    let json = fs::read_to_string(exported).unwrap();
    assert!(!json.contains("1280") && !json.contains("window_size") && !json.contains("width"));
}

#[test]
fn window_size_rejects_invalid_values_without_losing_last_valid_size() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    store.save_window_size([900., 600.]).unwrap();
    for size in [
        [0., 0.],
        [559., 420.],
        [560., 419.],
        [20_000., 600.],
        [f32::NAN, 600.],
        [900., f32::INFINITY],
    ] {
        assert!(store.save_window_size(size).is_err());
    }
    assert_eq!(store.load_window_size().unwrap(), Some([900., 600.]));
    store
        .connection()
        .unwrap()
        .execute("UPDATE window_state SET width=-1", [])
        .unwrap();
    assert_eq!(store.load_window_size().unwrap(), None);
}

#[test]
fn two_editors_keep_independent_preferences_and_shortcut_rows() {
    let dir = tempfile::tempdir().unwrap();
    let a = isolated(dir.path());
    let b = isolated(dir.path());
    let mut left = a.load().unwrap();
    let mut right = b.load().unwrap();
    left.spread = true;
    left.flip_horizontal = true;
    left.keyboard_shortcuts = Some(vec![KeyboardShortcut {
        action: "next".into(),
        key: "J".into(),
    }]);
    right.cover = true;
    right.flip_vertical = true;

    let start = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let save_left = scope.spawn(|| {
            start.wait();
            a.save(&left)
        });
        start.wait();
        b.save(&right).unwrap();
        save_left.join().unwrap().unwrap();
    });
    let read_back = || isolated(dir.path()).load().unwrap();
    let merged = read_back();
    assert!(merged.spread && merged.cover);
    assert!(merged.flip_horizontal && merged.flip_vertical);
    assert_eq!(merged.keyboard_shortcuts, left.keyboard_shortcuts);
    assert!(merged.shortcuts.is_empty());
    // Unrelated preferences from another process must survive later edits.
    left.flip_horizontal = false;
    a.save(&left).unwrap();
    let merged = read_back();
    assert!(merged.spread && merged.cover);
    assert!(!merged.flip_horizontal && merged.flip_vertical);
    assert_eq!(merged.keyboard_shortcuts, left.keyboard_shortcuts);
}

#[test]
fn conflicting_rows_leave_last_valid_settings_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let a = isolated(dir.path());
    let b = isolated(dir.path());
    let mut left = a.load().unwrap();
    let mut right = b.load().unwrap();
    left.keyboard_shortcuts = Some(vec![KeyboardShortcut {
        action: "next".into(),
        key: "J".into(),
    }]);
    a.save(&left).unwrap();
    right.keyboard_shortcuts = Some(vec![
        KeyboardShortcut {
            action: "next".into(),
            key: "J".into(),
        },
        KeyboardShortcut {
            action: "prev".into(),
            key: "J".into(),
        },
    ]);
    assert!(b.save(&right).is_err());
    assert_eq!(isolated(dir.path()).load().unwrap(), left);
    right.keyboard_shortcuts.as_mut().unwrap()[1].key = "K".into();
    b.save(&right).unwrap();
    let actual = isolated(dir.path()).load().unwrap();
    assert_eq!(actual.keyboard_shortcuts, right.keyboard_shortcuts);
    assert!(actual.shortcuts.is_empty());
}

#[test]
fn import_replaces_all_values_and_becomes_the_importers_new_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let a = isolated(dir.path());
    let b = isolated(dir.path());
    let mut left = a.load().unwrap();
    let mut right = b.load().unwrap();
    left.rtl = true;
    left.flip_horizontal = true;
    a.save(&left).unwrap();
    right.cover = true;
    b.save(&right).unwrap();

    let mut imported = Settings {
        view_mode: "fill".into(),
        flip_vertical: true,
        ..Settings::default()
    };
    a.replace_from_import(&imported).unwrap();
    assert_eq!(isolated(dir.path()).load().unwrap(), imported);
    right.view_mode = "actual".into();
    b.save(&right).unwrap();
    imported.spread = true;
    a.save(&imported).unwrap();
    let actual = isolated(dir.path()).load().unwrap();
    assert!(actual.spread && !actual.cover && !actual.rtl);
    assert!(!actual.flip_horizontal && actual.flip_vertical);
    assert_eq!(actual.view_mode, "actual");
}

#[test]
fn portable_settings_round_trip_leaves_history_on_this_device() {
    let dir = tempfile::tempdir().unwrap();
    let store = isolated(dir.path());
    let mut s = Settings {
        rtl: true,
        spread: true,
        flip_horizontal: true,
        flip_vertical: true,
        view_mode: "fill".into(),
        cache_image_limit: 12,
        cache_memory_mb: 256,
        ..Default::default()
    };
    s.keyboard_shortcuts = Some(vec![KeyboardShortcut {
        action: "next".into(),
        key: "J".into(),
    }]);
    store.record_folder(dir.path(), 50).unwrap();
    store.save(&s).unwrap();
    assert_eq!(store.load().unwrap(), s);
    assert_eq!(isolated(dir.path()).load().unwrap(), s);
    let exported = dir.path().join("portable.json");
    store.export(&exported, &s).unwrap();
    assert_eq!(store.import(&exported).unwrap(), s);
    assert!(
        !fs::read_to_string(&exported)
            .unwrap()
            .contains(&*dir.path().to_string_lossy())
    );
    s.schema_version += 1;
    assert!(store.save(&s).is_err());
    assert_eq!(store.import(&exported).unwrap(), store.load().unwrap());
}

#[test]
fn keyboard_rows_round_trip_and_survive_unrelated_editor_saves() {
    let dir = tempfile::tempdir().unwrap();
    let a = isolated(dir.path());
    let b = isolated(dir.path());
    let mut left = a.load().unwrap();
    let mut right = b.load().unwrap();
    let rows = vec![
        KeyboardShortcut {
            action: "next".into(),
            key: "J".into(),
        },
        KeyboardShortcut {
            action: "next".into(),
            key: "K".into(),
        },
        KeyboardShortcut {
            action: "prev".into(),
            key: "L".into(),
        },
    ];
    left.keyboard_shortcuts = Some(rows.clone());
    a.save(&left).unwrap();
    right.view_mode = "fill".into();
    b.save(&right).unwrap();
    let saved = isolated(dir.path()).load().unwrap();
    assert_eq!(saved.keyboard_shortcuts, Some(rows));
    assert_eq!(saved.view_mode, "fill");
    assert!(saved.shortcuts.is_empty());

    let exported = dir.path().join("keyboard-rows.json");
    a.export(&exported, &saved).unwrap();
    assert_eq!(b.import(&exported).unwrap(), saved);
    let destination = isolated(&dir.path().join("another-device"));
    destination
        .replace_from_import(&b.import(&exported).unwrap())
        .unwrap();
    assert_eq!(destination.load().unwrap(), saved);

    // An explicitly empty row list disables every shortcut. A stale editor
    // changing an unrelated setting must not restore its earlier bindings.
    left.keyboard_shortcuts = Some(Vec::new());
    a.save(&left).unwrap();
    right.flip_horizontal = true;
    b.save(&right).unwrap();
    let saved = isolated(dir.path()).load().unwrap();
    assert_eq!(saved.keyboard_shortcuts, Some(Vec::new()));
    assert_eq!(saved.view_mode, "fill");
    assert!(saved.flip_horizontal);
}

#[test]
fn keyboard_rows_reject_oversized_or_malformed_entries() {
    let mut settings = Settings::default();
    for (action, key) in [
        (String::new(), "J".into()),
        ("next".into(), String::new()),
        ("x".repeat(129), "J".into()),
        ("next".into(), "J".repeat(129)),
        ("next\n".into(), "J".into()),
        ("next".into(), "J\n".into()),
    ] {
        settings.keyboard_shortcuts = Some(vec![KeyboardShortcut { action, key }]);
        assert!(settings.validate().is_err());
    }
    settings.keyboard_shortcuts = Some(vec![
        KeyboardShortcut {
            action: "next".into(),
            key: "J".into()
        };
        257
    ]);
    assert!(settings.validate().is_err());
    assert!(
        serde_json::from_str::<KeyboardShortcut>(
            r#"{"action":"next","key":"J","unexpected":true}"#
        )
        .is_err()
    );
}

#[test]
fn explicit_shortcut_saves_replace_stale_lists_without_reverting_other_settings() {
    let dir = tempfile::tempdir().unwrap();
    let a = isolated(dir.path());
    let b = isolated(dir.path());
    let mut left = a.load().unwrap();
    let mut right = b.load().unwrap();
    let row = |key: &str| KeyboardShortcut {
        action: "next".into(),
        key: key.into(),
    };
    right.keyboard_shortcuts = Some(vec![row("J")]);
    right.shortcuts.insert("first".into(), "K".into());
    right.view_mode = "fill".into();
    b.save(&right).unwrap();

    // A still sees the registered default list. Reset must explicitly replace
    // the newer list on disk despite matching A's unchanged baseline.
    a.save_shortcuts(&left).unwrap();
    let reset = isolated(dir.path()).load().unwrap();
    assert_eq!(
        reset.keyboard_shortcuts,
        Settings::default().keyboard_shortcuts
    );
    assert!(reset.shortcuts.is_empty());
    assert_eq!(reset.view_mode, "fill");

    left.keyboard_shortcuts = Some(vec![row("L")]);
    a.save_shortcuts(&left).unwrap();
    right.keyboard_shortcuts = Some(vec![row("M")]);
    right.shortcuts.clear();
    right.flip_vertical = true;
    b.save_shortcuts(&right).unwrap();

    // Saving A's existing list is explicit too; it must replace B's later
    // list while preserving B's independently changed viewing preference.
    a.save_shortcuts(&left).unwrap();
    let saved = isolated(dir.path()).load().unwrap();
    assert_eq!(saved.keyboard_shortcuts, left.keyboard_shortcuts);
    assert!(saved.shortcuts.is_empty());
    assert_eq!(saved.view_mode, "fill");
    assert!(saved.flip_vertical);
}

#[test]
fn reorder_bound_clear_and_persist_folder_history() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    assert!(store.recent().unwrap().is_empty());
    assert!(!store.database.exists());
    let folders: Vec<_> = ["a", "b", "c"]
        .map(|s| {
            let p = root.path().join(s);
            fs::create_dir(&p).unwrap();
            fs::write(p.join("1.png"), b"image").unwrap();
            p.canonicalize()
                .map(crate::core::paths::simplified)
                .unwrap()
        })
        .into();
    for i in [0, 1, 0, 2] {
        store.record_folder(&folders[i], 2).unwrap();
    }
    assert_eq!(
        isolated(root.path()).recent().unwrap(),
        [folders[2].clone(), folders[0].clone()]
    );
    store.clear_recent().unwrap();
    assert!(store.recent().unwrap().is_empty());
}

#[test]
fn clear_data_removes_device_state_but_keeps_preferences() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    store.clear_data().unwrap();
    assert!(!store.database.exists());

    let folder = root.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("1.png"), b"image").unwrap();
    store.record_folder(&folder, 10).unwrap();
    store.record_move(&folder, 10).unwrap();
    store.save_window_size([1280., 800.]).unwrap();
    let mut settings = Settings::default();
    settings.recent_limit = 7;
    store.save(&settings).unwrap();

    store.clear_data().unwrap();
    assert!(store.recent().unwrap().is_empty());
    assert!(store.recent_moves(10).unwrap().is_empty());
    assert_eq!(store.load_window_size().unwrap(), None);
    assert_eq!(isolated(root.path()).load().unwrap().recent_limit, 7);

    store.save_window_size([900., 600.]).unwrap();
    assert_eq!(store.load_window_size().unwrap(), Some([900., 600.]));
}

#[test]
fn recent_removes_unavailable_paths_from_persisted_history() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    let folders = ["first", "missing", "replaced", "last"].map(|name| {
        let path = root.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("1.png"), b"image").unwrap();
        let path = path
            .canonicalize()
            .map(crate::core::paths::simplified)
            .unwrap();
        store.record_folder(&path, 10).unwrap();
        path
    });
    fs::remove_dir_all(&folders[1]).unwrap();
    fs::remove_dir_all(&folders[2]).unwrap();
    fs::write(&folders[2], b"not a folder").unwrap();

    let expected = [folders[3].clone(), folders[0].clone()];
    assert_eq!(store.recent().unwrap(), expected);
    assert_eq!(fs::read(&folders[2]).unwrap(), b"not a folder");

    // Restoring access must not resurrect a deleted history record.
    fs::create_dir(&folders[1]).unwrap();
    fs::remove_file(&folders[2]).unwrap();
    fs::create_dir(&folders[2]).unwrap();
    for path in &folders[1..=2] {
        fs::write(path.join("1.png"), b"image").unwrap();
    }
    assert_eq!(isolated(root.path()).recent().unwrap(), expected);
}

#[cfg(unix)]
#[test]
fn recent_removes_unreadable_folders_even_when_they_exist() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    let folder = root.path().join("unreadable");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("1.png"), b"image").unwrap();
    store.record_folder(&folder, 10).unwrap();
    let permissions = fs::metadata(&folder).unwrap().permissions();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0)).unwrap();
    let denied = fs::read_dir(&folder).is_err();
    let recent = store.recent();
    // Restore permissions before assertions so the fixture can always be removed.
    fs::set_permissions(&folder, permissions).unwrap();
    if denied {
        assert!(recent.unwrap().is_empty());
        assert!(isolated(root.path()).recent().unwrap().is_empty());
    } else {
        // Elevated test runners may retain access despite the permission bits.
        assert_eq!(
            recent.unwrap(),
            [folder
                .canonicalize()
                .map(crate::core::paths::simplified)
                .unwrap()]
        );
    }
}

#[test]
fn recent_removes_folders_without_images_and_keeps_their_contents() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    let folders = ["empty", "notes-only", "nested-only", "images"].map(|name| {
        let path = root.path().join(name);
        fs::create_dir(&path).unwrap();
        store.record_folder(&path, 10).unwrap();
        path.canonicalize()
            .map(crate::core::paths::simplified)
            .unwrap()
    });
    fs::write(folders[1].join("notes.txt"), b"keep these notes").unwrap();
    let nested = folders[2].join("directory.png");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("1.png"), b"nested image").unwrap();
    fs::write(folders[3].join("1.PNG"), b"image").unwrap();

    assert_eq!(store.recent().unwrap(), [folders[3].clone()]);
    assert!(folders.iter().all(|path| path.is_dir()));
    assert_eq!(
        fs::read(folders[1].join("notes.txt")).unwrap(),
        b"keep these notes"
    );
    assert!(nested.join("1.png").is_file());

    // Adding an image later must not resurrect the removed records.
    for folder in &folders[..3] {
        fs::write(folder.join("2.png"), b"image").unwrap();
    }
    assert_eq!(
        isolated(root.path()).recent().unwrap(),
        [folders[3].clone()]
    );
}

#[test]
fn recent_move_history_is_ordered_bounded_private_and_removable() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::in_directories(root.path().join("config"), root.path().join("state"));
    assert!(store.recent_moves(10).unwrap().is_empty());
    let paths: Vec<_> = (0..12)
        .map(|n| root.path().join(format!("destination-{n}")))
        .collect();
    for path in &paths {
        store.record_move(path, 10).unwrap();
    }
    assert_eq!(
        store.recent_moves(10).unwrap(),
        paths[2..].iter().rev().cloned().collect::<Vec<_>>()
    );
    store.record_move(&paths[5], 10).unwrap();
    assert_eq!(store.recent_moves(10).unwrap()[0], paths[5]);
    assert_eq!(store.recent_moves(10).unwrap().len(), 10);
    store.trim_recent_moves(3).unwrap();
    assert_eq!(
        store.recent_moves(10).unwrap(),
        [paths[5].clone(), paths[11].clone(), paths[10].clone()]
    );
    store.remove_recent_move(Some(&paths[11])).unwrap();
    assert_eq!(
        store.recent_moves(10).unwrap(),
        [paths[5].clone(), paths[10].clone()]
    );
    let settings = Settings {
        recent_move_limit: 17,
        ..Default::default()
    };
    store.save(&settings).unwrap();
    assert_eq!(store.load().unwrap().recent_move_limit, 17);
    let export = root.path().join("export.json");
    store.export(&export, &settings).unwrap();
    assert!(
        !std::fs::read_to_string(&export)
            .unwrap()
            .contains("destination-")
    );
    assert_eq!(store.import(&export).unwrap().recent_move_limit, 17);
    assert!(store.recent().unwrap().is_empty());
    store.remove_recent_move(None).unwrap();
    assert!(store.recent_moves(10).unwrap().is_empty());
    assert!(store.record_move(&paths[0], 0).is_err());
}

#[test]
fn move_shortcut_upgrade_uses_free_m_and_preserves_explicit_choices() {
    let defaults = crate::core::commands::default_shortcut_rows();
    let old: Vec<_> = defaults
        .iter()
        .filter(|row| row.action != "move")
        .cloned()
        .collect();
    let mut json = serde_json::to_value(Settings {
        keyboard_shortcuts: Some(old.clone()),
        ..Default::default()
    })
    .unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("move_shortcut_migrated");
    json.as_object_mut().unwrap().remove("recent_move_limit");
    let mut upgraded: Settings = serde_json::from_value(json).unwrap();
    upgraded.normalize();
    assert_eq!(upgraded.recent_move_limit, 10);
    assert!(
        upgraded
            .keyboard_shortcuts
            .as_ref()
            .unwrap()
            .iter()
            .any(|row| row.action == "move" && row.key == "M")
    );
    upgraded.validate().unwrap();
    let mut conflict = old.clone();
    conflict.push(KeyboardShortcut {
        action: "metadata".into(),
        key: "M".into(),
    });
    let mut custom = old.clone();
    custom.push(KeyboardShortcut {
        action: "move".into(),
        key: "F8".into(),
    });
    for rows in [conflict, custom, Vec::new()] {
        let mut settings = Settings {
            keyboard_shortcuts: Some(rows.clone()),
            move_shortcut_migrated: false,
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.keyboard_shortcuts.as_ref(), Some(&rows));
        settings.validate().unwrap();
    }
    upgraded.keyboard_shortcuts = Some(old.clone());
    upgraded.normalize();
    assert_eq!(upgraded.keyboard_shortcuts, Some(old));
}

#[test]
fn settings_ctrl_s_upgrade_keeps_primary_comma_and_respects_taken_chords() {
    let defaults = crate::core::commands::default_shortcut_rows();
    let keys = |rows: &[KeyboardShortcut]| -> Vec<String> {
        rows.iter()
            .filter(|row| row.action == "settings")
            .map(|row| row.key.clone())
            .collect()
    };
    assert_eq!(keys(&defaults), ["Primary+Comma", "Ctrl+S"]);
    let old: Vec<_> = defaults
        .iter()
        .filter(|row| !(row.action == "settings" && row.key == "Ctrl+S"))
        .cloned()
        .collect();
    let mut json = serde_json::to_value(Settings {
        keyboard_shortcuts: Some(old.clone()),
        ..Default::default()
    })
    .unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("settings_shortcut_migrated");
    let mut upgraded: Settings = serde_json::from_value(json).unwrap();
    upgraded.normalize();
    assert_eq!(
        keys(upgraded.keyboard_shortcuts.as_ref().unwrap()),
        ["Primary+Comma", "Ctrl+S"]
    );
    upgraded.validate().unwrap();
    // A chord the user already gave to another command stays theirs.
    let taken = crate::core::commands::canonical_chord("Ctrl+S");
    let mut conflict = old.clone();
    conflict.push(KeyboardShortcut {
        action: "open".into(),
        key: taken,
    });
    for rows in [conflict, Vec::new()] {
        let mut settings = Settings {
            keyboard_shortcuts: Some(rows.clone()),
            settings_shortcut_migrated: false,
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.keyboard_shortcuts.as_ref(), Some(&rows));
        settings.validate().unwrap();
    }
    // Removing it later is not undone.
    upgraded.keyboard_shortcuts = Some(old.clone());
    upgraded.normalize();
    assert_eq!(upgraded.keyboard_shortcuts, Some(old));
}

#[cfg(windows)]
#[test]
fn verbatim_history_from_older_versions_is_shown_as_plain_paths() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    let folders = ["old", "both"].map(|name| {
        let path = root.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("1.png"), b"image").unwrap();
        crate::core::paths::canonicalize(&path).unwrap()
    });
    let verbatim = |path: &Path| PathBuf::from(format!(r"\\?\{}", path.display()));
    // Older versions stored fs::canonicalize output directly.
    let conn = store.connection().unwrap();
    for (table, path) in [
        ("library", verbatim(&folders[0])),
        ("library", folders[1].clone()),
        ("library", verbatim(&folders[1])),
        ("recent_moves", verbatim(&folders[0])),
        ("recent_moves", verbatim(&folders[1])),
        ("recent_moves", folders[1].clone()),
    ] {
        let columns = if table == "library" {
            "(kind, path) VALUES (0, ?1)"
        } else {
            "(path) VALUES (?1)"
        };
        conn.execute(
            &format!("INSERT INTO {table} {columns}"),
            [encode_path(&path)],
        )
        .unwrap();
    }
    drop(conn);

    let newest_first = [folders[1].clone(), folders[0].clone()];
    assert_eq!(store.recent().unwrap(), newest_first);
    assert_eq!(store.recent_moves(10).unwrap(), newest_first);
    store.remove_entry(Kind::Recent, &folders[0]).unwrap();
    store.remove_recent_move(Some(&folders[0])).unwrap();
    assert_eq!(store.recent().unwrap(), [folders[1].clone()]);
    assert_eq!(store.recent_moves(10).unwrap(), [folders[1].clone()]);
}

fn image_folder(root: &Path, name: &str, images: &[&str]) -> PathBuf {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    for image in images {
        fs::write(path.join(image), b"image").unwrap();
    }
    crate::core::paths::canonicalize(&path).unwrap()
}

#[test]
fn legacy_recent_folders_migrate_in_order_once() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    let folders: Vec<_> = ["a", "b", "c"]
        .iter()
        .map(|name| image_folder(root.path(), name, &["1.png"]))
        .collect();
    fs::create_dir_all(store.database.parent().unwrap()).unwrap();
    let conn = Connection::open(&store.database).unwrap();
    conn.execute_batch(
        "CREATE TABLE recent_folders (
            id INTEGER PRIMARY KEY, path BLOB NOT NULL UNIQUE, opened_at INTEGER NOT NULL);",
    )
    .unwrap();
    // Older versions listed the highest id first.
    for folder in [&folders[1], &folders[0], &folders[2]] {
        conn.execute(
            "INSERT INTO recent_folders (path, opened_at) VALUES (?1, 0)",
            [encode_path(folder)],
        )
        .unwrap();
    }
    drop(conn);

    let expected = [folders[2].clone(), folders[0].clone(), folders[1].clone()];
    assert_eq!(store.recent().unwrap(), expected);
    assert_eq!(isolated(root.path()).recent().unwrap(), expected);
    let legacy: bool = store
        .connection()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='recent_folders')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!legacy);
    // Migrated entries start at their folder's first image.
    let entry = &store.library().unwrap()[0];
    assert_eq!(
        entry.image.as_deref(),
        Some(folders[2].join("1.png").as_path())
    );
    assert_eq!(
        (entry.name.as_deref(), entry.thumbnail.as_deref()),
        (None, None)
    );
}

#[test]
fn recent_folders_reopen_at_the_page_last_shown() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    let folder = image_folder(root.path(), "album", &["1.png", "2.png", "10.png"]);
    let other = image_folder(root.path(), "other", &["1.png"]);
    // Nothing is created for a folder that is not in the list.
    store.remember_page(&folder.join("2.png")).unwrap();
    assert!(!store.database.exists());

    store.record_folder(&folder, 10).unwrap();
    assert_eq!(
        store.library().unwrap()[0].image,
        Some(folder.join("1.png"))
    );
    store.remember_page(&folder.join("10.png")).unwrap();
    store
        .set_thumbnail(Kind::Recent, &folder, &"a".repeat(32))
        .unwrap();
    // Reopening moves the folder up and keeps its page and thumbnail.
    store.record_folder(&other, 10).unwrap();
    store.record_folder(&folder, 10).unwrap();
    let entries = store.library().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].folder, folder);
    assert_eq!(entries[0].image, Some(folder.join("10.png")));
    assert_eq!(entries[0].thumbnail, Some("a".repeat(32)));
    assert!(store.set_thumbnail(Kind::Recent, &folder, "../x").is_err());

    // A deleted page falls back to the first image.
    fs::remove_file(folder.join("10.png")).unwrap();
    assert_eq!(
        store.library().unwrap()[0].image,
        Some(folder.join("1.png"))
    );
    // Remembering a page never adds a bookmark or another folder.
    store.remember_page(&other.join("1.png")).unwrap();
    assert_eq!(store.recent().unwrap(), [folder.clone(), other.clone()]);
    assert!(
        store
            .library()
            .unwrap()
            .iter()
            .all(|entry| entry.kind == Kind::Recent)
    );
}

#[test]
fn bookmarks_are_folders_with_an_image_title_and_stay_when_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    let comics = image_folder(root.path(), "comics", &["1.png", "2.png"]);
    let photos = image_folder(root.path(), "photos", &["a.png"]);
    assert!(store.add_bookmark(&comics, "Folder only").is_err());
    assert!(
        store
            .add_bookmark(&comics.join("missing.png"), "x")
            .is_err()
    );
    fs::write(comics.join("notes.txt"), b"notes").unwrap();
    assert!(store.add_bookmark(&comics.join("notes.txt"), "x").is_err());
    assert_eq!(store.bookmark_title(&comics).unwrap(), None);

    assert_eq!(
        store
            .add_bookmark(&comics.join("2.png"), "  Comics \n")
            .unwrap(),
        comics.join("2.png")
    );
    store.add_bookmark(&photos.join("a.png"), "   ").unwrap();
    store.record_folder(&comics, 10).unwrap();
    assert_eq!(
        store.bookmark_title(&comics).unwrap(),
        Some("Comics".into())
    );
    assert_eq!(
        store.bookmark_title(&photos).unwrap(),
        Some("photos".into())
    );

    let entries = store.library().unwrap();
    let titles: Vec<_> = entries
        .iter()
        .map(|entry| (entry.kind, entry.title()))
        .collect();
    assert_eq!(
        titles,
        [
            (Kind::Recent, "comics".to_owned()),
            (Kind::Bookmark, "photos".to_owned()),
            (Kind::Bookmark, "Comics".to_owned()),
        ]
    );
    assert_eq!(entries[2].image, Some(comics.join("2.png")));

    // Adding again updates the bookmark in place, without moving it.
    store
        .add_bookmark(&comics.join("1.png"), "Renamed")
        .unwrap();
    store.rename_bookmark(&photos, "Trips").unwrap();
    let bookmarks: Vec<_> = store
        .library()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.kind == Kind::Bookmark)
        .map(|entry| (entry.title(), entry.image))
        .collect();
    assert_eq!(
        bookmarks,
        [
            ("Trips".to_owned(), Some(photos.join("a.png"))),
            ("Renamed".to_owned(), Some(comics.join("1.png"))),
        ]
    );

    // An emptied or missing folder stays bookmarked, unlike a recent folder.
    for image in ["1.png", "2.png"] {
        fs::remove_file(comics.join(image)).unwrap();
    }
    fs::remove_dir_all(&photos).unwrap();
    let entries = store.library().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.kind == Kind::Bookmark));
    assert!(
        entries
            .iter()
            .all(|entry| !entry.available && entry.image.is_none())
    );
    fs::write(comics.join("3.png"), b"image").unwrap();
    let comics_entry = store
        .library()
        .unwrap()
        .into_iter()
        .find(|entry| entry.folder == comics)
        .unwrap();
    assert!(comics_entry.available);
    assert_eq!(comics_entry.image, Some(comics.join("3.png")));

    // Clearing recent items or device data keeps bookmarks; removal is explicit.
    store.clear_recent().unwrap();
    store.clear_data().unwrap();
    assert_eq!(store.library().unwrap().len(), 2);
    store.remove_entry(Kind::Bookmark, &photos).unwrap();
    assert_eq!(store.library().unwrap().len(), 1);
}

#[test]
fn thumbnails_are_pruned_once_no_entry_uses_them() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    store.prune_thumbnails(Duration::ZERO).unwrap();
    assert!(!store.thumbnail_dir().exists());
    let folder = image_folder(root.path(), "album", &["1.png"]);
    let marked = image_folder(root.path(), "marked", &["1.png"]);
    store.record_folder(&folder, 10).unwrap();
    store.add_bookmark(&marked.join("1.png"), "").unwrap();
    let dir = store.thumbnail_dir();
    fs::create_dir_all(&dir).unwrap();
    let keys = ["1", "2", "3", "4"].map(|digit| digit.repeat(32));
    for key in &keys {
        fs::write(library::thumbnail_file(&dir, key), b"png").unwrap();
        fs::write(dir.join(format!("{key}.jpg")), b"legacy jpeg").unwrap();
    }
    fs::write(dir.join(".tmpABC"), b"partial").unwrap();
    fs::write(dir.join("keep.txt"), b"not ours").unwrap();
    store
        .set_thumbnail(Kind::Recent, &folder, &keys[0])
        .unwrap();
    store
        .set_thumbnail(Kind::Bookmark, &marked, &keys[1])
        .unwrap();

    // Young files may belong to another window that has not recorded them yet.
    store.prune_thumbnails(Duration::from_secs(600)).unwrap();
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 10);
    store.prune_thumbnails(Duration::ZERO).unwrap();
    let mut left: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|file| file.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            format!("{}.jpg", keys[0]),
            format!("{}.png", keys[0]),
            format!("{}.jpg", keys[1]),
            format!("{}.png", keys[1]),
            "keep.txt".to_owned()
        ]
    );

    // Clearing recent items deletes their thumbnails but not the bookmark's.
    store.clear_recent().unwrap();
    assert!(!library::thumbnail_file(&dir, &keys[0]).exists());
    assert!(!dir.join(format!("{}.jpg", keys[0])).exists());
    assert!(library::thumbnail_file(&dir, &keys[1]).exists());
    assert!(dir.join(format!("{}.jpg", keys[1])).exists());
    store.remove_entry(Kind::Bookmark, &marked).unwrap();
    store.prune_thumbnails(Duration::ZERO).unwrap();
    assert!(!library::thumbnail_file(&dir, &keys[1]).exists());
    assert!(!dir.join(format!("{}.jpg", keys[1])).exists());
}

#[test]
fn bookmark_key_is_added_once_to_saved_lists_where_it_is_free() {
    let defaults = crate::core::commands::default_shortcut_rows();
    let without: Vec<_> = defaults
        .iter()
        .filter(|row| row.action != "bookmark")
        .cloned()
        .collect();
    let upgrade = |rows: Vec<KeyboardShortcut>| {
        let mut settings = Settings {
            keyboard_shortcuts: Some(rows),
            bookmark_shortcut_migrated: false,
            ..Default::default()
        };
        settings.normalize();
        assert!(settings.bookmark_shortcut_migrated);
        settings.keyboard_shortcuts.unwrap()
    };
    // An untouched older list gains B right after the recent-folders key.
    assert_eq!(upgrade(without.clone()), defaults);
    // B already used for something else, or a deliberately empty list, stays.
    let mut taken = without.clone();
    taken.push(KeyboardShortcut {
        action: "next".into(),
        key: "B".into(),
    });
    assert_eq!(upgrade(taken.clone()), taken);
    assert_eq!(upgrade(Vec::new()), Vec::new());
    // Settings saved by this version never get the key back after removal.
    let mut settings = Settings {
        keyboard_shortcuts: Some(without.clone()),
        ..Default::default()
    };
    settings.normalize();
    assert_eq!(settings.keyboard_shortcuts, Some(without));
}

#[test]
fn mouse_gestures_default_for_old_files_and_repair_unknown_values_per_gesture() {
    use crate::core::settings::MouseSettings;
    let defaults = MouseSettings::default();
    assert_eq!(
        (
            defaults.wheel.as_str(),
            defaults.primary_wheel.as_str(),
            defaults.right_click.as_str(),
            defaults.middle_click.as_str(),
            defaults.double_click.as_str(),
            defaults.back_button.as_str(),
            defaults.forward_button.as_str(),
            defaults.drag.as_str(),
            defaults.primary_drag.as_str(),
        ),
        (
            "zoom",
            "none",
            "menu",
            "recent",
            "fullscreen",
            "prev",
            "next",
            "pan",
            "none"
        )
    );
    assert!(!defaults.reverse_wheel);

    let dir = tempfile::tempdir().unwrap();
    let store = isolated(dir.path());
    let mut old = serde_json::to_value(Settings::default()).unwrap();
    old.as_object_mut().unwrap().remove("mouse");
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    let file = dir.path().join("config").join("settings.json");
    std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(store.load().unwrap().mouse, defaults);

    // A partial object keeps its valid choices; unknown ones fall back alone.
    old["mouse"] = serde_json::json!({
        "wheel": "page",
        "primary_wheel": "spin",
        "right_click": "trash",
        "middle_click": "no-such-command",
        "drag": "window",
        "primary_drag": "fly",
        "reverse_wheel": true
    });
    std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
    let loaded = store.load().unwrap().mouse;
    assert_eq!(
        loaded,
        MouseSettings {
            wheel: "page".into(),
            right_click: "trash".into(),
            drag: "window".into(),
            reverse_wheel: true,
            ..defaults.clone()
        }
    );

    let mut invalid = Settings::default();
    invalid.mouse.double_click = "bogus".into();
    assert!(invalid.validate().is_err());
    invalid.normalize();
    invalid.validate().unwrap();
    assert_eq!(invalid.mouse.double_click, "fullscreen");
}

#[test]
fn mouse_gestures_round_trip_and_merge_across_windows() {
    let before = Settings::default();
    let mut after = before.clone();
    after.mouse.middle_click = "none".into();
    after.mouse.primary_drag = "window".into();
    let imported: Settings = serde_json::from_str(&serde_json::to_string(&after).unwrap()).unwrap();
    assert_eq!(imported.mouse, after.mouse);
    let mut merged = before.clone();
    merged.language = crate::i18n::Language::Korean;
    merged.merge_changes(&before, &imported);
    assert_eq!(merged.mouse, after.mouse);
    assert_eq!(merged.language, crate::i18n::Language::Korean);
}

#[test]
fn touch_gestures_default_for_old_files_and_repair_only_invalid_actions() {
    use crate::core::settings::TouchSettings;
    let defaults = TouchSettings::default();
    assert!(defaults.swipe_navigation && defaults.pinch_zoom);
    assert!(defaults.one_finger_pan && defaults.two_finger_pan);
    assert_eq!(defaults.double_tap, "fit");
    assert_eq!(defaults.long_press, "menu");

    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    let file = root.path().join("old-settings.json");
    let mut old = serde_json::to_value(Settings::default()).unwrap();
    old.as_object_mut().unwrap().remove("touch");
    old["mouse"]["double_click"] = serde_json::json!("recent");
    std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
    let imported = store.import(&file).unwrap();
    assert_eq!(imported.touch, defaults);
    assert_eq!(imported.mouse.double_click, "recent");

    // Partial files retain disabled gestures and the other valid action.
    old["touch"] = serde_json::json!({
        "swipe_navigation": false,
        "one_finger_pan": false,
        "double_tap": "share",
        "long_press": "no-such-command"
    });
    std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
    let imported = store.import(&file).unwrap();
    assert_eq!(
        imported.touch,
        TouchSettings {
            swipe_navigation: false,
            one_finger_pan: false,
            double_tap: "share".into(),
            ..defaults.clone()
        }
    );

    for action in ["bogus", "new-window", "move", "chrome"] {
        let mut invalid = imported.clone();
        invalid.touch.double_tap = action.into();
        assert!(invalid.validate().is_err());
        invalid.normalize();
        invalid.validate().unwrap();
        assert_eq!(invalid.touch.double_tap, defaults.double_tap);
        assert!(!invalid.touch.swipe_navigation && !invalid.touch.one_finger_pan);
    }
    for action in ["none", "menu", "fit", "fullscreen", "share", "orientation"] {
        let mut valid = imported.clone();
        valid.touch.double_tap = action.into();
        valid.touch.long_press = action.into();
        valid.validate().unwrap();
    }
}

#[test]
fn touch_and_mouse_gestures_save_merge_export_and_reset_independently() {
    use crate::core::settings::{MouseSettings, TouchSettings};
    let root = tempfile::tempdir().unwrap();
    let desktop = isolated(root.path());
    let mobile = isolated(root.path());
    let mut mouse_edit = desktop.load().unwrap();
    let mut touch_edit = mobile.load().unwrap();
    mouse_edit.mouse.double_click = "recent".into();
    mouse_edit.mouse.wheel = "page".into();
    touch_edit.touch = TouchSettings {
        swipe_navigation: false,
        pinch_zoom: false,
        one_finger_pan: false,
        two_finger_pan: false,
        double_tap: "none".into(),
        long_press: "share".into(),
    };
    desktop.save(&mouse_edit).unwrap();
    mobile.save(&touch_edit).unwrap();
    let merged = desktop.load().unwrap();
    assert_eq!(merged.mouse, mouse_edit.mouse);
    assert_eq!(merged.touch, touch_edit.touch);

    let exported = root.path().join("export.json");
    desktop.export(&exported, &merged).unwrap();
    assert_eq!(desktop.import(&exported).unwrap(), merged);
    let mut touch_reset = merged.clone();
    touch_reset.touch = TouchSettings::default();
    desktop.save(&touch_reset).unwrap();
    assert_eq!(mobile.load().unwrap().touch, TouchSettings::default());
    assert_eq!(mobile.load().unwrap().mouse, mouse_edit.mouse);

    // Editing the touch group after a concurrent mouse reset keeps both changes.
    let mut mouse_reset = mobile.load().unwrap();
    let mut next_touch = desktop.load().unwrap();
    mouse_reset.mouse = MouseSettings::default();
    next_touch.touch.long_press = "none".into();
    mobile.save(&mouse_reset).unwrap();
    desktop.save(&next_touch).unwrap();
    let merged = mobile.load().unwrap();
    assert_eq!(merged.mouse, MouseSettings::default());
    assert_eq!(merged.touch.long_press, "none");

    desktop.replace_from_import(&Settings::default()).unwrap();
    assert_eq!(desktop.load().unwrap(), Settings::default());
}

#[test]
fn new_display_preferences_round_trip_default_old_files_and_merge_independently() {
    let root = tempfile::tempdir().unwrap();
    let first = isolated(root.path());
    let second = isolated(root.path());
    let mut original = serde_json::to_value(Settings::default()).unwrap();
    for key in [
        "sort_by",
        "sort_descending",
        "background_rgb",
        "transparency_checker",
        "metadata_font_size",
        "reduce_motion",
        "disk_cache_enabled",
        "disk_cache_mb",
    ] {
        original.as_object_mut().unwrap().remove(key);
    }
    let old: Settings = serde_json::from_value(original).unwrap();
    assert_eq!(old.sort_by, "natural");
    assert_eq!(old.background_rgb, [0, 0, 0]);
    assert!(old.transparency_checker && old.disk_cache_enabled);
    assert_eq!(old.metadata_font_size, 12);
    assert_eq!(old.disk_cache_mb, 1024);
    first.save(&old).unwrap();
    let mut a = first.load().unwrap();
    let mut b = second.load().unwrap();
    a.sort_by = "modified".into();
    a.sort_descending = true;
    a.background_rgb = [15, 20, 25];
    a.transparency_checker = false;
    first.save(&a).unwrap();
    b.metadata_font_size = 24;
    b.reduce_motion = true;
    b.disk_cache_enabled = false;
    b.disk_cache_mb = 512;
    second.save(&b).unwrap();
    let merged = first.load().unwrap();
    assert_eq!(merged.sort_by, "modified");
    assert!(merged.sort_descending && merged.reduce_motion);
    assert_eq!(merged.background_rgb, [15, 20, 25]);
    assert!(!merged.transparency_checker && !merged.disk_cache_enabled);
    assert_eq!(merged.metadata_font_size, 24);
    assert_eq!(merged.disk_cache_mb, 512);
    let exported = root.path().join("export.json");
    first.export(&exported, &merged).unwrap();
    assert_eq!(first.import(&exported).unwrap(), merged);
    for invalid in [
        Settings {
            sort_by: "random".into(),
            ..Default::default()
        },
        Settings {
            metadata_font_size: 100,
            ..Default::default()
        },
        Settings {
            disk_cache_mb: 0,
            ..Default::default()
        },
    ] {
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn legacy_thumbnail_migration_preserves_keys_and_keeps_user_state_separate() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let legacy = state.join("thumbnails");
    fs::create_dir_all(&legacy).unwrap();
    let key = "a".repeat(32);
    fs::write(legacy.join(format!("{key}.png")), b"rgba png").unwrap();
    fs::write(legacy.join(format!("{key}.jpg")), b"old jpeg").unwrap();
    fs::write(legacy.join("keep.txt"), b"unowned").unwrap();
    let cache = root.path().join("os-cache");
    let store =
        Store::in_directories_with_cache(root.path().join("config"), state.clone(), cache.clone());
    assert_eq!(store.thumbnail_dir(), cache.join("v1/thumbnails"));
    assert!(store.library().unwrap().is_empty());
    for (extension, expected) in [
        ("png", b"rgba png".as_slice()),
        ("jpg", b"old jpeg".as_slice()),
    ] {
        assert_eq!(
            fs::read(store.thumbnail_dir().join(format!("{key}.{extension}"))).unwrap(),
            expected
        );
        assert!(!legacy.join(format!("{key}.{extension}")).exists());
    }
    assert!(legacy.join("keep.txt").is_file());
    assert!(!state.join("state.sqlite").exists());
    crate::core::disk_cache::Cache::new(store.cache_dir(), 0)
        .clear()
        .unwrap();
    assert!(legacy.join("keep.txt").is_file());
    assert!(store.library().unwrap().is_empty());
}

#[test]
fn saved_mobile_source_history_and_bookmark_survive_released_import_pixels() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    let sources = root.path().join("document-sources");
    let mut batch = crate::core::documents::ImportBatch::new(root.path()).unwrap();
    batch.add("one.png", &b"image"[..]).unwrap();
    batch
        .set_source(&sources, "12345678-source", "Selected album")
        .unwrap();
    batch.commit_source().unwrap();
    let copy = batch.first().unwrap().to_owned();
    let logical = batch.logical_path(&copy).unwrap();
    let folder = logical.parent().unwrap();
    store.record_folder(folder, 50).unwrap();
    store.remember_page(&logical).unwrap();
    store.add_bookmark(&logical, "Saved page").unwrap();
    drop(batch);
    assert!(!copy.exists());
    let entries = store.library().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .all(|entry| entry.available && entry.image.as_ref() == Some(&logical))
    );
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.kind == Kind::Recent)
            .unwrap()
            .title(),
        "Selected album"
    );
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.kind == Kind::Bookmark)
            .unwrap()
            .title(),
        "Saved page"
    );
}

#[test]
fn single_step_defaults_upgrade_free_keys_and_preserve_custom_or_deleted_assignments() {
    let defaults = crate::core::commands::default_shortcut_rows();
    let old: Vec<_> = defaults
        .iter()
        .filter(|row| !matches!(row.action.as_str(), "page-prev" | "page-next"))
        .cloned()
        .collect();
    let mut json = serde_json::to_value(Settings {
        keyboard_shortcuts: Some(old.clone()),
        ..Default::default()
    })
    .unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("single_step_shortcuts_migrated");
    let mut settings: Settings = serde_json::from_value(json).unwrap();
    settings.normalize();
    assert_eq!(settings.keyboard_shortcuts, Some(defaults));
    settings.validate().unwrap();
    settings.keyboard_shortcuts = Some(old);
    let mut reloaded: Settings =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    reloaded.normalize();
    assert_eq!(reloaded.keyboard_shortcuts, settings.keyboard_shortcuts);

    let row = |action: &str, key: &str| KeyboardShortcut {
        action: action.into(),
        key: key.into(),
    };
    for rows in [
        vec![],
        vec![row("metadata", "Alt+D")],
        vec![row("page-next", "J")],
    ] {
        let mut settings = Settings {
            keyboard_shortcuts: Some(rows.clone()),
            single_step_shortcuts_migrated: false,
            ..Default::default()
        };
        settings.normalize();
        settings.validate().unwrap();
        let upgraded = settings.keyboard_shortcuts.as_ref().unwrap();
        for existing in &rows {
            assert!(upgraded.contains(existing));
        }
        if rows.is_empty() {
            assert!(upgraded.is_empty());
        } else if rows[0].action == "metadata" {
            assert!(!upgraded.contains(&row("page-next", "Alt+D")));
            assert!(upgraded.contains(&row("page-next", "Alt+Right")));
        } else {
            assert!(!upgraded.contains(&row("page-next", "Alt+D")));
            assert!(!upgraded.contains(&row("page-next", "Alt+Right")));
        }
    }
}

#[test]
fn renamed_library_pages_follow_name_exchanges_once_and_keep_bookmark_title() {
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let folder = root.path().join("album");
    std::fs::create_dir(&folder).unwrap();
    let first = folder.join("one.png");
    let second = folder.join("two.png");
    std::fs::write(&first, b"one").unwrap();
    std::fs::write(&second, b"two").unwrap();
    let store = Store::in_directories(root.path().join("config"), root.path().join("state"));
    store.record_folder(&folder, 10).unwrap();
    store.remember_page(&first).unwrap();
    store.add_bookmark(&second, "My album").unwrap();
    store
        .rename_library_images(&[(first.clone(), second.clone()), (second, first)])
        .unwrap();
    let entries = store.library().unwrap();
    let recent = entries
        .iter()
        .find(|entry| entry.kind == Kind::Recent)
        .unwrap();
    let bookmark = entries
        .iter()
        .find(|entry| entry.kind == Kind::Bookmark)
        .unwrap();
    assert_eq!(
        recent.image.as_ref().unwrap().file_name().unwrap(),
        "two.png"
    );
    assert_eq!(
        bookmark.image.as_ref().unwrap().file_name().unwrap(),
        "one.png"
    );
    assert_eq!(bookmark.title(), "My album");
}
