//! Visible-area decoding. One worker, two replaceable slots, bounded outputs.
//! Non-interlaced PNG is scanned a row at a time; other raster decoders retain
//! their existing input/allocation limits. A crop is not a promise of random I/O.
use super::{CancelReader, Frame, MAX_DECODE_BYTES, MAX_EDGE, MAX_FILE_BYTES, MAX_PIXELS, color};
use eframe::egui;
use image::{
    DynamicImage, GenericImageView, ImageDecoder, ImageReader, Limits, metadata::Orientation,
};
use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const MAX_REGION_SIDE: u32 = 8192;
pub const MAX_REGION_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_SCAN_PIXELS: u64 = 1_000_000_000;
const MAX_SCAN_EDGE: u32 = 262_144;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl Area {
    pub fn full(size: [u32; 2]) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size[0],
            height: size[1],
        }
    }
    fn valid(self, size: [u32; 2]) -> bool {
        self.width > 0
            && self.height > 0
            && self.x.checked_add(self.width).is_some_and(|x| x <= size[0])
            && self
                .y
                .checked_add(self.height)
                .is_some_and(|y| y <= size[1])
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub path: PathBuf,
    pub area: Area,
    pub max_side: u32,
}
impl Request {
    pub fn reserved_bytes(&self) -> usize {
        let [width, height] = output(self.area, self.max_side);
        width as usize * height as usize * 28
    }
}
pub type Pixels = (egui::ColorImage, Option<color::HdrImage>);
pub struct ResultImage {
    pub request: Request,
    pub result: Result<Pixels, String>,
}
#[derive(Default)]
struct State {
    pending: [Option<(u64, Request)>; 2],
    ready: [Option<ResultImage>; 2],
    closed: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    ids: [AtomicU64; 2],
}
pub struct Worker(Arc<Shared>);
impl Worker {
    pub fn new(ctx: egui::Context) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            ids: Default::default(),
        });
        let work = shared.clone();
        thread::spawn(move || {
            loop {
                let (slot, id, request) = {
                    let mut state = work.state.lock().unwrap();
                    loop {
                        if state.closed {
                            return;
                        }
                        if let Some(slot) = state.pending.iter().position(Option::is_some) {
                            let (id, request) = state.pending[slot].take().unwrap();
                            break (slot, id, request);
                        }
                        state = work.changed.wait(state).unwrap();
                    }
                };
                let live = || work.ids[slot].load(Ordering::Relaxed) == id;
                let result = decode(&request.path, request.area, request.max_side, &live);
                let mut state = work.state.lock().unwrap();
                if live() && !state.closed {
                    state.ready[slot] = Some(ResultImage { request, result });
                    ctx.request_repaint();
                }
            }
        });
        Self(shared)
    }
    pub fn request(&self, slot: usize, request: Request) {
        let mut state = self.0.state.lock().unwrap();
        let id = self.0.ids[slot].fetch_add(1, Ordering::Relaxed) + 1;
        state.pending[slot] = Some((id, request));
        state.ready[slot] = None;
        self.0.changed.notify_one();
    }
    pub fn clear(&self, slot: usize) {
        let mut state = self.0.state.lock().unwrap();
        self.0.ids[slot].fetch_add(1, Ordering::Relaxed);
        state.pending[slot] = None;
        state.ready[slot] = None;
    }
    pub fn take(&self, slot: usize) -> Option<ResultImage> {
        self.0.state.lock().unwrap().ready[slot].take()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        state.closed = true;
        for id in &self.0.ids {
            id.fetch_add(1, Ordering::Relaxed);
        }
        self.0.changed.notify_one();
    }
}

