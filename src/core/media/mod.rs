use eframe::egui;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageReader, Limits};
pub(crate) mod animation;
#[cfg(test)]
pub(crate) mod animation_fixtures;
pub(crate) mod avif;
pub(crate) mod color;
mod jpeg;
mod persistent;
pub(crate) mod regions;
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

pub const EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "apng", "gif", "webp", "bmp", "tif", "tiff", "avif", "heic",
    "heif", "jxl", "dng", "cr2", "cr3", "nef", "arw", "raf", "svg", "ico", "tga", "qoi", "pbm",
    "pgm", "ppm", "pam", "exr", "hdr", "dds", "ff",
];
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PIXELS: u64 = 64_000_000;
const MAX_OUTPUT_PIXELS: u64 = 6 * 1024 * 1024;
const MAX_EDGE: u32 = 32_768;
const FOLDER_SLOT: usize = 2;
const THUMB_SLOT: usize = 3;
const PREFETCH_SLOT: usize = 4;
const WATCH_SLOT: usize = 5;
const FOLDER_POLL_INTERVAL: Duration = Duration::from_millis(350);

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            EXTENSIONS
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
}

/// Immediate image-file candidates, without decoding pixels or entering subfolders.
/// Directory names with image extensions and broken links are not image files.
pub(crate) fn images_in_folder(dir: &Path) -> io::Result<impl Iterator<Item = PathBuf>> {
    Ok(fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_supported(path) && path.is_file()))
}

pub struct AnimationFrame {
    pub hdr: Option<color::HdrImage>,
    pub image: egui::ColorImage,
    pub delay: Duration,
}

pub struct Frame {
    pub hdr: Option<color::HdrImage>,
    pub animation: Option<animation::Animation>,
    pub image: egui::ColorImage,
    pub original_size: [u32; 2],
}

/// A complete folder snapshot and current files whose cached contents are stale.
pub struct FolderUpdate {
    pub files: Vec<PathBuf>,
    pub changed: Vec<PathBuf>,
}

pub enum MediaEvent {
    Image {
        id: u64,
        path: PathBuf,
        result: Result<Frame, String>,
    },
    Thumbnail {
        id: u64,
        path: PathBuf,
        result: Result<Frame, String>,
    },
    Prefetched {
        id: u64,
        path: PathBuf,
        result: Result<Frame, String>,
    },
    Folder {
        id: u64,
        path: PathBuf,
        result: Result<Vec<PathBuf>, String>,
    },
    FolderChanged {
        id: u64,
        path: PathBuf,
        result: Result<FolderUpdate, String>,
    },
    /// A disk-only prefetch: the display entry is on disk, no pixels returned.
    Warmed {
        id: u64,
        path: PathBuf,
        result: Result<(), String>,
    },
}

/// Order of speculative work, most urgent first. Jobs sharing a decoder run
/// in this order; `Lead` and above keep running when the current page changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    /// A strip thumbnail currently on screen.
    VisibleThumbnail,
    /// The next page or spread in the reading direction.
    Lead,
    /// Thumbnails around the current page.
    RangeThumbnail,
    /// The page or spread just behind the current one.
    Opposite,
    /// Further pages of the working set.
    #[default]
    Ahead,
}

struct Job {
    id: u64,
    path: PathBuf,
    max_side: u32,
    urgency: Urgency,
    /// Decode only to fill the disk cache; skip when an entry already exists.
    disk_only: bool,
}

/// Display cache sizes. A request decodes at the smallest tier that covers it
/// (larger requests keep their own size), so a resized window hits the cache.
const DISPLAY_TIERS: [u32; 3] = [1024, 2048, 4096];

pub(crate) fn display_tier(max_side: u32) -> u32 {
    DISPLAY_TIERS
        .into_iter()
        .find(|&tier| tier >= max_side)
        .unwrap_or(max_side)
}

/// Thumbnails that reduced JPEG decoding serves run on their own decoder, so
/// at most two full-size decodes (visible and background) overlap.
fn reduced_thumbnail(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["jpg", "jpeg", "jfif"]
                .iter()
                .any(|e| ext.eq_ignore_ascii_case(e))
        })
}

#[derive(Default)]
struct Queues {
    pending: [Option<Job>; 5],
    ready: [Option<MediaEvent>; 6],
    visible_active: bool,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    queues: Mutex<Queues>,
    changed: Condvar,
    current: [AtomicU64; 5],
    visible_epoch: AtomicU64,
    thumbnail_epoch: AtomicU64,
    /// Folder generation listed once without the change poll.
    unwatched: AtomicU64,
    next_id: AtomicU64,
    disk_cache: Mutex<Option<crate::core::disk_cache::Cache>>,
    folder_stamps: Mutex<(u64, HashMap<PathBuf, FileSortMetadata>)>,
}

/// Visible images and speculative work each have one decoder. A slow thumbnail
/// cannot hold the current page behind a non-interruptible codec call.
pub struct Media(Arc<Shared>);

impl Media {
    pub fn new(ctx: egui::Context) -> Self {
        let shared = Arc::new(Shared::default());
        spawn_decode_workers(&shared, &ctx, decode);
        {
            let (shared, ctx) = (Arc::clone(&shared), ctx.clone());
            thread::spawn(move || folder_worker(shared, ctx));
        }
        Self(shared)
    }

