//! Bookmarks and recent folders, prepared on disk while the viewer is idle:
//! thumbnails ±10 and display images ±3 around each saved page. Planning lists
//! folders off the UI thread; one disk-only job runs at a time, only when the
//! current folder has nothing left to do, and never past half the disk cache.
use super::*;
use crate::core::library::{Entry, Kind};
use std::{collections::VecDeque, sync::mpsc};

/// Recent folders prepared, most recent first; every bookmark is prepared.
const RECENT: usize = 5;
const THUMBNAILS: usize = 10;
const DISPLAY: usize = 3;
/// Larger folders are listed only up to this many images.
const MAX_LISTED: usize = 10_000;
/// Prefill stops once the disk cache is this full.
const DISK_SHARE: f64 = 0.5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub path: PathBuf,
    /// A list/strip thumbnail; otherwise a display image at the window's tier.
    pub thumbnail: bool,
}

#[derive(Default)]
pub(super) struct Prefill {
    pending: VecDeque<Target>,
    ticket: Option<(u64, Target)>,
    planner: Option<mpsc::Receiver<Vec<Target>>>,
    /// The library changed; plan again.
    pub dirty: bool,
}

/// Every bookmark and the most recent folders, with their saved pages.
/// Archives and unavailable entries are skipped.
pub(super) fn sources<'a>(
    entries: impl Iterator<Item = &'a Entry>,
) -> Vec<(PathBuf, Option<PathBuf>)> {
    let mut recent = 0;
    entries
        .filter(|entry| entry.available && !crate::core::archive::is_archive(&entry.folder))
        .filter(|entry| match entry.kind {
            Kind::Bookmark => true,
            Kind::Recent => {
                recent += 1;
                recent <= RECENT
            }
        })
        .map(|entry| (entry.folder.clone(), entry.image.clone()))
        .collect()
}

