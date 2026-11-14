use super::*;

fn fixture(names: &[&str]) -> (tempfile::TempDir, Plan) {
    let root = tempfile::tempdir().unwrap();
    let paths: Vec<_> = names
        .iter()
        .map(|name| {
            let path = root.path().join(name);
            fs::write(&path, name.as_bytes()).unwrap();
            path
        })
        .collect();
    (root, Plan::new(&paths))
}
fn names(plan: &Plan) -> Vec<String> {
    plan.entries
        .iter()
        .map(|e| e.target.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn numbering_uses_current_order_preserves_extensions_and_expands_width() {
    let (_root, plan) = fixture(&["z.JPG", "a.png", "b.webp"]);
    let parent = plan.entries[0]
        .source
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(plan.digits(), 4);
    let next = plan.folder_names(plan.digits()).unwrap();
    assert_eq!(
        names(&next),
        [
            format!("{parent}_0001.JPG"),
            format!("{parent}_0002.png"),
            format!("{parent}_0003.webp")
        ]
    );
    let huge = Plan::new(&vec![PathBuf::from("a.jpg"); 10_000]);
    assert_eq!(huge.digits(), 5);
    let narrow = plan.folder_names(1).unwrap();
    assert!(names(&narrow)[0].ends_with("_1.JPG"));
}

#[test]
fn replacements_accumulate_match_case_and_leave_extension_untouched() {
    let (_root, plan) = fixture(&["CatCat.cat", "cat.png"]);
    let next = plan
        .replace("Cat", "Dog")
        .unwrap()
        .replace("Dog", "")
        .unwrap_err();
    assert_eq!(next.message, "rename-invalid-name");
    let next = plan
        .replace("Cat", "Dog")
        .unwrap()
        .replace("Dog", "Bird")
        .unwrap();
    assert_eq!(names(&next), ["BirdBird.cat", "cat.png"]);
    assert_eq!(names(&plan.replace("", "ignored").unwrap()), names(&plan));
    assert_eq!(fs::read(&plan.entries[0].source).unwrap(), b"CatCat.cat");
}

#[test]
fn rejects_duplicate_external_reserved_separator_and_symlink_destinations_before_mutation() {
    let (root, plan) = fixture(&["a.jpg", "b.jpg"]);
    fs::write(root.path().join("occupied.jpg"), b"existing").unwrap();
    for (from, to, issue) in [
        ("a", "b", "rename-conflict"),
        ("a", "occupied", "rename-conflict"),
        ("a", "B", "rename-conflict"),
        ("a", "CON", "rename-invalid-name"),
        ("a", "bad?name", "rename-invalid-name"),
        ("a", "../escape", "rename-invalid-name"),
        ("a", "bad\\name", "rename-invalid-name"),
    ] {
        let error = check(plan.replace(from, to).unwrap(), None).err().unwrap();
        assert_eq!(error.message, issue, "{to}");
    }
    assert_eq!(fs::read(&plan.entries[0].source).unwrap(), b"a.jpg");
    assert_eq!(
        fs::read(root.path().join("occupied.jpg")).unwrap(),
        b"existing"
    );
}

#[test]
fn exchanges_names_preserves_bytes_and_timestamp_and_cleans_staging() {
    let (root, mut plan) = fixture(&["a.jpg", "b.jpg"]);
    plan.entries[0].target = plan.entries[1].source.clone();
    plan.entries[1].target = plan.entries[0].source.clone();
    let modified = fs::metadata(&plan.entries[0].source)
        .unwrap()
        .modified()
        .unwrap();
    let result = rename(check(plan, None).unwrap());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.renamed.len(), 2);
    assert_eq!(fs::read(root.path().join("b.jpg")).unwrap(), b"a.jpg");
    assert_eq!(fs::read(root.path().join("a.jpg")).unwrap(), b"b.jpg");
    assert_eq!(
        fs::metadata(root.path().join("b.jpg"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn case_only_rename_and_noop_are_safe() {
    let (root, plan) = fixture(&["Cat.JPG"]);
    let result = rename(check(plan.clone(), None).unwrap());
    assert!(result.renamed.is_empty() && result.errors.is_empty());
    let result = rename(check(plan.replace("Cat", "cat").unwrap(), None).unwrap());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(fs::read(root.path().join("cat.JPG")).unwrap(), b"Cat.JPG");
    assert_eq!(
        fs::read_dir(root.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name(),
        "cat.JPG"
    );
}

#[test]
fn revalidates_source_and_destination_after_preview() {
    for replace_source in [false, true] {
        let (root, plan) = fixture(&["a.jpg"]);
        let checked = check(plan.replace("a", "renamed").unwrap(), None).unwrap();
        if replace_source {
            fs::write(root.path().join("a.jpg"), b"replacement image").unwrap();
        } else {
            fs::write(root.path().join("renamed.jpg"), b"unrelated").unwrap();
        }
        let result = rename(checked);
        assert_eq!(result.errors.len(), 1);
        assert!(result.renamed.is_empty());
        assert!(root.path().join("a.jpg").exists());
        if !replace_source {
            assert_eq!(
                fs::read(root.path().join("renamed.jpg")).unwrap(),
                b"unrelated"
            );
        }
    }
}

#[test]
fn every_mid_batch_failure_rolls_back_without_changing_file_bytes() {
    for fail_at in 1..=6 {
        let (root, mut plan) = fixture(&["a.jpg", "b.jpg", "c.jpg"]);
        plan.entries[0].target = root.path().join("b.jpg");
        plan.entries[1].target = root.path().join("c.jpg");
        plan.entries[2].target = root.path().join("a.jpg");
        let mut calls = 0;
        let result = rename_with(check(plan, None).unwrap(), |a, b| {
            calls += 1;
            if calls == fail_at {
                Err(io::Error::other("injected failure"))
            } else {
                rename_noclobber(a, b)
            }
        });
        assert!(!result.errors.is_empty());
        assert!(result.renamed.is_empty(), "{fail_at}: {:?}", result.renamed);
        for name in ["a.jpg", "b.jpg", "c.jpg"] {
            assert_eq!(fs::read(root.path().join(name)).unwrap(), name.as_bytes());
        }
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    }
}

#[test]
fn late_collision_never_overwrites_and_failed_rollback_preserves_recovery_files() {
    for obstruct_rollback in [false, true] {
        let (root, plan) = fixture(&["a.jpg"]);
        let checked = check(plan.replace("a", "new").unwrap(), None).unwrap();
        let mut calls = 0;
        let result = rename_with(checked, |a, b| {
            calls += 1;
            if calls == 2 {
                fs::write(b, b"late collision").unwrap();
                if obstruct_rollback {
                    fs::write(root.path().join("a.jpg"), b"new original occupant").unwrap();
                }
            }
            rename_noclobber(a, b)
        });
        assert!(!result.errors.is_empty());
        assert_eq!(
            fs::read(root.path().join("new.jpg")).unwrap(),
            b"late collision"
        );
        if obstruct_rollback {
            assert_eq!(result.renamed.len(), 1);
            assert_eq!(fs::read(&result.renamed[0].1).unwrap(), b"a.jpg");
            assert_eq!(
                fs::read(root.path().join("a.jpg")).unwrap(),
                b"new original occupant"
            );
        } else {
            assert!(result.renamed.is_empty());
            assert_eq!(fs::read(root.path().join("a.jpg")).unwrap(), b"a.jpg");
        }
    }
}

#[cfg(unix)]
#[test]
fn rejects_source_symlinks_and_dangling_destinations() {
    let (root, plan) = fixture(&["a.jpg"]);
    let link = root.path().join("link.jpg");
    std::os::unix::fs::symlink(&plan.entries[0].source, &link).unwrap();
    assert_eq!(
        check(Plan::new(&[link]), None).err().unwrap().message,
        "rename-invalid-source"
    );
    std::os::unix::fs::symlink(root.path().join("missing"), root.path().join("new.jpg")).unwrap();
    assert_eq!(
        check(plan.replace("a", "new").unwrap(), None)
            .err()
            .unwrap()
            .message,
        "rename-conflict"
    );
}