    pub(crate) fn configure_cache(&self, cache: Option<crate::core::disk_cache::Cache>) {
        *self.0.disk_cache.lock().unwrap() = cache;
    }

    pub fn request_image_in_slot(&self, slot: usize, path: PathBuf, max_side: u32) -> u64 {
        assert!(slot < FOLDER_SLOT);
        self.submit(slot, path, max_side, Urgency::VisibleThumbnail, false)
    }

    pub fn request_folder(&self, dir: PathBuf) -> u64 {
        self.submit(FOLDER_SLOT, dir, 0, Urgency::VisibleThumbnail, false)
    }

    /// Stops polling folder generation `id` after its listing. Mobile imports
    /// are app-owned copies: provider changes arrive as a new import instead.
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    pub fn stop_folder_watch(&self, id: u64) {
        let _q = self.0.queues.lock().unwrap();
        self.0.unwatched.store(id, AtomicOrdering::Relaxed);
        self.0.changed.notify_all();
    }

    /// Memory-only metadata captured by the folder worker. Call on folder
    /// events, not every frame; stale folder generations cannot leak across tabs.
    pub(crate) fn folder_stamps(&self, id: u64) -> HashMap<PathBuf, FileSortMetadata> {
        let snapshot = self.0.folder_stamps.lock().unwrap();
        if snapshot.0 == id {
            snapshot.1.clone()
        } else {
            HashMap::new()
        }
    }

    #[cfg(test)]
    pub fn request_thumbnail(&self, path: PathBuf, max_side: u32) -> u64 {
        self.request_thumbnail_with(path, max_side, Urgency::VisibleThumbnail)
    }

    pub fn request_thumbnail_with(&self, path: PathBuf, max_side: u32, urgency: Urgency) -> u64 {
        self.submit(THUMB_SLOT, path, max_side.clamp(1, 256), urgency, false)
    }

    pub fn clear_thumbnail(&self) {
        self.cancel(THUMB_SLOT);
    }

    #[cfg(test)]
    pub fn request_prefetch(&self, path: PathBuf, max_side: u32) -> u64 {
        self.request_prefetch_with(path, max_side, Urgency::Ahead)
    }

    pub fn request_prefetch_with(&self, path: PathBuf, max_side: u32, urgency: Urgency) -> u64 {
        self.submit(PREFETCH_SLOT, path, max_side, urgency, false)
    }

    /// Fills the disk cache for a page outside the memory working set.
    /// Answers with `MediaEvent::Warmed` in the prefetch slot.
    pub fn request_disk_warm(&self, path: PathBuf, max_side: u32, urgency: Urgency) -> u64 {
        self.submit(PREFETCH_SLOT, path, max_side, urgency, true)
    }

    /// Fills the shared thumbnail store for a page that is not on screen.
    /// Answers with `MediaEvent::Warmed` in the thumbnail slot.
    pub fn request_thumbnail_warm(&self, path: PathBuf) -> u64 {
        self.submit(
            THUMB_SLOT,
            path,
            crate::core::library::THUMBNAIL_EDGE,
            Urgency::RangeThumbnail,
            true,
        )
    }

    /// Share of the disk cache limit in use, when this process knows it.
    pub(crate) fn disk_cache_share(&self) -> Option<f64> {
        let cache = self.0.disk_cache.lock().unwrap().clone()?;
        let bytes = cache.estimated_bytes()?;
        (cache.limit() > 0).then(|| bytes as f64 / cache.limit() as f64)
    }

    pub fn clear_prefetch(&self) {
        self.cancel(PREFETCH_SLOT);
    }

    pub fn clear_slot(&self, slot: usize) {
        assert!(slot < FOLDER_SLOT);
        self.cancel(slot);
    }

    fn cancel(&self, slot: usize) {
        let mut q = self.0.queues.lock().unwrap();
        self.0.current[slot].store(0, AtomicOrdering::Relaxed);
        q.pending[slot] = None;
        q.ready[slot] = None;
        self.0.changed.notify_all();
    }

    fn submit(
        &self,
        slot: usize,
        path: PathBuf,
        max_side: u32,
        urgency: Urgency,
        disk_only: bool,
    ) -> u64 {
        let mut q = self.0.queues.lock().unwrap();
        let id = self.0.next_id.fetch_add(1, AtomicOrdering::Relaxed) + 1;
        self.0.current[slot].store(id, AtomicOrdering::Relaxed);
        if slot < FOLDER_SLOT {
            self.0.visible_epoch.fetch_add(1, AtomicOrdering::Relaxed);
        }
        // Only an on-screen thumbnail that shares the background decoder
        // interrupts speculative display work; reduced JPEGs run beside it.
        if slot == THUMB_SLOT && urgency == Urgency::VisibleThumbnail && !reduced_thumbnail(&path) {
            self.0.thumbnail_epoch.fetch_add(1, AtomicOrdering::Relaxed);
        }
        q.pending[slot] = Some(Job {
            id,
            path,
            max_side,
            urgency,
            disk_only,
        });
        q.ready[slot] = None;
        if slot == FOLDER_SLOT {
            q.ready[WATCH_SLOT] = None;
        }
        self.0.changed.notify_all();
        id
    }