/// Network shares are never read for prefill.
fn network(path: &Path) -> bool {
    let text = path.to_string_lossy();
    cfg!(windows)
        && (text.starts_with(r"\\?\UNC\")
            || (text.starts_with(r"\\")
                && !text.starts_with(r"\\?\")
                && !text.starts_with(r"\\.\")))
}

/// Targets in priority order per folder: the saved page, its thumbnails
/// nearest first, then the display images around it. Lists folders (I/O).
pub(super) fn plan(sources: Vec<(PathBuf, Option<PathBuf>)>) -> Vec<Target> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for (folder, page) in sources {
        if network(&folder) {
            continue;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        let Ok(access) = crate::platform::mac_access::resolve(&folder) else {
            continue;
        };
        #[cfg(all(target_os = "macos", not(test)))]
        let folder = access.path.clone();
        let Ok(files) = media::images_in_folder(&folder) else {
            continue;
        };
        let mut files: Vec<_> = files.take(MAX_LISTED).collect();
        files.sort_by(|a, b| media::compare_file_names(a, b));
        if files.is_empty() {
            continue;
        }
        let center = page
            .as_ref()
            .and_then(|page| files.iter().position(|file| file == page))
            .unwrap_or(0);
        let around = |reach: usize| {
            std::iter::once(center).chain(
                (1..=reach)
                    .flat_map(move |distance| {
                        [center.checked_add(distance), center.checked_sub(distance)]
                    })
                    .flatten(),
            )
        };
        let mut push = |index: usize, thumbnail| {
            if let Some(path) = files.get(index)
                && seen.insert((path.clone(), thumbnail))
            {
                targets.push(Target {
                    path: path.clone(),
                    thumbnail,
                });
            }
        };
        push(center, false);
        for index in around(THUMBNAILS) {
            push(index, true);
        }
        for index in around(DISPLAY) {
            push(index, false);
        }
    }
    targets
}

impl Viewer {
    /// Runs one prefill job at a time while nothing else needs the decoders.
    pub(super) fn run_prefill(&mut self, ctx: &egui::Context) {
        if !self.settings.prefill_library
            || !self.settings.disk_cache_enabled
            || self.store.is_none()
            // Mobile sources reopen only through the native host.
            || crate::app::is_mobile()
        {
            self.cancel_prefill();
            self.prefill.dirty = false;
            return;
        }
        if self.prefill.dirty && self.prefill.planner.is_none() {
            self.prefill.dirty = false;
            let sources = sources(self.library.items.iter().map(|item| &item.entry));
            let (send, receive) = mpsc::channel();
            let repaint = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("prefill-plan".into())
                .spawn(move || {
                    crate::platform::worker_priority::background();
                    let _ = send.send(plan(sources));
                    repaint.request_repaint();
                });
            if spawned.is_ok() {
                self.prefill.planner = Some(receive);
            }
        }
        if let Some(planner) = &self.prefill.planner {
            match planner.try_recv() {
                Ok(targets) => {
                    self.prefill.pending = targets.into();
                    self.prefill.planner = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.prefill.planner = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some((_, target)) = &self.prefill.ticket {
            // The viewer took this decoder slot: the job was replaced.
            let replaced = if target.thumbnail {
                self.thumb_ticket.is_some()
            } else {
                self.prefetch_ticket.is_some()
            };
            if !replaced {
                return;
            }
            let (_, target) = self.prefill.ticket.take().unwrap();
            self.prefill.pending.push_front(target);
        }
        if self.prefill.pending.is_empty() || !self.prefill_idle(ctx) {
            return;
        }
        if self
            .media
            .disk_cache_share()
            .is_some_and(|share| share >= DISK_SHARE)
        {
            self.prefill.pending.clear();
            return;
        }
        let target = self.prefill.pending.pop_front().unwrap();
        let id = if target.thumbnail {
            self.media.request_thumbnail_warm(target.path.clone())
        } else {
            self.media
                .request_disk_warm(target.path.clone(), self.fit_limit, media::Urgency::Ahead)
        };
        self.prefill.ticket = Some((id, target));
    }

    /// Foreground, no interaction, and the current folder's own work is done.
    fn prefill_idle(&self, ctx: &egui::Context) -> bool {
        self.memory_policy.background_allowed()
            && ctx.input(|input| input.focused)
            && !self.renaming()
            && self.listing.is_none()
            && !self.current_images_pending()
            && self.prefetch_ticket.is_none()
            && self.thumb_ticket.is_none()
            && !self.library.thumbnails_pending()
            && !self.details.pending()
            && !self.touch_gesture.active()
            && self.mobile_swipe.is_none()
    }

    /// A finished prefill job; failures are not retried this session.
    pub(super) fn prefill_finished(&mut self, id: u64) -> bool {
        if self
            .prefill
            .ticket
            .as_ref()
            .is_some_and(|(ticket, _)| *ticket == id)
        {
            self.prefill.ticket = None;
            true
        } else {
            false
        }
    }

    pub(super) fn cancel_prefill(&mut self) {
        self.prefill.pending.clear();
        self.prefill.planner = None;
        if let Some((_, target)) = self.prefill.ticket.take() {
            if target.thumbnail {
                if self.thumb_ticket.is_none() {
                    self.media.clear_thumbnail();
                }
            } else if self.prefetch_ticket.is_none() {
                self.media.clear_prefetch();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: Kind, folder: &Path, image: Option<PathBuf>) -> Entry {
        Entry {
            kind,
            folder: folder.to_owned(),
            image,
            name: None,
            thumbnail: None,
            available: true,
        }
    }

    #[test]
    fn every_bookmark_and_five_recent_folders_are_prepared() {
        let entries: Vec<_> = (0..8)
            .map(|i| entry(Kind::Recent, Path::new(&format!("recent-{i}")), None))
            .chain([
                entry(Kind::Bookmark, Path::new("book-a"), None),
                entry(Kind::Bookmark, Path::new("comic.zip"), None),
                Entry {
                    available: false,
                    ..entry(Kind::Bookmark, Path::new("offline"), None)
                },
            ])
            .collect();
        let folders: Vec<_> = sources(entries.iter())
            .into_iter()
            .map(|(f, _)| f)
            .collect();
        assert_eq!(
            folders,
            [
                "recent-0", "recent-1", "recent-2", "recent-3", "recent-4", "book-a"
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn plan_centers_on_the_saved_page_in_natural_order() {
        let dir = tempfile::tempdir().unwrap();
        for i in 1..=30 {
            std::fs::write(dir.path().join(format!("p{i}.png")), b"x").unwrap();
        }
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        let page = dir.path().join("p15.png");
        let targets = plan(vec![
            (dir.path().to_owned(), Some(page.clone())),
            (dir.path().join("missing"), None),
        ]);
        let name = |t: &Target| t.path.file_name().unwrap().to_string_lossy().into_owned();
        // The saved page's display image first, then thumbnails nearest first.
        assert_eq!(
            targets[0],
            Target {
                path: page,
                thumbnail: false
            }
        );
        let thumbnails: Vec<_> = targets.iter().filter(|t| t.thumbnail).map(name).collect();
        assert_eq!(thumbnails.len(), 21);
        assert_eq!(&thumbnails[..3], ["p15.png", "p16.png", "p14.png"]);
        assert!(
            thumbnails.contains(&"p5.png".to_owned()) && thumbnails.contains(&"p25.png".to_owned())
        );
        let display: Vec<_> = targets.iter().filter(|t| !t.thumbnail).map(name).collect();
        assert_eq!(
            display,
            [
                "p15.png", "p16.png", "p14.png", "p17.png", "p13.png", "p18.png", "p12.png"
            ]
        );
        // Near the start, the window clips instead of wrapping.
        let start = plan(vec![(dir.path().to_owned(), None)]);
        assert_eq!(start.iter().filter(|t| t.thumbnail).count(), 11);
    }

    #[cfg(windows)]
    #[test]
    fn network_shares_are_not_read() {
        assert!(network(Path::new(r"\\server\share\photos")));
        assert!(network(Path::new(r"\\?\UNC\server\share")));
        assert!(!network(Path::new(r"\\?\C:\photos")));
        assert!(!network(Path::new(r"C:\photos")));
    }

    fn app(root: &Path) -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            None,
            Ok(crate::storage::Store::in_directories_with_cache(
                root.join("config"),
                root.join("state"),
                root.join("cache"),
            )),
        );
        app.memory_monitor = crate::core::cache::Monitor::disabled();
        app.apply_memory_sample(Some(crate::platform::system_memory::Snapshot {
            total: 8 << 30,
            available: 4 << 30,
        }));
        // Input state (window focus) exists only after a pass.
        let _ = ctx.run_pass(egui::RawInput::default(), |_| {});
        (app, ctx)
    }

    #[test]
    fn idle_prefill_warms_a_bookmark_on_disk_and_yields_to_viewer_work() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let folder = root.path().join("book");
        std::fs::create_dir(&folder).unwrap();
        for i in 1..=3 {
            image::RgbaImage::from_pixel(64, 32, image::Rgba([i * 40, 20, 30, 255]))
                .save(folder.join(format!("{i}.png")))
                .unwrap();
        }
        let (mut app, ctx) = app(root.path());
        assert!(app.settings.prefill_library);
        app.library.items.clear();
        app.library
            .items
            .push(super::super::library::Item::for_test(entry(
                Kind::Bookmark,
                &folder,
                Some(folder.join("2.png")),
            )));
        app.prefill.dirty = true;
        let thumbnails = app.store.as_ref().unwrap().thumbnail_dir();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut finished = 0;
        while finished < 6 {
            assert!(std::time::Instant::now() < deadline, "prefill stalled");
            app.run_prefill(&ctx);
            while let Some(event) = app.media.try_recv() {
                if let MediaEvent::Warmed { id, result, .. } = event {
                    assert!(result.is_ok());
                    assert!(app.prefill_finished(id));
                    finished += 1;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // 3 display images and 3 thumbnails, on disk without textures.
        assert!(app.pictures.is_empty() && app.thumbs.is_empty());
        let stored = || std::fs::read_dir(&thumbnails).map_or(0, |files| files.count());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while stored() < 3 {
            assert!(
                std::time::Instant::now() < deadline,
                "thumbnails not stored"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Viewer work in the same slot replaces a prefill job; it is requeued.
        app.prefill.pending.push_back(Target {
            path: folder.join("1.png"),
            thumbnail: false,
        });
        app.run_prefill(&ctx);
        assert!(app.prefill.ticket.is_some());
        app.prefetch_ticket = Some((u64::MAX, folder.join("3.png"), app.fit_limit));
        app.run_prefill(&ctx);
        assert!(app.prefill.ticket.is_none());
        assert_eq!(app.prefill.pending.len(), 1, "replaced job waits again");
        // Turning the setting off drops the plan.
        app.prefetch_ticket = None;
        app.settings.prefill_library = false;
        app.run_prefill(&ctx);
        assert!(app.prefill.pending.is_empty() && app.prefill.ticket.is_none());
    }

    #[test]
    fn large_prefetched_uploads_wait_for_gestures_and_go_one_per_frame() {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let (mut app, ctx) = app(root.path());
        app.settings.cache_memory_mb = 1024;
        app.files = (0..5).map(|i| PathBuf::from(format!("p{i}.png"))).collect();
        let frame = || media::Frame {
            hdr: None,
            animation: None,
            image: egui::ColorImage::filled([2100, 2000], egui::Color32::WHITE),
            original_size: [2100, 2000],
        };
        let requested = app.fit_limit;
        app.large_uploads = 0;
        app.offer_prefetched(&ctx, 1, app.files[1].clone(), frame(), requested);
        app.offer_prefetched(&ctx, 2, app.files[2].clone(), frame(), requested);
        assert!(app.pictures.contains_key(&app.files[1]));
        assert!(
            !app.pictures.contains_key(&app.files[2]),
            "one large upload per frame"
        );
        assert_eq!(app.held_prefetch.len(), 1);
        app.touch_gesture.start(egui::pos2(10., 10.));
        app.large_uploads = 0;
        app.release_held_prefetch(&ctx);
        assert_eq!(
            app.held_prefetch.len(),
            1,
            "nothing uploads during a gesture"
        );
        app.touch_gesture.cancel();
        app.collect(&ctx);
        assert!(app.pictures.contains_key(&app.files[2]));
        assert!(app.held_prefetch.is_empty());
    }
}
