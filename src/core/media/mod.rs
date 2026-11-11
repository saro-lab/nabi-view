use eframe::egui;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageReader, Limits};
pub(crate) mod animation;
#[cfg(test)]
pub(crate) mod animation_fixtures;
pub(crate) mod avif;
pub(crate) mod color;
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
}

struct Job {
    id: u64,
    path: PathBuf,
    max_side: u32,
}

#[derive(Default)]
struct Queues {
    pending: [Option<Job>; 5],
    ready: [Option<MediaEvent>; 6],
    closed: bool,
}

#[derive(Default)]
struct Shared {
    queues: Mutex<Queues>,
    changed: Condvar,
    current: [AtomicU64; 5],
    visible_epoch: AtomicU64,
    thumbnail_epoch: AtomicU64,
    next_id: AtomicU64,
    disk_cache: Mutex<Option<crate::core::disk_cache::Cache>>,
    folder_stamps: Mutex<(u64, HashMap<PathBuf, FileSortMetadata>)>,
}

/// Two visible image slots share one decoder; directory discovery runs independently.
pub struct Media(Arc<Shared>);

impl Media {
    pub fn new(ctx: egui::Context) -> Self {
        let shared = Arc::new(Shared::default());
        {
            let (shared, ctx) = (Arc::clone(&shared), ctx.clone());
            thread::spawn(move || worker(shared, ctx, &[0, 1, THUMB_SLOT, PREFETCH_SLOT]));
        }
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
        self.submit(slot, path, max_side)
    }

    pub fn request_folder(&self, dir: PathBuf) -> u64 {
        self.submit(FOLDER_SLOT, dir, 0)
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

    pub fn request_thumbnail(&self, path: PathBuf, max_side: u32) -> u64 {
        self.submit(THUMB_SLOT, path, max_side.clamp(1, 256))
    }

    pub fn clear_thumbnail(&self) {
        self.cancel(THUMB_SLOT);
    }

    pub fn request_prefetch(&self, path: PathBuf, max_side: u32) -> u64 {
        self.submit(PREFETCH_SLOT, path, max_side)
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
    }

    fn submit(&self, slot: usize, path: PathBuf, max_side: u32) -> u64 {
        let mut q = self.0.queues.lock().unwrap();
        let id = self.0.next_id.fetch_add(1, AtomicOrdering::Relaxed) + 1;
        self.0.current[slot].store(id, AtomicOrdering::Relaxed);
        if slot < FOLDER_SLOT {
            self.0.visible_epoch.fetch_add(1, AtomicOrdering::Relaxed);
        }
        if slot == THUMB_SLOT {
            self.0.thumbnail_epoch.fetch_add(1, AtomicOrdering::Relaxed);
        }
        q.pending[slot] = Some(Job { id, path, max_side });
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

fn worker(shared: Arc<Shared>, ctx: egui::Context, slots: &[usize]) {
    let writer = persistent::Writer::new();
    loop {
        let (slot, job, visible_epoch, thumbnail_epoch) = {
            let mut q = shared.queues.lock().unwrap();
            loop {
                if q.closed {
                    return;
                }
                let next = slots
                    .iter()
                    .copied()
                    .filter_map(|s| {
                        q.pending[s].as_ref().map(|job| {
                            (
                                if s == PREFETCH_SLOT {
                                    2
                                } else if s == THUMB_SLOT {
                                    1
                                } else {
                                    0
                                },
                                job.id,
                                s,
                            )
                        })
                    })
                    .min();
                if let Some((_, _, slot)) = next {
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
        let preempted = || {
            (slot >= THUMB_SLOT
                && shared.visible_epoch.load(AtomicOrdering::Relaxed) != visible_epoch)
                || (slot == PREFETCH_SLOT
                    && shared.thumbnail_epoch.load(AtomicOrdering::Relaxed) != thumbnail_epoch)
        };
        let cache = shared.disk_cache.lock().unwrap().clone();
        let cache_key = cache
            .as_ref()
            .and_then(|cache| cache.key(&job.path, job.max_side));
        let cached = cache
            .as_ref()
            .zip(cache_key.as_ref())
            .and_then(|(cache, key)| cache.load(key));
        let result = if let Some(cached) = cached {
            Ok(Frame {
                hdr: None,
                animation: None,
                image: cached.image,
                original_size: cached.original_size,
            })
        } else {
            let result = decode(&job.path, job.max_side, &|| live() && !preempted());
            if live()
                && !preempted()
                && let Ok(frame) = &result
                && frame.animation.is_none()
                && frame.hdr.is_none()
                && let Some((cache, key)) = cache.zip(cache_key)
            {
                writer.store(cache, key, frame);
            }
            result
        };
        if slot >= THUMB_SLOT {
            let mut q = shared.queues.lock().unwrap();
            if live() && preempted() && !q.closed {
                q.pending[slot] = Some(job);
                continue;
            }
        }
        let event = if slot == THUMB_SLOT {
            MediaEvent::Thumbnail {
                id,
                path: job.path,
                result,
            }
        } else if slot == PREFETCH_SLOT {
            MediaEvent::Prefetched {
                id,
                path: job.path,
                result,
            }
        } else {
            MediaEvent::Image {
                id,
                path: job.path,
                result,
            }
        };
        let mut q = shared.queues.lock().unwrap();
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

/// Decodes the first frame outside the viewer's queues, for cached thumbnails.
pub(crate) fn decode_still(path: &Path, max_side: u32) -> Result<Frame, String> {
    decode(path, max_side, &|| true)
}

fn decode(path: &Path, max_side: u32, live: &dyn Fn() -> bool) -> Result<Frame, String> {
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
    let [out_w, out_h] = output_size(w, h, max_side);
    let image =
        DynamicImage::from_decoder(decoder).map_err(|e| format!("Image decoding failed. {e}"))?;
    if !live() {
        return Err("Cancelled".into());
    }
    let mut image = if (out_w, out_h) == (w, h) {
        image
    } else {
        image.thumbnail_exact(out_w, out_h)
    };
    if !live() {
        return Err("Cancelled".into());
    }
    image = color::to_srgb(image, icc.as_deref())?;
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