    pub fn try_recv(&self) -> Option<MediaEvent> {
        self.0
            .queues
            .lock()
            .unwrap()
            .ready
            .iter_mut()
            .find_map(Option::take)
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        let mut q = self.0.queues.lock().unwrap();
        q.closed = true;
        for id in &self.0.current {
            id.store(0, AtomicOrdering::Relaxed);
        }
        self.0.changed.notify_all();
    }
}

#[derive(Clone, Copy)]
enum DecodeLane {
    Visible,
    Background,
    /// Reduced JPEG thumbnails only, beside the full-size background decoder.
    Thumbnail,
}

impl DecodeLane {
    fn next_slot(self, queues: &Queues) -> Option<usize> {
        if matches!(self, Self::Visible) {
            return [0, 1]
                .into_iter()
                .filter_map(|slot| queues.pending[slot].as_ref().map(|job| (job.id, slot)))
                .min()
                .map(|(_, slot)| slot);
        }
        // Do not start more speculative work while either current page is
        // pending/running. Only work already in a codec may overlap.
        if queues.visible_active || queues.pending[..FOLDER_SLOT].iter().any(Option::is_some) {
            return None;
        }
        [THUMB_SLOT, PREFETCH_SLOT]
            .into_iter()
            .filter_map(|slot| {
                let job = queues.pending[slot].as_ref()?;
                let reduced = slot == THUMB_SLOT && reduced_thumbnail(&job.path);
                (reduced == matches!(self, Self::Thumbnail)).then_some((
                    job.urgency,
                    slot == PREFETCH_SLOT,
                    job.id,
                    slot,
                ))
            })
            .min()
            .map(|(.., slot)| slot)
    }
}

fn spawn_decode_workers<F>(shared: &Arc<Shared>, ctx: &egui::Context, decode_frame: F)
where
    F: Fn(&Path, u32, &dyn Fn() -> bool) -> Result<Frame, String> + Send + Sync + 'static,
{
    // One disk writer with a small priority queue serves both decoders.
    let writer = Arc::new(persistent::Writer::new());
    let decode_frame = Arc::new(decode_frame);
    for lane in [
        DecodeLane::Visible,
        DecodeLane::Background,
        DecodeLane::Thumbnail,
    ] {
        let (shared, ctx, writer, decode_frame) = (
            Arc::clone(shared),
            ctx.clone(),
            Arc::clone(&writer),
            Arc::clone(&decode_frame),
        );
        thread::Builder::new()
            .name(
                match lane {
                    DecodeLane::Visible => "nabi-visible",
                    DecodeLane::Background => "nabi-background",
                    DecodeLane::Thumbnail => "nabi-thumbnail",
                }
                .into(),
            )
            .spawn(move || worker(shared, ctx, lane, writer, decode_frame))
            .expect("image decoder worker");
    }
}