pub(super) fn output(area: Area, max_side: u32) -> [u32; 2] {
    let scale = (max_side.clamp(1, MAX_REGION_SIDE) as f64 / area.width.max(area.height) as f64)
        .min(1.)
        .min((MAX_REGION_PIXELS as f64 / (area.width as f64 * area.height as f64)).sqrt());
    [
        (area.width as f64 * scale).floor().max(1.) as u32,
        (area.height as f64 * scale).floor().max(1.) as u32,
    ]
}
fn oriented_size(size: [u32; 2], orientation: Orientation) -> [u32; 2] {
    match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => [size[1], size[0]],
        _ => size,
    }
}
fn raw_area(a: Area, size: [u32; 2], o: Orientation) -> Area {
    let [w, h] = size;
    match o {
        Orientation::NoTransforms => a,
        Orientation::FlipHorizontal => Area {
            x: w - a.x - a.width,
            ..a
        },
        Orientation::FlipVertical => Area {
            y: h - a.y - a.height,
            ..a
        },
        Orientation::Rotate180 => Area {
            x: w - a.x - a.width,
            y: h - a.y - a.height,
            ..a
        },
        Orientation::Rotate90 => Area {
            x: a.y,
            y: h - a.x - a.width,
            width: a.height,
            height: a.width,
        },
        Orientation::Rotate270 => Area {
            x: w - a.y - a.height,
            y: a.x,
            width: a.height,
            height: a.width,
        },
        Orientation::Rotate90FlipH => Area {
            x: a.y,
            y: a.x,
            width: a.height,
            height: a.width,
        },
        Orientation::Rotate270FlipH => Area {
            x: w - a.y - a.height,
            y: h - a.x - a.width,
            width: a.height,
            height: a.width,
        },
    }
}
fn check(live: &dyn Fn() -> bool, start: Instant) -> Result<(), String> {
    if !live() {
        Err("Cancelled".into())
    } else if start.elapsed() > Duration::from_secs(10) {
        Err("Visible region exceeds decode time limit".into())
    } else {
        Ok(())
    }
}

pub(super) fn png_preview(
    path: &Path,
    max_side: u32,
    live: &dyn Fn() -> bool,
) -> Result<Option<Frame>, String> {
    match scan_png(path, None, max_side, live)? {
        Some((image, original_size)) => {
            let (image, hdr) = color::display(image);
            Ok(Some(Frame {
                hdr,
                animation: None,
                image,
                original_size,
            }))
        }
        None => Ok(None),
    }
}
fn decode(
    path: &Path,
    area: Area,
    max_side: u32,
    live: &dyn Fn() -> bool,
) -> Result<Pixels, String> {
    #[cfg(all(target_os = "macos", not(test)))]
    let access = crate::platform::mac_access::resolve(path)?;
    #[cfg(all(target_os = "macos", not(test)))]
    let path = access.path.as_path();
    let started = Instant::now();
    check(live, started)?;
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err("Image input exceeds region limit".into());
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        if let Some((image, _)) = scan_png(path, Some(area), max_side, live)? {
            return Ok(color::display(image));
        }
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        return svg_region(path, area, max_side, live).map(|image| (image, None));
    }
    let extension = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension == "avif" {
        return super::avif::decode_region(path, area, max_side, live);
    }
    if matches!(
        extension.as_str(),
        "heic" | "heif" | "dng" | "cr2" | "cr3" | "nef" | "arw" | "raf"
    ) {
        let image = crate::platform::platform_image::decode(path)?;
        if !area.valid([image.width(), image.height()]) {
            return Err("The source dimensions changed".into());
        }
        let image = image.crop_imm(area.x, area.y, area.width, area.height);
        let [w, h] = output(area, max_side);
        check(live, started)?;
        return Ok(color::display(image.thumbnail_exact(w, h)));
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = ImageReader::new(BufReader::new(CancelReader { file, live }))
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
    reader.limits(limits);
    let mut decoder: Box<dyn ImageDecoder + '_> = if extension == "jxl" {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mut decoder =
            jxl_oxide::integration::JxlDecoder::new(BufReader::new(CancelReader { file, live }))
                .map_err(|e| e.to_string())?;
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_EDGE);
        limits.max_image_height = Some(MAX_EDGE);
        limits.max_alloc = Some(MAX_DECODE_BYTES);
        decoder.set_limits(limits).map_err(|e| e.to_string())?;
        let icc = decoder.icc_profile().ok().flatten();
        let (w, h) = decoder.dimensions();
        if !area.valid([w, h]) {
            return Err("The source dimensions changed".into());
        }
        let bytes = u64::from(area.width)
            .checked_mul(u64::from(area.height))
            .and_then(|p| p.checked_mul(u64::from(decoder.color_type().bytes_per_pixel())));
        if bytes.is_none_or(|bytes| bytes > MAX_DECODE_BYTES) {
            return Err("JPEG XL region exceeds allocation limit".into());
        }
        let image =
            DynamicImage::from_decoder(JxlRegion { decoder, area }).map_err(|e| e.to_string())?;
        let [w, h] = output(area, max_side);
        check(live, started)?;
        let image = if image.dimensions() == (w, h) {
            image
        } else {
            image.thumbnail_exact(w, h)
        };
        return Ok(color::display(color::to_srgb(image, icc.as_deref())?));
    } else {
        Box::new(reader.into_decoder().map_err(|e| e.to_string())?)
    };
    let (w, h) = decoder.dimensions();
    if w == 0
        || h == 0
        || u64::from(w) * u64::from(h) > MAX_PIXELS
        || decoder.total_bytes() > MAX_DECODE_BYTES
    {
        return Err("This decoder cannot read the large image within its memory limit".into());
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    if !area.valid(oriented_size([w, h], orientation)) {
        return Err("The source dimensions changed; reopen the image".into());
    }
    let icc = decoder.icc_profile().ok().flatten();
    let image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    check(live, started)?;
    let raw = raw_area(area, [w, h], orientation);
    let mut image = image.crop_imm(raw.x, raw.y, raw.width, raw.height);
    image.apply_orientation(orientation);
    let [out_w, out_h] = output(area, max_side);
    if image.dimensions() != (out_w, out_h) {
        image = image.thumbnail_exact(out_w, out_h);
    }
    check(live, started)?;
    Ok(color::display(color::to_srgb(image, icc.as_deref())?))
}

