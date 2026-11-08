//! Decode only the first frame for loading/prefetch. Visible animations own a
//! worker with one queued frame and one in flight, independent of their length.
use super::{
    AnimationFrame, CancelReader, Frame, MAX_DECODE_BYTES, MAX_EDGE, MAX_FILE_BYTES, MAX_PIXELS,
    output_size,
};
use eframe::egui;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, Limits};
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Source {
    path: PathBuf,
    format: ImageFormat,
    max_side: u32,
    size: [u32; 2],
}

pub struct Animation {
    source: Source,
    frame_bytes: usize,
    playback: Option<Playback>,
}

struct Playback {
    frames: Receiver<Result<AnimationFrame, String>>,
    live: Arc<AtomicBool>,
    deadline: Instant,
    finished: bool,
}

// A cancelled native decode may take time to return. Bound live decoder state
// even when rapid navigation/resize starts replacements before it has stopped.
static DECODERS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
struct DecoderPermit;

impl DecoderPermit {
    fn acquire(live: &dyn Fn() -> bool) -> Option<Self> {
        let (lock, changed) = &DECODERS;
        let mut count = lock.lock().unwrap();
        while *count >= 2 && live() {
            count = changed
                .wait_timeout(count, Duration::from_millis(25))
                .unwrap()
                .0;
        }
        if !live() {
            return None;
        }
        *count += 1;
        Some(Self)
    }
}

impl Drop for DecoderPermit {
    fn drop(&mut self) {
        *DECODERS.0.lock().unwrap() -= 1;
        DECODERS.1.notify_all();
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        // Dropping the receiver also wakes a worker blocked on the full queue.
        // Never join a synchronous codec call on the UI thread.
        self.live.store(false, Ordering::Relaxed);
    }
}

impl Animation {
    pub(crate) fn new(
        path: &Path,
        format: ImageFormat,
        max_side: u32,
        size: [u32; 2],
        image: &egui::ColorImage,
    ) -> Self {
        Self {
            source: Source {
                path: path.to_owned(),
                format,
                max_side,
                size,
            },
            frame_bytes: image.pixels.len() * 4,
            playback: None,
        }
    }

    /// Output reservation: one queued frame plus one being decoded/sent.
    /// Codec scratch/input and the separately counted display texture are not included.
    pub fn retained_bytes(&self) -> usize {
        self.playback
            .as_ref()
            .filter(|p| !p.finished)
            .map_or(0, |_| self.frame_bytes * 2)
    }

    pub fn stop(&mut self) {
        self.playback = None;
    }

    pub fn advance(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
    ) -> Result<Option<egui::ColorImage>, String> {
        if self.playback.is_none() {
            self.playback = Some(self.source.start(ctx.clone())?);
        }
        let playback = self.playback.as_mut().unwrap();
        if playback.finished {
            return Ok(None);
        }
        if now < playback.deadline {
            ctx.request_repaint_after(playback.deadline - now);
            return Ok(None);
        }
        match playback.frames.try_recv() {
            Ok(Ok(frame)) => {
                // A late decoder holds the last good frame. Do not skip frames
                // or race through a backlog after a stalled/hidden window.
                playback.deadline = now
                    .checked_add(frame.delay)
                    .ok_or("Invalid animation frame delay")?;
                ctx.request_repaint_after(frame.delay);
                Ok(Some(frame.image))
            }
            Ok(Err(error)) => {
                playback.finished = true;
                Err(error)
            }
            Err(TryRecvError::Disconnected) => {
                playback.finished = true;
                Ok(None)
            }
            // The worker requests a repaint after publishing, without UI polling.
            Err(TryRecvError::Empty) => Ok(None),
        }
    }
}