fn worker<F>(
    shared: Arc<Shared>,
    ctx: egui::Context,
    lane: DecodeLane,
    writer: Arc<persistent::Writer>,
    decode_frame: Arc<F>,
) where
    F: Fn(&Path, u32, &dyn Fn() -> bool) -> Result<Frame, String>,
{
    if !matches!(lane, DecodeLane::Visible) {
        crate::platform::worker_priority::background();
    }
    loop {
        let (slot, job, visible_epoch, thumbnail_epoch) = {
            let mut q = shared.queues.lock().unwrap();
            loop {
                if q.closed {
                    return;
                }
                if let Some(slot) = lane.next_slot(&q) {
                    if slot < FOLDER_SLOT {
                        q.visible_active = true;
                    }
                    break (
                        slot,
                        q.pending[slot].take().unwrap(),
                        shared.visible_epoch.load(AtomicOrdering::Relaxed),
                        shared.thumbnail_epoch.load(AtomicOrdering::Relaxed),
                    );
                }
                q = shared.changed.wait(q).unwrap();
            }
        };
        let id = job.id;
        let live = || shared.current[slot].load(AtomicOrdering::Relaxed) == id;
        // The on-screen strip and the next page keep their decoder; wider
        // speculative work yields the CPU and resumes later.
        let preemptible = slot >= THUMB_SLOT && job.urgency > Urgency::Lead;
        let preempted = || {
            preemptible
                && (shared.visible_epoch.load(AtomicOrdering::Relaxed) != visible_epoch
                    || (slot == PREFETCH_SLOT
                        && shared.thumbnail_epoch.load(AtomicOrdering::Relaxed) != thumbnail_epoch))
        };
        let result: Result<Option<Frame>, String> = (|| {
            #[cfg(any(target_os = "ios", target_os = "android", test))]
            let provider_thumbnail =
                slot == THUMB_SLOT && crate::core::documents::has_provider_thumbnail(&job.path);
            // Still wanted, regardless of preemption: a finished result is kept.
            let usable = || {
                live() && {
                    #[cfg(any(target_os = "ios", target_os = "android", test))]
                    {
                        crate::core::documents::provider_cache_identity(&job.path).is_ok()
                    }
                    #[cfg(not(any(target_os = "ios", target_os = "android", test)))]
                    {
                        true
                    }
                }
            };
            let active = || usable() && !preempted();
            // Current authorized provider metadata can identify a display cache
            // without copying its original. Other providers retain local keys.
            #[cfg(any(target_os = "ios", target_os = "android", test))]
            if !provider_thumbnail
                && !matches!(
                    crate::core::documents::provider_cache_identity(&job.path),
                    Ok(Some(_))
                )
            {
                crate::core::documents::materialize(&job.path, &active)?;
            }
            #[cfg(not(any(target_os = "ios", target_os = "android", test)))]
            let provider_thumbnail = false;
            let cache = shared.disk_cache.lock().unwrap().clone();
            let cache_current = || *shared.disk_cache.lock().unwrap() == cache;
            // Display work decodes at a fixed tier, so resizing the window
            // reuses entries; thumbnails keep their own small size.
            let side = if slot == THUMB_SLOT {
                job.max_side
            } else {
                display_tier(job.max_side)
            };
            // Strip thumbnails live in the library's thumbnail store, so a page
            // and the cover of its recent/bookmark entry decode only once.
            let thumbnail = cache
                .as_ref()
                .filter(|_| slot == THUMB_SLOT && !provider_thumbnail)
                .and_then(|cache| {
                    // An archive page keys by `archive/inner`, like library covers.
                    let logical = crate::core::archive::logical_page(&job.path);
                    crate::core::library::thumbnail_key(logical.as_deref().unwrap_or(&job.path))
                        .map(|key| (cache.thumbnail_dir(), key, cache.current_generation()))
                });
            // Cache reads take no lock, so the visible lane never waits for
            // (or skips the cache because of) background writes and pruning.
            let cache_key = cache
                .as_ref()
                .filter(|_| slot != THUMB_SLOT || provider_thumbnail)
                .and_then(|cache| {
                    #[cfg(any(target_os = "ios", target_os = "android", test))]
                    if provider_thumbnail {
                        return cache.provider_thumbnail_key(&job.path, job.max_side);
                    }
                    cache.key(&job.path, side)
                });
            if job.disk_only {
                if let Some((dir, key, _)) = &thumbnail {
                    if crate::core::library::thumbnail_file(dir, key).is_file() {
                        return Ok(None);
                    }
                } else {
                    let Some((cache, key)) = cache.as_ref().zip(cache_key.as_ref()) else {
                        return Err("The disk cache is off.".into());
                    };
                    if cache.contains(key) {
                        return Ok(None);
                    }
                }
            }
            let cached = if let Some((dir, key, _)) = &thumbnail {
                crate::core::library::load_thumbnail(dir, key)
                    .ok()
                    .map(|image| Frame {
                        original_size: [image.size[0] as u32, image.size[1] as u32],
                        image,
                        animation: None,
                        hdr: None,
                    })
            } else {
                cache
                    .as_ref()
                    .zip(cache_key.as_ref())
                    .and_then(|(cache, key)| cache.load(key))
                    .or_else(|| {
                        // A larger tier from a bigger window is better than
                        // decoding the original again.
                        let cache = cache.as_ref().filter(|_| slot != THUMB_SLOT)?;
                        DISPLAY_TIERS
                            .into_iter()
                            .filter(|&tier| tier > side)
                            .find_map(|tier| cache.load(&cache.key(&job.path, tier)?))
                    })
                    .map(|cached| Frame {
                        hdr: None,
                        animation: None,
                        image: cached.image,
                        original_size: cached.original_size,
                    })
            }
            .filter(|_| usable() && cache_current());
            if let Some(cached) = cached {
                Ok(Some(cached))
            } else {
                // A stable-key miss still materializes through the guarded
                // loader before the decoder touches a private placeholder.
                let result = (|| {
                    #[cfg(any(target_os = "ios", target_os = "android", test))]
                    {
                        if provider_thumbnail {
                            let image = crate::core::documents::provider_thumbnail(
                                &job.path,
                                job.max_side,
                                &active,
                            )?;
                            return Ok(Frame {
                                original_size: [image.size[0] as u32, image.size[1] as u32],
                                image,
                                animation: None,
                                hdr: None,
                            });
                        }
                        crate::core::documents::materialize(&job.path, &active)?;
                    }
                    decode_frame(&job.path, side, &active)
                })();
                // A finished decode is stored even when it was preempted or
                // superseded meanwhile: the work is already paid for.
                if cache_current()
                    && let Ok(frame) = &result
                    && let Some(((_, key, generation), cache)) =
                        thumbnail.as_ref().zip(cache.as_ref())
                {
                    // Like the library list: the first (SDR) frame is the thumbnail.
                    let _ = crate::core::library::store_thumbnail_image(
                        cache,
                        *generation,
                        key,
                        &frame.image,
                        || cache.limit(),
                    );
                } else if cache_current()
                    && let Ok(frame) = &result
                    && frame.animation.is_none()
                    && frame.hdr.is_none()
                    && let Some((cache, key)) = cache.clone().zip(cache_key)
                {
                    let priority = match slot {
                        THUMB_SLOT => persistent::Priority::Thumbnail,
                        PREFETCH_SLOT => persistent::Priority::Prefetch,
                        _ => persistent::Priority::Visible,
                    };
                    writer.store(cache, key, frame, priority);
                }
                if !usable() {
                    return Err("Cancelled".into());
                }
                result.map(|frame| (!job.disk_only).then_some(frame))
            }
        })();
        // Only an interrupted decode starts over; a finished one is published.
        if slot >= THUMB_SLOT && result.is_err() {
            let mut q = shared.queues.lock().unwrap();
            if live() && preempted() && !q.closed {
                q.pending[slot] = Some(job);
                continue;
            }
        }
        let event = if job.disk_only {
            MediaEvent::Warmed {
                id,
                path: job.path,
                result: result.map(drop),
            }
        } else {
            let result =
                result.and_then(|frame| frame.ok_or_else(|| "No image was decoded.".to_owned()));
            let path = job.path;
            match slot {
                THUMB_SLOT => MediaEvent::Thumbnail { id, path, result },
                PREFETCH_SLOT => MediaEvent::Prefetched { id, path, result },
                _ => MediaEvent::Image { id, path, result },
            }
        };
        let mut q = shared.queues.lock().unwrap();
        if slot < FOLDER_SLOT {
            q.visible_active = false;
            shared.changed.notify_all();
        }
        if live() && !q.closed {
            q.ready[slot] = Some(event);
            drop(q);
            ctx.request_repaint();
        }
    }
}