struct JxlRegion<R: Read> {
    decoder: jxl_oxide::integration::JxlDecoder<R>,
    area: Area,
}
impl<R: Read> ImageDecoder for JxlRegion<R> {
    fn dimensions(&self) -> (u32, u32) {
        (self.area.width, self.area.height)
    }
    fn color_type(&self) -> image::ColorType {
        self.decoder.color_type()
    }
    fn read_image_boxed(self: Box<Self>, buf: &mut [u8]) -> image::ImageResult<()> {
        (*self).read_image(buf)
    }
    fn read_image(mut self, buf: &mut [u8]) -> image::ImageResult<()> {
        let stride = self.area.width as usize * self.color_type().bytes_per_pixel() as usize;
        image::ImageDecoderRect::read_rect(
            &mut self.decoder,
            self.area.x,
            self.area.y,
            self.area.width,
            self.area.height,
            buf,
            stride,
        )
    }
}

fn svg_region(
    path: &Path,
    area: Area,
    max_side: u32,
    live: &dyn Fn() -> bool,
) -> Result<egui::ColorImage, String> {
    let mut options = resvg::usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    options.fontdb_mut().load_system_fonts();
    let tree =
        resvg::usvg::Tree::from_data(&std::fs::read(path).map_err(|e| e.to_string())?, &options)
            .map_err(|e| e.to_string())?;
    let size = tree.size().to_int_size();
    if !area.valid([size.width(), size.height()]) {
        return Err("SVG dimensions changed".into());
    }
    let [w, h] = output(area, max_side);
    let mut pixels = resvg::tiny_skia::Pixmap::new(w, h).ok_or("Region allocation failed")?;
    let sx = w as f32 / area.width as f32;
    let sy = h as f32 / area.height as f32;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_row(
            sx,
            0.,
            0.,
            sy,
            -(area.x as f32) * sx,
            -(area.y as f32) * sy,
        ),
        &mut pixels.as_mut(),
    );
    if !live() {
        return Err("Cancelled".into());
    }
    Ok(egui::ColorImage::from_rgba_premultiplied(
        [w as usize, h as usize],
        pixels.data(),
    ))
}

