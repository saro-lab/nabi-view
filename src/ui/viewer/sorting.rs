use super::*;
use std::cmp::Ordering;

#[derive(Default)]
pub(super) struct Sorting {
    pub stamps: HashMap<PathBuf, media::FileSortMetadata>,
    applied: Option<(String, bool)>,
}

struct Key {
    path: PathBuf,
    name: PathBuf,
    folded: String,
    stamp: Option<media::FileSortMetadata>,
}

fn compare(a: &Key, b: &Key, by: &str, descending: bool) -> Ordering {
    let natural = || media::compare_paths(&a.name, &b.name).then_with(|| a.path.cmp(&b.path));
    let numeric = match by {
        "size" => Some((
            a.stamp.map(|value| value.bytes as u128),
            b.stamp.map(|value| value.bytes as u128),
        )),
        "modified" => Some((
            a.stamp.and_then(|value| value.modified),
            b.stamp.and_then(|value| value.modified),
        )),
        _ => None,
    };
    let order = if let Some((a, b)) = numeric {
        match (a, b) {
            // Missing provider/archive metadata stays last in either direction.
            (Some(_), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Greater,
            _ => a.cmp(&b).then_with(natural),
        }
    } else if by == "name" {
        a.folded
            .cmp(&b.folded)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    } else {
        natural()
    };
    if descending { order.reverse() } else { order }
}

impl Viewer {
    /// Pure in-memory ordering: filesystem/provider work belongs to workers.
    pub(super) fn sorted_files(&self, files: Vec<PathBuf>) -> Vec<PathBuf> {
        // The normal folder worker already publishes natural order. Avoid
        // allocating per-name keys on the first/default display of large lists.
        if self.settings.sort_by == "natural"
            && !self.settings.sort_descending
            && files.windows(2).all(|pair| {
                media::compare_paths(
                    self.archives.sort_name(&pair[0]),
                    self.archives.sort_name(&pair[1]),
                )
                .is_le()
            })
        {
            return files;
        }
        let mut entries: Vec<_> = files
            .into_iter()
            .map(|path| {
                let name = self.archives.sort_name(&path).to_owned();
                let folded = name.to_string_lossy().to_lowercase();
                let stamp = self.sorting.stamps.get(&path).copied();
                Key {
                    path,
                    name,
                    folded,
                    stamp,
                }
            })
            .collect();
        entries.sort_unstable_by(|a, b| {
            compare(a, b, &self.settings.sort_by, self.settings.sort_descending)
        });
        entries.into_iter().map(|key| key.path).collect()
    }

    pub(super) fn sync_sort_settings(&mut self) {
        if self
            .sorting
            .applied
            .as_ref()
            .is_some_and(|(by, descending)| {
                by == &self.settings.sort_by && *descending == self.settings.sort_descending
            })
        {
            return;
        }
        self.sorting.applied = Some((self.settings.sort_by.clone(), self.settings.sort_descending));
        if self.random || self.files.len() < 2 {
            return;
        }
        let focused = self.files.get(self.focus_index).cloned();
        let files = std::mem::take(&mut self.files);
        self.files = self.sorted_files(files);
        self.focus_index = focused
            .and_then(|path| self.files.iter().position(|candidate| candidate == &path))
            .unwrap_or(0);
        self.span_cache.get_mut().take();
        self.cancel_prefetch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(name: &str, bytes: u64, modified: Option<u128>) -> Key {
        Key {
            path: name.into(),
            name: name.into(),
            folded: name.to_lowercase(),
            stamp: Some(media::FileSortMetadata { bytes, modified }),
        }
    }
    #[test]
    fn sort_modes_use_snapshot_metadata_and_keep_unknown_dates_last() {
        let a = key("02.png", 800, Some(300));
        let b = key("10.png", 200, Some(100));
        assert_eq!(compare(&a, &b, "natural", false), Ordering::Less);
        assert_eq!(compare(&a, &b, "size", false), Ordering::Greater);
        assert_eq!(compare(&a, &b, "modified", false), Ordering::Greater);
        assert_eq!(compare(&a, &b, "modified", true), Ordering::Less);
        let unknown = key("00.png", 0, None);
        assert_eq!(compare(&unknown, &a, "modified", false), Ordering::Greater);
        assert_eq!(compare(&unknown, &a, "modified", true), Ordering::Greater);
        assert_eq!(
            compare(&key("2.png", 0, None), &b, "name", false),
            Ordering::Greater
        );
    }

    #[test]
    fn folder_settings_reorder_live_files_and_random_exit_preserves_current_page() {
        use std::{
            io::Write,
            time::{Duration, Instant, SystemTime},
        };
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let images = root.path().join("images");
        std::fs::create_dir(&images).unwrap();
        for (name, padding, age) in [("2.png", 200, 10), ("10.png", 100, 20)] {
            let path = images.join(name);
            image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 128]))
                .save(&path)
                .unwrap();
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            file.write_all(&vec![0; padding]).unwrap();
            file.set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(age)),
            )
            .unwrap();
        }
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            Some(images.join("2.png")),
            Ok(Store::in_directories(
                root.path().join("config"),
                root.path().join("state"),
            )),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.listing.is_some() {
            assert!(Instant::now() < deadline);
            app.collect(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        let names = |app: &Viewer| {
            app.files
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&app), ["2.png", "10.png"]);
        let focused = app.files[app.focus_index].clone();
        app.settings.sort_by = "size".into();
        app.sync_sort_settings();
        assert_eq!(names(&app), ["10.png", "2.png"]);
        assert_eq!(app.files[app.focus_index], focused);
        app.settings.sort_by = "modified".into();
        app.sync_sort_settings();
        assert_eq!(names(&app), ["2.png", "10.png"]);
        app.settings.sort_descending = true;
        app.sync_sort_settings();
        assert_eq!(names(&app), ["10.png", "2.png"]);
        app.toggle_random();
        let random_focused = app.files[app.focus_index].clone();
        app.toggle_random();
        assert_eq!(names(&app), ["10.png", "2.png"]);
        assert_eq!(app.files[app.focus_index], random_focused);
        assert!(app.media.folder_stamps(u64::MAX - 1).is_empty());
        // Sorting uses the captured worker metadata, never a fresh UI stat.
        for path in &app.files {
            std::fs::remove_file(path).unwrap();
        }
        app.settings.sort_by = "size".into();
        app.sync_sort_settings();
        assert_eq!(names(&app), ["2.png", "10.png"]);
    }
}