/// Upper bound for retained RGBA output, excluding decoder scratch/input memory.
/// Prefetch retains only the first frame; playback starts when visible.
pub fn prefetch_output_budget(max_side: u32) -> usize {
    let pixels = (u64::from(max_side) * u64::from(max_side)).min(MAX_OUTPUT_PIXELS);
    // SDR texture plus retained float HDR source and FP16 texture (worst case).
    (pixels * 28) as usize
}

/// Ordering keys only: archive timestamps may have no timezone, so callers must
/// not display `modified` as an epoch timestamp or compare different sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileSortMetadata {
    pub bytes: u64,
    pub modified: Option<u128>,
}

#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    bytes: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct FolderSnapshot {
    files: Vec<PathBuf>,
    stamps: HashMap<PathBuf, FileStamp>,
}

fn scan_snapshot(dir: &Path, live: &dyn Fn() -> bool) -> Result<FolderSnapshot, String> {
    #[cfg(all(target_os = "macos", not(test)))]
    let access = crate::platform::mac_access::resolve(dir)?;
    #[cfg(all(target_os = "macos", not(test)))]
    let dir = access.path.as_path();
    let mut files = Vec::new();
    let mut stamps = HashMap::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // Keep watching the path so recreation can recover without reopening.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(FolderSnapshot::default()),
        Err(e) => return Err(format!("Could not open the folder. {e}")),
    };
    for entry in entries {
        if !live() {
            return Err("Cancelled".into());
        }
        let entry = entry.map_err(|e| format!("Could not read the folder. {e}"))?;
        let path = entry.path();
        if !is_supported(&path) {
            continue;
        }
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            // Like the folder listing, leave out a file that cannot be read.
            Err(_) => continue,
        };
        if metadata.is_file() {
            #[cfg(any(target_os = "ios", target_os = "android", test))]
            if let Some((bytes, modified)) = crate::core::documents::deferred_stamp(&path) {
                // Filling an app-owned placeholder is not a source edit. Keep
                // its listing identity stable while workers fetch its pixels.
                stamps.insert(
                    path.clone(),
                    FileStamp {
                        bytes,
                        modified,
                        created: None,
                    },
                );
                files.push(path);
                continue;
            }
            stamps.insert(
                path.clone(),
                FileStamp {
                    bytes: metadata.len(),
                    modified: metadata.modified().ok(),
                    created: metadata.created().ok(),
                },
            );
            files.push(path);
        }
    }
    files.sort_unstable_by(|a, b| compare_file_names(a, b));
    Ok(FolderSnapshot { files, stamps })
}

/// Natural filename order, with the original name breaking case/numeric ties.
pub fn compare_file_names(a: &Path, b: &Path) -> Ordering {
    let (a, b) = (
        a.file_name().unwrap_or(a.as_os_str()),
        b.file_name().unwrap_or(b.as_os_str()),
    );
    natural_cmp(
        a.to_string_lossy().as_bytes(),
        b.to_string_lossy().as_bytes(),
    )
    .then_with(|| a.cmp(b))
}

/// Natural order of relative paths, one component at a time, so the pages of
/// a folder inside an archive stay together.
pub(crate) fn compare_paths(a: &Path, b: &Path) -> Ordering {
    let mut a_parts = a.components();
    let mut b_parts = b.components();
    loop {
        match (a_parts.next(), b_parts.next()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let (x, y) = (x.as_os_str(), y.as_os_str());
                let order = natural_cmp(
                    x.to_string_lossy().as_bytes(),
                    y.to_string_lossy().as_bytes(),
                )
                .then_with(|| x.cmp(y));
                if order.is_ne() {
                    return order;
                }
            }
        }
    }
}

