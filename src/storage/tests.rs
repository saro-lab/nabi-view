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
    let mut old = defaults.clone();
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
        json.as_object_mut()
            .unwrap()
            .remove("trash_shortcut_migrated");
        let mut settings: Settings = serde_json::from_value(json).unwrap();
        settings.normalize_shortcuts();
        assert_eq!(settings.keyboard_shortcuts, Some(defaults.clone()));
        settings.validate().unwrap();
        settings.keyboard_shortcuts = Some(old.clone());
        let mut reloaded: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        reloaded.normalize_shortcuts();
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
        settings.normalize_shortcuts();
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
            settings.normalize_shortcuts();
            assert_eq!(settings.keyboard_shortcuts.as_ref(), Some(&defaults));
            settings.keyboard_shortcuts = Some(old.clone());
            let json = serde_json::to_vec(&settings).unwrap();
            let mut reloaded: Settings = serde_json::from_slice(&json).unwrap();
            reloaded.normalize_shortcuts();
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
        settings.normalize_shortcuts();
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
    settings.normalize_shortcuts();
    assert_eq!(
        settings.keyboard_shortcuts,
        Some(vec![KeyboardShortcut {
            action: "about".into(),
            key: "F2".into()
        },])
    );
    settings.normalize_shortcuts();
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
        settings.normalize_shortcuts();
        assert_eq!(settings.keyboard_shortcuts.as_ref(), Some(&defaults));
        settings
            .keyboard_shortcuts
            .as_mut()
            .unwrap()
            .retain(|r| r.action != "about");
        let deleted = settings.keyboard_shortcuts.clone();
        settings.normalize_shortcuts();
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
        settings.normalize_shortcuts();
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
    settings.normalize_shortcuts();
    assert_eq!(settings.keyboard_shortcuts, Some(defaults));
    settings.keyboard_shortcuts = Some(old.clone());
    settings.normalize_shortcuts();
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
        settings.normalize_shortcuts();
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
    migrated.normalize_shortcuts();
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
            p.canonicalize().unwrap()
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
fn recent_removes_unavailable_paths_from_persisted_history() {
    let root = tempfile::tempdir().unwrap();
    let store = isolated(root.path());
    let folders = ["first", "missing", "replaced", "last"].map(|name| {
        let path = root.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("1.png"), b"image").unwrap();
        let path = path.canonicalize().unwrap();
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
        assert_eq!(recent.unwrap(), [folder.canonicalize().unwrap()]);
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
        path.canonicalize().unwrap()
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
    upgraded.normalize_shortcuts();
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
        settings.normalize_shortcuts();
        assert_eq!(settings.keyboard_shortcuts.as_ref(), Some(&rows));
        settings.validate().unwrap();
    }
    upgraded.keyboard_shortcuts = Some(old.clone());
    upgraded.normalize_shortcuts();
    assert_eq!(upgraded.keyboard_shortcuts, Some(old));
}