impl Source {
    fn start(&self, ctx: egui::Context) -> Result<Playback, String> {
        let (send, frames) = mpsc::sync_channel(1);
        let live = Arc::new(AtomicBool::new(true));
        let active = Arc::clone(&live);
        let source = self.clone();
        thread::Builder::new()
            .name("animation".into())
            .spawn(move || {
                let is_live = || active.load(Ordering::Relaxed);
                let Some(_permit) = DecoderPermit::acquire(&is_live) else {
                    return;
                };
                while is_live() {
                    let result = visit(&source, &is_live, &mut |frame, size| {
                        if size != source.size {
                            return Err("Animation dimensions changed; reopen the image".into());
                        }
                        if !is_live() || send.send(Ok(frame)).is_err() {
                            return Ok(false);
                        }
                        ctx.request_repaint();
                        Ok(is_live())
                    });
                    match result {
                        Ok(count) if count > 1 && is_live() => {}
                        Ok(_) => break,
                        Err(error) => {
                            if is_live() && send.send(Err(error)).is_ok() {
                                ctx.request_repaint();
                            }
                            break;
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Playback {
            frames,
            live,
            deadline: Instant::now(),
            finished: false,
        })
    }
}

fn check_work(live: &dyn Fn() -> bool, started: Instant) -> Result<(), String> {
    if !live() {
        return Err("Cancelled".into());
    }
    // Apply the watchdog to each decode, excluding playback/backpressure time.
    if started.elapsed() > Duration::from_secs(5) {
        return Err("Animation frame exceeds decode time limit".into());
    }
    Ok(())
}

fn open_frames<'a>(
    path: &Path,
    format: ImageFormat,
    live: &'a dyn Fn() -> bool,
) -> Result<Option<(image::Frames<'a>, [u32; 2])>, String> {
    let started = Instant::now();
    check_work(live, started)?;
    let file = File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err("Animation exceeds 256 MiB file limit".into());
    }
    let reader = BufReader::new(CancelReader { file, live });
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    let (frames, (w, h)) = match format {
        ImageFormat::Gif => {
            let mut decoder =
                image::codecs::gif::GifDecoder::new(reader).map_err(|e| e.to_string())?;
            decoder.set_limits(limits).map_err(|e| e.to_string())?;
            let size = decoder.dimensions();
            (decoder.into_frames(), size)
        }
        ImageFormat::WebP => {
            let mut decoder =
                image::codecs::webp::WebPDecoder::new(reader).map_err(|e| e.to_string())?;
            if !decoder.has_animation() {
                return Ok(None);
            }
            decoder.set_limits(limits).map_err(|e| e.to_string())?;
            let size = decoder.dimensions();
            (decoder.into_frames(), size)
        }
        ImageFormat::Png => {
            let mut decoder =
                image::codecs::png::PngDecoder::new(reader).map_err(|e| e.to_string())?;
            if !decoder.is_apng().map_err(|e| e.to_string())? {
                return Ok(None);
            }
            decoder.set_limits(limits).map_err(|e| e.to_string())?;
            let size = decoder.dimensions();
            (
                decoder.apng().map_err(|e| e.to_string())?.into_frames(),
                size,
            )
        }
        _ => return Ok(None),
    };
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err("Animation exceeds pixel limit".into());
    }
    check_work(live, started)?;
    Ok(Some((frames, [w, h])))
}

fn next_frame(
    frames: &mut image::Frames<'_>,
    size: [u32; 2],
    max_side: u32,
    live: &dyn Fn() -> bool,
) -> Result<Option<AnimationFrame>, String> {
    let started = Instant::now();
    check_work(live, started)?;
    let next = frames.next();
    check_work(live, started)?;
    let Some(frame) = next else {
        return Ok(None);
    };
    let frame = frame.map_err(|e| e.to_string())?;
    let (n, d) = frame.delay().numer_denom_ms();
    let delay = Duration::from_secs_f64((n as f64 / d.max(1) as f64 / 1000.).max(0.01));
    let [w, h] = output_size(size[0], size[1], max_side);
    let buffer = frame.into_buffer();
    let rgba = if buffer.dimensions() == (w, h) {
        buffer
    } else {
        DynamicImage::ImageRgba8(buffer)
            .thumbnail_exact(w, h)
            .into_rgba8()
    };
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
    check_work(live, started)?;
    Ok(Some(AnimationFrame { image, delay }))
}

pub(super) fn decode_first(
    path: &Path,
    format: Option<ImageFormat>,
    max_side: u32,
    live: &dyn Fn() -> bool,
) -> Result<Option<Frame>, String> {
    let Some(format @ (ImageFormat::Gif | ImageFormat::WebP | ImageFormat::Png)) = format else {
        return Ok(None);
    };
    let Some((mut frames, size)) = open_frames(path, format, live)? else {
        return Ok(None);
    };
    let first = next_frame(&mut frames, size, max_side, live)?.ok_or("Empty animation")?;
    Ok(Some(Frame {
        animation: Some(Animation::new(path, format, max_side, size, &first.image)),
        image: first.image,
        original_size: size,
    }))
}

fn visit(
    source: &Source,
    live: &dyn Fn() -> bool,
    emit: &mut dyn FnMut(AnimationFrame, [u32; 2]) -> Result<bool, String>,
) -> Result<usize, String> {
    if source.format == ImageFormat::Avif {
        return super::avif::visit(
            &source.path,
            source.max_side,
            live,
            &mut |frame, size, _| emit(frame, size),
        );
    }
    let Some((mut frames, size)) = open_frames(&source.path, source.format, live)? else {
        return Err("Image is no longer an animation".into());
    };
    let mut count = 0;
    while let Some(frame) = next_frame(&mut frames, size, source.max_side, live)? {
        count += 1;
        if !emit(frame, size)? {
            break;
        }
    }
    if count == 0 {
        return Err("Empty animation".into());
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::animation_fixtures::gif;

    fn receive(playback: &Playback) -> AnimationFrame {
        playback
            .frames
            .recv_timeout(Duration::from_secs(10))
            .expect("animation worker stalled")
            .unwrap()
    }

    fn assert_worker_stopped(live: Arc<AtomicBool>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(&live) != 1 {
            assert!(
                Instant::now() < deadline,
                "cancelled animation worker is still alive"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!live.load(Ordering::Relaxed));
    }

    #[test]
    fn gif_over_128_mib_and_512_frames_streams_all_pixels_and_loops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.gif");
        gif(&path, 600, 256);
        let decoded = super::super::decode(&path, 1024, &|| true).unwrap();
        assert_eq!(decoded.original_size, [256, 256]);
        assert_eq!(decoded.image.size, [256, 256]);
        let mut animation = decoded.animation.unwrap();
        assert_eq!(
            animation.retained_bytes(),
            0,
            "prefetch must not start playback"
        );
        animation.playback = Some(animation.source.start(egui::Context::default()).unwrap());
        let playback = animation.playback.as_ref().unwrap();
        let mut total_bytes = 0;
        let mut total_delay = Duration::ZERO;
        for index in 0..602 {
            let frame = receive(playback);
            let position = index % 600;
            assert_eq!(frame.image.size, [256, 256]);
            assert_eq!(
                frame.image.pixels[0],
                egui::Color32::from_rgb(position as u8, (position / 256) as u8, 0)
            );
            assert_eq!(
                frame.delay,
                Duration::from_millis(10 + (position % 3) as u64 * 10)
            );
            if index < 600 {
                total_bytes += frame.image.pixels.len() * 4;
                total_delay += frame.delay;
            }
            assert_eq!(animation.retained_bytes(), 2 * 256 * 256 * 4);
        }
        assert!(total_bytes > 128 * 1024 * 1024);
        assert_eq!(total_delay, Duration::from_secs(12));
        let live = Arc::clone(&playback.live);
        animation.stop();
        assert_eq!(animation.retained_bytes(), 0);
        assert_worker_stopped(live);
        assert!(
            super::super::decode(&path, 128, &|| true)
                .unwrap()
                .animation
                .is_none()
        );
        assert!(super::super::decode(&path, 1024, &|| false).is_err());
    }

    #[test]
    fn first_frame_does_not_wait_for_tail_and_corruption_is_reported_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("truncated.gif");
        gif(&path, 1, 16);
        let mut bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.pop(), Some(0x3b)); // replace trailer with incomplete next image
        bytes.extend_from_slice(&[0x2c, 0]);
        std::fs::write(&path, bytes).unwrap();
        let first = super::super::decode(&path, 1024, &|| true).unwrap();
        let source = first.animation.unwrap().source;
        let playback = source.start(egui::Context::default()).unwrap();
        assert_eq!(receive(&playback).image.pixels, first.image.pixels);
        assert!(
            playback
                .frames
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .is_err()
        );
        assert!(matches!(
            playback.frames.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn full_queue_cancellation_releases_worker_and_reopening_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel.gif");
        gif(&path, 4, 32);
        let decoded = super::super::decode(&path, 1024, &|| true).unwrap();
        let animation = decoded.animation.unwrap();
        for _ in 0..3 {
            let playback = animation.source.start(egui::Context::default()).unwrap();
            assert_eq!(receive(&playback).image.pixels, decoded.image.pixels);
            // Leave the consumer idle while the bounded producer fills its slot.
            thread::sleep(Duration::from_millis(30));
            let live = Arc::clone(&playback.live);
            drop(playback);
            assert_worker_stopped(live);
        }
    }

    #[test]
    fn gif_alpha_disposal_and_timing_survive_repeat_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alpha.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path).unwrap());
        for index in 0..3 {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_fn(8, 8, |x, y| {
                        if x == index || y == index {
                            image::Rgba([255, 0, 0, 255])
                        } else {
                            image::Rgba([0, 0, 0, 0])
                        }
                    }),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100 + index * 20, 1),
                ))
                .unwrap();
        }
        drop(encoder);
        let decoded = super::super::decode(&path, 1024, &|| true).unwrap();
        let playback = decoded
            .animation
            .unwrap()
            .source
            .start(egui::Context::default())
            .unwrap();
        for position in 0..6 {
            let index = position % 3;
            let frame = receive(&playback);
            for y in 0..8 {
                for x in 0..8 {
                    assert_eq!(
                        frame.image.pixels[y * 8 + x].a(),
                        if x == index || y == index { 255 } else { 0 }
                    );
                }
            }
            assert_eq!(frame.delay, Duration::from_millis(100 + index as u64 * 20));
        }
    }

    #[test]
    fn webp_and_apng_stream_pixels_delays_and_repeated_cycles() {
        let dir = tempfile::tempdir().unwrap();
        for extension in ["webp", "apng"] {
            let path = dir.path().join(format!("animation.{extension}"));
            if extension == "webp" {
                super::super::animation_fixtures::webp(&path);
            } else {
                super::super::animation_fixtures::apng(&path);
            }
            let first = super::super::decode(&path, 1024, &|| true).unwrap();
            assert_eq!(first.image.pixels[0], egui::Color32::RED);
            let animation = first.animation.unwrap();
            assert_eq!(animation.retained_bytes(), 0);
            let playback = animation.source.start(egui::Context::default()).unwrap();
            for index in 0..6 {
                let frame = receive(&playback);
                assert_eq!(
                    frame.image.pixels[0],
                    if index % 2 == 0 {
                        egui::Color32::RED
                    } else {
                        egui::Color32::BLUE
                    }
                );
                assert_eq!(
                    frame.delay,
                    Duration::from_millis(if index % 2 == 0 { 100 } else { 200 })
                );
            }
            assert!(
                super::super::decode(&path, 128, &|| true)
                    .unwrap()
                    .animation
                    .is_none()
            );
        }
    }

    #[test]
    fn avif_over_previous_frame_budget_streams_to_end_and_repeats() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.avif");
        std::fs::write(
            &path,
            super::super::animation_fixtures::avif_bytes(498, 286, 64),
        )
        .unwrap();
        // The decoded sequence exceeds the former 32 MiB frame budget.
        let first = super::super::decode(&path, 1024, &|| true).unwrap();
        let animation = first.animation.unwrap();
        let playback = animation.source.start(egui::Context::default()).unwrap();
        for index in 0..65 {
            let frame = receive(&playback);
            assert_eq!(frame.image.size, [498, 286]);
            assert_eq!(frame.delay, Duration::from_millis(140));
            if index == 0 || index == 64 {
                assert_eq!(frame.image.pixels, first.image.pixels);
            }
        }
    }

    #[test]
    #[ignore = "Set NABI_TEST_GIF to a local GIF to verify every frame without retaining them"]
    fn supplied_gif_streams_every_frame() {
        let path = PathBuf::from(std::env::var_os("NABI_TEST_GIF").expect("NABI_TEST_GIF"));
        let first = super::super::decode(&path, 2048, &|| true).unwrap();
        let source = first.animation.unwrap().source;
        let mut bytes = 0;
        let mut delay = Duration::ZERO;
        let count = visit(&source, &|| true, &mut |frame, size| {
            assert_eq!(size, first.original_size);
            assert_eq!(frame.image.size, first.image.size);
            bytes += frame.image.pixels.len() * 4;
            delay += frame.delay;
            Ok(true)
        })
        .unwrap();
        println!(
            "{count} frames, {:?}, {bytes} total decoded bytes, {delay:?}; bounded streaming verified",
            first.original_size
        );
    }
}