fn folder_worker(shared: Arc<Shared>, ctx: egui::Context) {
    let mut active: Option<Job> = None;
    let mut previous: Option<FolderSnapshot> = None;
    let mut last_error: Option<String> = None;
    let mut deadline = Instant::now();
    loop {
        let initial = {
            let mut q = shared.queues.lock().unwrap();
            loop {
                if q.closed {
                    return;
                }
                if let Some(job) = q.pending[FOLDER_SLOT].take() {
                    active = Some(job);
                    previous = None;
                    last_error = None;
                    break true;
                }
                if active
                    .as_ref()
                    .is_some_and(|job| shared.unwatched.load(AtomicOrdering::Relaxed) == job.id)
                {
                    // The listing was published; this folder is not watched.
                    active = None;
                    continue;
                }
                if active.is_some() {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break false;
                    }
                    q = shared.changed.wait_timeout(q, remaining).unwrap().0;
                } else {
                    q = shared.changed.wait(q).unwrap();
                }
            }
        };
        let job = active.as_ref().unwrap();
        let live = || shared.current[FOLDER_SLOT].load(AtomicOrdering::Relaxed) == job.id;
        let result = scan_snapshot(&job.path, &live);
        // Scan time is added to the interval; there can only be one scan in flight.
        deadline = Instant::now() + FOLDER_POLL_INTERVAL;
        let mut q = shared.queues.lock().unwrap();
        if !live() || q.closed {
            continue;
        }
        let mut published = false;
        match result {
            Ok(snapshot) => {
                if initial || previous.as_ref() != Some(&snapshot) || last_error.is_some() {
                    *shared.folder_stamps.lock().unwrap() = (
                        job.id,
                        snapshot
                            .stamps
                            .iter()
                            .map(|(path, stamp)| {
                                let modified = stamp.modified.map(|time| {
                                    match time.duration_since(SystemTime::UNIX_EPOCH) {
                                        Ok(value) => (1u128 << 127) + value.as_nanos(),
                                        Err(value) => (1u128 << 127) - value.duration().as_nanos(),
                                    }
                                });
                                (
                                    path.clone(),
                                    FileSortMetadata {
                                        bytes: stamp.bytes,
                                        modified,
                                    },
                                )
                            })
                            .collect(),
                    );
                }
                if initial {
                    q.ready[FOLDER_SLOT] = Some(MediaEvent::Folder {
                        id: job.id,
                        path: job.path.clone(),
                        result: Ok(snapshot.files.clone()),
                    });
                    published = true;
                } else if previous.as_ref() != Some(&snapshot) || last_error.is_some() {
                    let changed = snapshot
                        .stamps
                        .iter()
                        .filter_map(|(path, stamp)| {
                            (previous.as_ref().and_then(|s| s.stamps.get(path)) != Some(stamp))
                                .then_some(path.clone())
                        })
                        .collect();
                    publish_folder_update(
                        &mut q,
                        job,
                        FolderUpdate {
                            files: snapshot.files.clone(),
                            changed,
                        },
                    );
                    published = true;
                }
                last_error = None;
                previous = Some(snapshot);
            }
            Err(error) => {
                if initial {
                    q.ready[FOLDER_SLOT] = Some(MediaEvent::Folder {
                        id: job.id,
                        path: job.path.clone(),
                        result: Err(error.clone()),
                    });
                    published = true;
                } else if last_error.as_ref() != Some(&error) && q.ready[WATCH_SLOT].is_none() {
                    q.ready[WATCH_SLOT] = Some(MediaEvent::FolderChanged {
                        id: job.id,
                        path: job.path.clone(),
                        result: Err(error.clone()),
                    });
                    published = true;
                }
                last_error = Some(error);
            }
        }
        drop(q);
        if published {
            ctx.request_repaint();
        }
    }
}

fn publish_folder_update(q: &mut Queues, job: &Job, mut update: FolderUpdate) {
    // Preserve invalidations when multiple changes arrive before the UI reads.
    // Removed paths need no invalidation: the receiver prunes by the full list.
    if let Some(MediaEvent::FolderChanged {
        id,
        result: Ok(pending),
        ..
    }) = q.ready[WATCH_SLOT].take()
        && id == job.id
    {
        let current: HashSet<_> = update.files.iter().collect();
        let mut changed: HashSet<_> = update.changed.into_iter().collect();
        changed.extend(
            pending
                .changed
                .into_iter()
                .filter(|path| current.contains(path)),
        );
        update.changed = changed.into_iter().collect();
    }
    q.ready[WATCH_SLOT] = Some(MediaEvent::FolderChanged {
        id: job.id,
        path: job.path.clone(),
        result: Ok(update),
    });
}

fn numeric_prefix(input: &[u8]) -> (&[u8], &[u8]) {
    let end = input
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(input.len());
    let (digits, tail) = input.split_at(end);
    let start = digits
        .iter()
        .position(|&b| b != b'0')
        .unwrap_or(digits.len());
    (&digits[start..], tail)
}

fn natural_cmp(mut a: &[u8], mut b: &[u8]) -> Ordering {
    while let (Some((&x, ar)), Some((&y, br))) = (a.split_first(), b.split_first()) {
        let cmp = if x.is_ascii_digit() && y.is_ascii_digit() {
            let (xs, xt) = numeric_prefix(a);
            let (ys, yt) = numeric_prefix(b);
            (a, b) = (xt, yt);
            xs.len().cmp(&ys.len()).then_with(|| xs.cmp(ys))
        } else {
            (a, b) = (ar, br);
            x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase())
        };
        if cmp != Ordering::Equal {
            return cmp;
        }
    }
    a.len().cmp(&b.len())
}

