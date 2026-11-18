use super::*;
use std::io::Write;

fn add(batch: &mut ImportBatch, name: &str) {
    assert!(
        batch
            .add_deferred(
                name,
                3,
                None,
                Arc::new(|output, _, live| {
                    if !live() {
                        return Err("Cancelled".into());
                    }
                    output.write_all(b"png").map_err(|error| error.to_string())
                })
            )
            .unwrap()
    );
}

#[test]
fn seed_and_full_import_share_capabilities_until_the_last_owner_drops() {
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let sources = root.path().join("document-sources");
    let cache = root.path().join("cache");
    let mut old = ImportBatch::new_folder(&cache).unwrap();
    old.set_source(&sources, "nabi-ios-photos", "Photos")
        .unwrap();
    add(&mut old, "old.png");
    old.commit_source().unwrap();
    let source = old.source().unwrap().root.clone();
    drop(old);

    let mut full = ImportBatch::new_folder(&cache).unwrap();
    full.set_source(&sources, "nabi-ios-photos", "Photos")
        .unwrap();
    full.request_page(Some(source.join("selected.png")));
    add(&mut full, "selected.png");
    let mut seed = full.snapshot_seed().unwrap();
    assert!(seed.commit_source().is_err());
    assert_eq!(
        full.requested_name(),
        Some(std::ffi::OsStr::new("selected.png"))
    );
    assert!(seed.is_seed() && seed.same_import(&full));
    assert!(Arc::ptr_eq(&seed.deferred_budget, &full.deferred_budget));
    assert_eq!(
        source_record(&source).unwrap().pages,
        vec![PathBuf::from("old.png")]
    );
    let selected = seed.initial_path().unwrap();
    assert_eq!(
        source_local_path(&source.join("selected.png")),
        Some(selected.clone())
    );
    add(&mut full, "neighbor.png");
    assert_eq!(seed.files().len(), 1);
    assert_eq!(full.files().len(), 2);
    let prepared = full.prepare_source_commit().unwrap();
    assert_eq!(
        source_record(&source).unwrap().pages,
        vec![PathBuf::from("old.png")]
    );
    full.commit_prepared_source(prepared).unwrap();
    assert_eq!(source_record(&source).unwrap().pages.len(), 2);
    let retained_seed = seed.clone();
    drop(seed);
    assert!(deferred::is_deferred(&selected));
    materialize(&selected, &|| true).unwrap();
    let directory = full.directory().to_owned();
    drop(full);
    assert!(directory.is_dir());
    assert!(deferred::is_deferred(&selected));
    assert_eq!(
        source_local_path(&source.join("selected.png")),
        Some(selected.clone())
    );
    drop(retained_seed);
    assert!(!deferred::is_deferred(&selected));
    assert!(!directory.exists());
    assert!(source_local_path(&source.join("selected.png")).is_none());
}

#[test]
fn seed_selection_never_substitutes_another_explicit_page_or_publishes_partial_metadata() {
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let mut batch = ImportBatch::new_folder(&root.path().join("cache")).unwrap();
    batch
        .set_source(
            &root.path().join("document-sources"),
            "nabi-ios-photos",
            "Photos",
        )
        .unwrap();
    let source = batch.source().unwrap().root.clone();
    batch.request_page(Some(source.join("missing.png")));
    add(&mut batch, "native-first.png");
    assert!(batch.snapshot_seed().is_none());
    batch.request_page(Some(source.clone()));
    assert!(batch.requested_name().is_none());
    assert_eq!(batch.snapshot_seed().unwrap().first(), batch.first());
    assert!(!source.join(SOURCE_MARKER).exists());
    let prepared = batch.prepare_source_commit().unwrap();
    drop(prepared); // Cancel between slow serialization/fsync and publication.
    assert!(!source.join(SOURCE_MARKER).exists());
}
