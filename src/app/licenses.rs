//! Small, embedded credits list; complete notices remain in the distribution.
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
pub(crate) struct License {
    pub name: String,
    pub version: String,
    pub license: String,
    pub url: String,
}

pub(crate) fn direct_dependencies() -> &'static [License] {
    static ENTRIES: OnceLock<Vec<License>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/licenses/direct-dependencies.json"
        )))
        .expect("validated direct-dependency credits")
    })
}