struct CancelReader<'a> {
    file: fs::File,
    live: &'a dyn Fn() -> bool,
}

impl Read for CancelReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !(self.live)() {
            return Err(io::Error::other("Cancelled"));
        }
        self.file.read(buf)
    }
}

impl Seek for CancelReader<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if !(self.live)() {
            return Err(io::Error::other("Cancelled"));
        }
        self.file.seek(pos)
    }
}

fn output_size(w: u32, h: u32, max_side: u32) -> [u32; 2] {
    let edge = max_side.clamp(1, 8192);
    let scale = (f64::from(edge) / f64::from(w.max(h)))
        .min((MAX_OUTPUT_PIXELS as f64 / (f64::from(w) * f64::from(h))).sqrt())
        .min(1.0);
    [
        (f64::from(w) * scale).floor().max(1.0) as u32,
        (f64::from(h) * scale).floor().max(1.0) as u32,
    ]
}

/// Android's ImageDecoder scales while decoding (and reads HEIC/HEIF, which
/// the Rust decoders cannot here). Display keeps the Rust path for formats
/// with animation, alpha regions or HDR; thumbnails take only the first frame.
/// None falls back to the Rust decoders.
#[cfg(target_os = "android")]
fn android_decode(
    path: &Path,
    extension: &str,
    max_side: u32,
    live: &dyn Fn() -> bool,
) -> Result<Option<Frame>, String> {
    let thumbnail = max_side <= crate::core::library::THUMBNAIL_EDGE;
    let eligible = match extension {
        "jpg" | "jpeg" | "jfif" | "heic" | "heif" => true,
        "png" | "webp" | "bmp" => thumbnail,
        _ => false,
    };
    if !eligible {
        return Ok(None);
    }
    if !live() {
        return Err("Cancelled".into());
    }
    let Some((image, original)) =
        crate::platform::android_media::decode_scaled(path, max_side, MAX_OUTPUT_PIXELS)
    else {
        return Ok(None);
    };
    if !live() {
        return Err("Cancelled".into());
    }
    Ok(Some(Frame {
        hdr: None,
        animation: None,
        original_size: upright_original(original, image.size),
        image,
    }))
}

/// The OS decoder applies EXIF orientation to its output; report the original
/// size in the same orientation even if the header size was pre-rotation.
#[cfg(any(target_os = "android", test))]
fn upright_original(original: [u32; 2], output: [usize; 2]) -> [u32; 2] {
    let landscape = |w: u64, h: u64| w > h;
    let [ow, oh] = original;
    let [w, h] = output;
    if w != h && ow != oh && landscape(w as u64, h as u64) != landscape(ow.into(), oh.into()) {
        [oh, ow]
    } else {
        original
    }
}

/// OS-decoded pixels through the same color management as the Rust path.
fn scaled_frame(scaled: crate::platform::platform_image::Scaled) -> Result<Frame, String> {
    let image = color::to_srgb(
        DynamicImage::ImageRgba8(scaled.image),
        scaled.icc.as_deref(),
    )?;
    let (image, hdr) = color::display(image);
    Ok(Frame {
        hdr,
        animation: None,
        image,
        original_size: scaled.original,
    })
}

/// Decodes the first frame outside the viewer's queues, for cached thumbnails.
pub(crate) fn decode_still(path: &Path, max_side: u32) -> Result<Frame, String> {
    decode(path, max_side, &|| true)
}

