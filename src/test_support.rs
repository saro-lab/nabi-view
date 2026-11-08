//! Reproducible fixtures must not depend on a developer's private AI workspace.
pub(crate) fn root() -> std::path::PathBuf {
    let root = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .join("test-fixtures");
    std::fs::create_dir_all(&root).expect("create isolated test fixtures");
    root
}
