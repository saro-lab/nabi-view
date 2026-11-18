//! iOS Files-app folder items: a persistent display-cache identity from the
//! bookmarked source, the item's name, size and content modification date.
//! (`NSURLFileResourceIdentifierKey` is not stable across restarts, so it is
//! not used.) The identity is hashed into disk keys and never persisted as is.
use crate::core::documents::ProviderCache;
use std::{sync::Arc, time::SystemTime};

pub(crate) fn from_document_folder(
    source: &str,
    name: &str,
    bytes: u64,
    modified: Option<SystemTime>,
) -> Option<ProviderCache> {
    if source.is_empty() || name.is_empty() || crate::core::photo_library::is_ios(source) {
        return None;
    }
    let mut identity = b"ios-document-folder-v1".to_vec();
    for part in [source.as_bytes(), name.as_bytes()] {
        identity.extend((part.len() as u64).to_le_bytes());
        identity.extend(part);
    }
    identity.extend(bytes.to_le_bytes());
    // A revoked bookmark fails the read itself; nothing else to re-check.
    ProviderCache::new(identity, modified, Arc::new(|| true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::documents::ImportBatch;
    use std::{io::Write, time::Duration};

    #[test]
    fn files_app_items_keep_their_display_key_across_imports() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let cache = crate::core::disk_cache::Cache::new(root.path().join("cache"), 8 * 1024 * 1024);
        let at = |seconds| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
        let key = |source: &str, name: &str, bytes, modified| {
            let mut batch = ImportBatch::new_folder(root.path()).unwrap();
            batch
                .add_deferred_with_cache(
                    name,
                    bytes,
                    modified,
                    from_document_folder(source, name, bytes, modified),
                    Arc::new(|file, _, _| file.write_all(b"jpeg").map_err(|e| e.to_string())),
                )
                .unwrap();
            cache
                .key(batch.first().unwrap(), 2048)
                .map(|key| key.name().to_owned())
        };
        let first = key("12345678-folder", "page.jpg", 10, at(100));
        assert!(first.is_some());
        assert_eq!(first, key("12345678-folder", "page.jpg", 10, at(100)));
        assert_ne!(first, key("12345678-folder", "page.jpg", 11, at(100)));
        assert_ne!(first, key("12345678-folder", "page.jpg", 10, at(101)));
        assert_ne!(first, key("87654321-folder", "page.jpg", 10, at(100)));
        // No modification date: no persistent identity.
        assert!(from_document_folder("12345678-folder", "page.jpg", 10, None).is_none());
    }
}