fn decode(path: &Path, max_side: u32, live: &dyn Fn() -> bool) -> Result<Frame, String> {
    #[cfg(any(target_os = "ios", target_os = "android", test))]
    crate::core::documents::materialize(path, live)?;
    #[cfg(all(target_os = "macos", not(test)))]
    let access = crate::platform::mac_access::resolve(path)?;
    #[cfg(all(target_os = "macos", not(test)))]
    let path = access.path.as_path();
    let meta = fs::metadata(path).map_err(|e| format!("Could not open the image. {e}"))?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err("Choose a regular image file no larger than 256 MiB.".into());
    }
    let extension = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    #[cfg(target_os = "android")]
    if let Some(frame) = android_decode(path, &extension, max_side, live)? {
        return Ok(frame);
    }
    if extension == "png"
        && let Some(frame) = regions::png_preview(path, max_side, live)?
    {
        return Ok(frame);
    }
    if extension == "avif" {
        return avif::decode(path, max_side, live);
    }
    if matches!(
        extension.as_str(),
        "heic" | "heif" | "dng" | "cr2" | "cr3" | "nef" | "arw" | "raf"
    ) {
        // The OS codec scales while decoding and applies EXIF orientation.
        if let Some(scaled) =
            crate::platform::platform_image::decode_scaled(path, max_side, MAX_OUTPUT_PIXELS)?
        {
            if !live() {
                return Err("Cancelled".into());
            }
            return scaled_frame(scaled);
        }
        let image = crate::platform::platform_image::decode(path)?;
        if !live() {
            return Err("Cancelled".into());
        }
        let (w, h) = image.dimensions();
        let [out_w, out_h] = output_size(w, h, max_side);
        let rgba = image.thumbnail_exact(out_w, out_h).into_rgba8();
        return Ok(Frame {
            hdr: None,
            animation: None,
            image: egui::ColorImage::from_rgba_unmultiplied(
                [out_w as usize, out_h as usize],
                rgba.as_raw(),
            ),
            original_size: [w, h],
        });
    }
    if extension == "svg" {
        let data = fs::read(path).map_err(|e| e.to_string())?;
        let mut options = resvg::usvg::Options::default();
        options.image_href_resolver.resolve_string = Box::new(|_, _| None);
        options.fontdb_mut().load_system_fonts();
        let tree = resvg::usvg::Tree::from_data(&data, &options).map_err(|e| e.to_string())?;
        let size = tree.size().to_int_size();
        if u64::from(size.width()) * u64::from(size.height()) > MAX_PIXELS {
            return Err("SVG exceeds pixel limit".into());
        }
        let [w, h] = output_size(size.width(), size.height(), max_side);
        let mut pixels = resvg::tiny_skia::Pixmap::new(w, h).ok_or("SVG allocation failed")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(
                w as f32 / tree.size().width(),
                h as f32 / tree.size().height(),
            ),
            &mut pixels.as_mut(),
        );
        if !live() {
            return Err("Cancelled".into());
        }
        return Ok(Frame {
            hdr: None,
            animation: None,
            image: egui::ColorImage::from_rgba_premultiplied(
                [w as usize, h as usize],
                pixels.data(),
            ),
            original_size: [size.width(), size.height()],
        });
    }
    // OS codecs scale in the decoder. WIC decodes JPEG about twice as fast as
    // the Rust paths (O5 A/B, Windows); ImageIO is assumed similar (unmeasured).
    // Embedded profiles go through the same color management.
    #[cfg(any(windows, target_os = "macos", target_os = "ios"))]
    if matches!(extension.as_str(), "jpg" | "jpeg" | "jfif")
        && let Some(scaled) =
            crate::platform::platform_image::decode_scaled(path, max_side, MAX_OUTPUT_PIXELS)?
    {
        if !live() {
            return Err("Cancelled".into());
        }
        return scaled_frame(scaled);
    }
    if matches!(extension.as_str(), "jpg" | "jpeg" | "jfif")
        && let Some(frame) = jpeg::decode_scaled(path, max_side, live)?
    {
        return Ok(frame);
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let reader = CancelReader { file, live };
    let mut reader = ImageReader::new(BufReader::new(reader))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if reader.format().is_none()
        && let Some(format) = image::ImageFormat::from_extension(&extension)
    {
        reader.set_format(format);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    if max_side > 256
        && let Some(frame) = animation::decode_first(path, reader.format(), max_side, live)?
    {
        return Ok(frame);
    }
    reader.limits(limits);
    let mut decoder: Box<dyn ImageDecoder + '_> = if extension == "jxl" {
        let file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut decoder =
            jxl_oxide::integration::JxlDecoder::new(BufReader::new(CancelReader { file, live }))
                .map_err(|e| e.to_string())?;
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_EDGE);
        limits.max_image_height = Some(MAX_EDGE);
        limits.max_alloc = Some(MAX_DECODE_BYTES);
        decoder.set_limits(limits).map_err(|e| e.to_string())?;
        Box::new(decoder)
    } else {
        Box::new(
            reader
                .into_decoder()
                .map_err(|e| format!("Could not read the image. {e}"))?,
        )
    };
    let (w, h) = decoder.dimensions();
    if w == 0
        || h == 0
        || w > MAX_EDGE
        || h > MAX_EDGE
        || u64::from(w) * u64::from(h) > MAX_PIXELS
        || decoder.total_bytes() > MAX_DECODE_BYTES
    {
        return Err("Image dimensions exceed the decoding limit.".into());
    }
    let icc = decoder.icc_profile().ok().flatten();
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let image =
        DynamicImage::from_decoder(decoder).map_err(|e| format!("Image decoding failed. {e}"))?;
    finish_still(image, [w, h], max_side, icc.as_deref(), orientation, live)
}

/// Shared tail of still decoders: `image` may already be reduced (for example
/// by DCT scaling) but `original` is the stored, pre-orientation source size.
fn finish_still(
    image: DynamicImage,
    original: [u32; 2],
    max_side: u32,
    icc: Option<&[u8]>,
    orientation: image::metadata::Orientation,
    live: &dyn Fn() -> bool,
) -> Result<Frame, String> {
    if !live() {
        return Err("Cancelled".into());
    }
    let [w, h] = original;
    let [out_w, out_h] = output_size(w, h, max_side);
    let mut image = if image.dimensions() == (out_w, out_h) {
        image
    } else {
        image.thumbnail_exact(out_w, out_h)
    };
    if !live() {
        return Err("Cancelled".into());
    }
    image = color::to_srgb(image, icc)?;
    image.apply_orientation(orientation);
    let original_size = match orientation {
        image::metadata::Orientation::Rotate90
        | image::metadata::Orientation::Rotate270
        | image::metadata::Orientation::Rotate90FlipH
        | image::metadata::Orientation::Rotate270FlipH => [h, w],
        _ => [w, h],
    };
    let (image, hdr) = color::display(image);
    Ok(Frame {
        hdr,
        animation: None,
        image,
        original_size,
    })
}

#[cfg(test)]
mod tests;