/// Returns None for interlaced PNG/APNG, whose existing bounded decoder is used.
/// Stores one PNG scanline, one output row accumulator, and the final display crop.
fn scan_png(
    path: &Path,
    area: Option<Area>,
    max_side: u32,
    live: &dyn Fn() -> bool,
) -> Result<Option<(DynamicImage, [u32; 2])>, String> {
    let started = Instant::now();
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut signature = [0; 8];
    if file.read_exact(&mut signature).is_err() || signature != *b"\x89PNG\r\n\x1a\n" {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let region_request = area.is_some();
    let mut decoder = png::Decoder::new_with_limits(
        BufReader::new(CancelReader { file, live }),
        png::Limits {
            bytes: 32 * 1024 * 1024,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    if info.interlaced || info.animation_control.is_some() {
        return Ok(None);
    }
    let size = [info.width, info.height];
    if size.contains(&0)
        || size.iter().any(|x| *x > MAX_SCAN_EDGE)
        || u64::from(size[0]) * u64::from(size[1]) > MAX_SCAN_PIXELS
    {
        return Err("PNG exceeds bounded scan dimensions".into());
    }
    let orientation = info
        .exif_metadata
        .as_deref()
        .and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms);
    let original = oriented_size(size, orientation);
    let area = area.unwrap_or_else(|| Area::full(original));
    if !area.valid(original) {
        return Err("The PNG dimensions changed; reopen the image".into());
    }
    let raw = raw_area(area, size, orientation);
    let [w, h] = if region_request {
        output(raw, max_side)
    } else {
        super::output_size(raw.width, raw.height, max_side)
    };
    let cicp = info.coding_independent_code_points;
    let profile = info
        .icc_profile
        .as_deref()
        .filter(|bytes| bytes.len() <= color::MAX_PROFILE_BYTES)
        .map(<[u8]>::to_vec);
    let (color_type, depth) = reader.output_color_type();
    let sixteen = depth == png::BitDepth::Sixteen;
    if !sixteen && depth != png::BitDepth::Eight {
        return Err("Unsupported PNG output depth".into());
    }
    let maximum = if sixteen { 65535 } else { 255 };
    let stride = color_type.samples() * if sixteen { 2 } else { 1 };
    // Compute bucket boundaries once, not a division for every source pixel.
    let spans: Vec<_> = (0..w)
        .map(|x| {
            let start = (u64::from(x) * u64::from(raw.width)).div_ceil(u64::from(w)) as usize;
            let end = (u64::from(x + 1) * u64::from(raw.width)).div_ceil(u64::from(w)) as usize;
            (
                (raw.x as usize + start) * stride,
                (raw.x as usize + end) * stride,
            )
        })
        .collect();
    let mut pixels = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::new(w, h);
    let mut sums = vec![[0u64; 4]; w as usize];
    let mut counts = vec![0u64; w as usize];
    let mut current = 0;
    let flush = |row: u32,
                 pixels: &mut image::ImageBuffer<image::Rgba<u16>, Vec<u16>>,
                 sums: &mut [[u64; 4]],
                 counts: &mut [u64]| {
        for x in 0..w as usize {
            let sum = sums[x];
            let count = counts[x].max(1);
            let alpha = (sum[3] + count / 2) / count;
            let channel = |n: usize| {
                if sum[3] > 0 {
                    ((sum[n] + sum[3] / 2) / sum[3]).min(maximum) as u16
                } else {
                    0
                }
            };
            pixels.put_pixel(
                x as u32,
                row,
                image::Rgba([
                    channel(0),
                    channel(1),
                    channel(2),
                    alpha.min(maximum) as u16,
                ]),
            );
            sums[x] = [0; 4];
            counts[x] = 0;
        }
    };
    for y in 0..raw.y + raw.height {
        check(live, started)?;
        let row = reader
            .next_row()
            .map_err(|e| e.to_string())?
            .ok_or("Incomplete PNG image")?;
        if y < raw.y {
            continue;
        }
        let out_y = (u64::from(y - raw.y) * u64::from(h) / u64::from(raw.height)) as u32;
        if out_y != current {
            flush(current, &mut pixels, &mut sums, &mut counts);
            current = out_y;
        }
        let row = row.data();
        match (sixteen, color_type) {
            (false, png::ColorType::Grayscale) => {
                accumulate::<1>(row, &spans, &mut sums, &mut counts, |p| {
                    [p[0] as u16, p[0] as u16, p[0] as u16, 255]
                })
            }
            (false, png::ColorType::GrayscaleAlpha) => {
                accumulate::<2>(row, &spans, &mut sums, &mut counts, |p| {
                    [p[0] as u16, p[0] as u16, p[0] as u16, p[1] as u16]
                })
            }
            (false, png::ColorType::Rgb) => {
                accumulate::<3>(row, &spans, &mut sums, &mut counts, |p| {
                    [p[0] as u16, p[1] as u16, p[2] as u16, 255]
                })
            }
            (false, png::ColorType::Rgba) => {
                accumulate::<4>(row, &spans, &mut sums, &mut counts, |p| {
                    [p[0] as u16, p[1] as u16, p[2] as u16, p[3] as u16]
                })
            }
            (true, png::ColorType::Grayscale) => {
                accumulate::<2>(row, &spans, &mut sums, &mut counts, |p| {
                    let v = u16::from_be_bytes([p[0], p[1]]);
                    [v, v, v, 65535]
                })
            }
            (true, png::ColorType::GrayscaleAlpha) => {
                accumulate::<4>(row, &spans, &mut sums, &mut counts, |p| {
                    let v = u16::from_be_bytes([p[0], p[1]]);
                    [v, v, v, u16::from_be_bytes([p[2], p[3]])]
                })
            }
            (true, png::ColorType::Rgb) => {
                accumulate::<6>(row, &spans, &mut sums, &mut counts, |p| {
                    [
                        u16::from_be_bytes([p[0], p[1]]),
                        u16::from_be_bytes([p[2], p[3]]),
                        u16::from_be_bytes([p[4], p[5]]),
                        65535,
                    ]
                })
            }
            (true, png::ColorType::Rgba) => {
                accumulate::<8>(row, &spans, &mut sums, &mut counts, |p| {
                    [
                        u16::from_be_bytes([p[0], p[1]]),
                        u16::from_be_bytes([p[2], p[3]]),
                        u16::from_be_bytes([p[4], p[5]]),
                        u16::from_be_bytes([p[6], p[7]]),
                    ]
                })
            }
            _ => return Err("Unexpanded PNG palette".into()),
        }
    }
    flush(current, &mut pixels, &mut sums, &mut counts);
    let image = if sixteen {
        DynamicImage::ImageRgba16(pixels)
    } else {
        DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(
                w,
                h,
                pixels.into_raw().into_iter().map(|v| v as u8).collect(),
            )
            .unwrap(),
        )
    };
    let mut image = if let Some(cicp) = cicp.filter(|c| matches!(c.transfer_function, 16 | 18)) {
        if cicp.matrix_coefficients != 0 || !cicp.is_video_full_range_image {
            return Err("Unsupported PNG HDR signal range".into());
        }
        color::cicp_hdr(
            image,
            u16::from(cicp.color_primaries),
            u16::from(cicp.transfer_function),
        )?
    } else {
        color::to_srgb(image, profile.as_deref())?
    };
    image.apply_orientation(orientation);
    Ok(Some((image, original)))
}

fn accumulate<const N: usize>(
    row: &[u8],
    spans: &[(usize, usize)],
    sums: &mut [[u64; 4]],
    counts: &mut [u64],
    parse: impl Fn(&[u8]) -> [u16; 4],
) {
    for ((&(start, end), sum), count) in spans.iter().zip(sums).zip(counts) {
        let mut local = [0u64; 4];
        for pixel in row[start..end].chunks_exact(N) {
            let rgba = parse(pixel);
            let a = u64::from(rgba[3]);
            local[0] += u64::from(rgba[0]) * a;
            local[1] += u64::from(rgba[1]) * a;
            local[2] += u64::from(rgba[2]) * a;
            local[3] += a;
        }
        for i in 0..4 {
            sum[i] += local[i];
        }
        *count += ((end - start) / N) as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_five_k_viewport_and_region_allocation_limits() {
        let viewport = Area::full([5248, 2944]);
        assert_eq!(output(viewport, MAX_REGION_SIDE), [5248, 2944]);
        for size in [[8192, 8192], [8191, 8001], [262144, 17], [17, 262144]] {
            let [w, h] = output(Area::full(size), u32::MAX);
            assert!(w.max(h) <= MAX_REGION_SIDE);
            assert!(u64::from(w) * u64::from(h) <= MAX_REGION_PIXELS);
        }
    }
    #[test]
    fn raw_crop_matches_every_oriented_full_image() {
        let original = image::RgbaImage::from_fn(13, 9, |x, y| {
            image::Rgba([x as u8, y as u8, (x + y) as u8, 255])
        });
        for orientation in [
            Orientation::NoTransforms,
            Orientation::FlipHorizontal,
            Orientation::FlipVertical,
            Orientation::Rotate90,
            Orientation::Rotate180,
            Orientation::Rotate270,
            Orientation::Rotate90FlipH,
            Orientation::Rotate270FlipH,
        ] {
            let area = Area {
                x: 2,
                y: 3,
                width: 4,
                height: 5,
            };
            let raw = raw_area(area, [13, 9], orientation);
            let mut all = DynamicImage::ImageRgba8(original.clone());
            all.apply_orientation(orientation);
            let mut crop = DynamicImage::ImageRgba8(original.clone())
                .crop_imm(raw.x, raw.y, raw.width, raw.height);
            crop.apply_orientation(orientation);
            assert_eq!(crop, all.crop_imm(area.x, area.y, area.width, area.height));
        }
    }
    #[test]
    fn png_region_preserves_native_pixels_alpha_and_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.png");
        let source = image::RgbaImage::from_fn(320, 240, |x, y| {
            image::Rgba([x as u8, y as u8, 71, if x % 2 == 0 { 128 } else { 255 }])
        });
        source.save(&path).unwrap();
        let area = Area {
            x: 133,
            y: 111,
            width: 37,
            height: 29,
        };
        let shown = decode(&path, area, 4096, &|| true).unwrap().0;
        let expected = source
            .view(area.x, area.y, area.width, area.height)
            .to_image();
        assert_eq!(
            shown,
            egui::ColorImage::from_rgba_unmultiplied([37, 29], expected.as_raw())
        );
        assert!(decode(&path, area, 4096, &|| false).is_err());
        let frame = png_preview(&path, 64, &|| true).unwrap().unwrap();
        assert_eq!(frame.original_size, [320, 240]);
        assert_eq!(frame.image.size, [64, 48]);
    }
    #[test]
    fn hdr_png_keeps_16_bit_pq_values_and_native_region() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hdr.png");
        let mut info = png::Info::with_size(8, 4);
        info.color_type = png::ColorType::Rgba;
        info.bit_depth = png::BitDepth::Sixteen;
        info.coding_independent_code_points = Some(png::CodingIndependentCodePoints {
            color_primaries: 9,
            transfer_function: 16,
            matrix_coefficients: 0,
            is_video_full_range_image: true,
        });
        let mut writer = png::Encoder::with_info(File::create(&path).unwrap(), info)
            .unwrap()
            .write_header()
            .unwrap();
        writer
            .write_chunk(png::chunk::cICP, &[9, 16, 0, 1])
            .unwrap();
        let pixel = [55000u16, 55000, 55000, 32768];
        let mut bytes = Vec::new();
        for _ in 0..32 {
            for c in pixel {
                bytes.extend(c.to_be_bytes());
            }
        }
        writer.write_image_data(&bytes).unwrap();
        writer.finish().unwrap();
        let frame = png_preview(&path, 8, &|| true).unwrap().unwrap();
        let hdr = frame.hdr.unwrap();
        assert!(hdr.rgba[0] > 1.);
        assert!((hdr.rgba[3] - 0.5).abs() < 0.001);
        let (_, region) = decode(
            &path,
            Area {
                x: 2,
                y: 1,
                width: 3,
                height: 2,
            },
            4096,
            &|| true,
        )
        .unwrap();
        let region = region.unwrap();
        assert_eq!(region.size, [3, 2]);
        assert_eq!(&region.rgba[..4], &hdr.rgba[..4]);
    }

    #[test]
    #[ignore = "Bounded-memory stress check: scans a generated 67 MP PNG"]
    fn png_above_full_decode_limit_has_bounded_preview_and_native_crop() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.png");
        let mut encoder = png::Encoder::new(File::create(&path).unwrap(), 8193, 8193);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().unwrap();
        {
            let mut stream = writer.stream_writer().unwrap();
            let row: Vec<u8> = (0..8193).flat_map(|x| [x as u8, 72, 110]).collect();
            for _ in 0..8193 {
                stream.write_all(&row).unwrap();
            }
            stream.finish().unwrap();
        }
        writer.finish().unwrap();
        let begin = Instant::now();
        let frame = super::super::decode(&path, 256, &|| true).unwrap();
        eprintln!(
            "67 MP PNG preview: {:?}, {} retained RGBA bytes",
            begin.elapsed(),
            frame.image.pixels.len() * 4
        );
        assert_eq!(frame.original_size, [8193, 8193]);
        assert_eq!(frame.image.size, [256, 256]);
        let begin = Instant::now();
        let (image, _) = decode(
            &path,
            Area {
                x: 7001,
                y: 7001,
                width: 91,
                height: 73,
            },
            4096,
            &|| true,
        )
        .unwrap();
        eprintln!(
            "67 MP PNG native crop: {:?}, {} retained RGBA bytes",
            begin.elapsed(),
            image.pixels.len() * 4
        );
        assert_eq!(image.size, [91, 73]);
        assert_eq!(
            image.pixels[0],
            egui::Color32::from_rgb(7001u32 as u8, 72, 110)
        );
    }

    #[test]
    #[ignore = "Compare the former full-buffer PNG path with row scanning on one deterministic image"]
    fn png_decode_comparison() {
        use std::io::Write;
        let mode = std::env::var("NABI_PNG_BENCH_MODE").unwrap_or_else(|_| "stream".into());
        assert!(mode == "full" || mode == "stream");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("comparison.png");
        let mut encoder = png::Encoder::new(File::create(&path).unwrap(), 6000, 4000);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().unwrap();
        {
            let mut stream = writer.stream_writer().unwrap();
            for y in 0..4000 {
                let row: Vec<u8> = (0..6000)
                    .flat_map(|x| [x as u8, y as u8, (x + y) as u8, 255])
                    .collect();
                stream.write_all(&row).unwrap();
            }
            stream.finish().unwrap();
        }
        writer.finish().unwrap();
        let mut samples = Vec::new();
        for _ in 0..50 {
            let start = Instant::now();
            let size = if mode == "full" {
                // Former PNG path: decode the full raster, then thumbnail_exact.
                let image = image::open(&path)
                    .unwrap()
                    .thumbnail_exact(2048, 1365)
                    .into_rgba8();
                std::hint::black_box(&image);
                [image.width() as usize, image.height() as usize]
            } else {
                let frame = png_preview(&path, 2048, &|| true).unwrap().unwrap();
                std::hint::black_box(&frame.image);
                frame.image.size
            };
            assert_eq!(size, [2048, 1365]);
            samples.push(start.elapsed().as_secs_f64() * 1000.);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "PNG decode {mode}: n=50 p50_ms={:.3} p95_ms={:.3}",
            samples[24], samples[47]
        );
    }
}
