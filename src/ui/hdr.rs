//! FP16 native textures for real HDR swapchains, with an SDR fallback selected
//! when the current display cannot present extended luminance.
use crate::core::media::color::HdrImage;
use eframe::{egui, egui_wgpu::RenderState, wgpu};
use std::sync::Mutex;

pub(crate) struct Texture {
    id: egui::TextureId,
    bytes: usize,
    state: RenderState,
    texture: wgpu::Texture,
    source: HdrImage,
    range: Mutex<(f32, f32)>,
}
impl Texture {
    pub fn new(state: &RenderState, image: HdrImage) -> Option<Self> {
        if state.target_format != wgpu::TextureFormat::Rgba16Float
            || state.target_color_space != wgpu::SurfaceColorSpace::ExtendedSrgbLinear
        {
            return None;
        }
        let (headroom, scale) = display_range(state);
        let [w, h] = image.size;
        if w == 0
            || h == 0
            || image.rgba.len() != w.checked_mul(h)?.checked_mul(4)?
            || w > state.device.limits().max_texture_dimension_2d as usize
            || h > state.device.limits().max_texture_dimension_2d as usize
        {
            return None;
        }
        let extent = wgpu::Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        };
        let texture = state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("HDR linear sRGB"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        upload_half_texture(&state.queue, &texture, &image, headroom, scale);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let id = state.renderer.write().register_native_linear_texture(
            &state.device,
            &view,
            wgpu::FilterMode::Linear,
        );
        Some(Self {
            id,
            bytes: w * h * 24,
            state: state.clone(),
            texture,
            source: image,
            range: Mutex::new((headroom, scale)),
        })
    }
    /// Refresh retained linear pixels when the window moves between SDR/HDR
    /// displays or available EDR headroom changes. Call before selecting id().
    pub fn active(&self) -> bool {
        let current = display_range(&self.state);
        if current.0 <= 1. {
            return false;
        }
        let mut range = self.range.lock().unwrap();
        if (range.0 - current.0).abs() > 0.01 || (range.1 - current.1).abs() > 0.01 {
            upload_half_texture(
                &self.state.queue,
                &self.texture,
                &self.source,
                current.0,
                current.1,
            );
            *range = current;
        }
        true
    }
    pub fn id(&self) -> egui::TextureId {
        self.id
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}
impl Drop for Texture {
    fn drop(&mut self) {
        self.state.renderer.write().free_texture(&self.id);
    }
}

// Bound the application's conversion scratch space. Queue::write_texture may
// retain its own staging copies until submission; those are separate from this
// reused row buffer and are not a promise of one-MiB total driver memory.
const HALF_UPLOAD_BYTES: usize = 1024 * 1024;
fn upload_half_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    image: &HdrImage,
    headroom: f32,
    scale: f32,
) {
    for_half_chunks(image, headroom, scale, |y, rows, pixels| {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.size[0] as u32 * 8),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width: image.size[0] as u32,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
    });
}
fn for_half_chunks(
    image: &HdrImage,
    headroom: f32,
    scale: f32,
    mut emit: impl FnMut(u32, u32, &[u8]),
) {
    let width = image.size[0];
    // Current surfaces/region limits make one row at most 64 KiB. Keep a full
    // row for future larger devices rather than creating zero-row chunks.
    let rows = (HALF_UPLOAD_BYTES / (width * 8)).max(1);
    let components = width * rows * 4;
    let mut pixels = Vec::with_capacity((image.rgba.len().min(components)) * 2);
    for (index, chunk) in image.rgba.chunks(components).enumerate() {
        pixels.clear();
        half_pixels_into(chunk, headroom, scale, &mut pixels);
        emit(
            (index * rows) as u32,
            (chunk.len() / (width * 4)) as u32,
            &pixels,
        );
    }
}
#[cfg(test)]
fn half_pixels(image: &HdrImage, headroom: f32, scale: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity(image.rgba.len() * 2);
    half_pixels_into(&image.rgba, headroom, scale, &mut out);
    out
}
fn half_pixels_into(rgba: &[f32], headroom: f32, scale: f32, out: &mut Vec<u8>) {
    for p in rgba.chunks_exact(4) {
        let y = (p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722).max(0.);
        // A smooth shoulder preserves SDR diffuse white and compresses only HDR
        // highlights that exceed this screen's available extended headroom.
        let gain = if y > 1. {
            (1. + (headroom - 1.) * (1. - (-(y - 1.) / (headroom - 1.)).exp())) / y
        } else {
            1.
        };
        for c in &p[..3] {
            let v = (*c * gain * scale * p[3]).clamp(0., 65504.);
            out.extend_from_slice(&half::f16::from_f32(v).to_le_bytes());
        }
        out.extend_from_slice(&half::f16::from_f32(p[3]).to_le_bytes());
    }
}

/// Update display reference white even when the current picture is SDR.
/// Call on the UI thread before egui paints the frame.
pub(crate) fn configure_display(state: &RenderState) {
    let _ = display_range(state);
}

fn display_range(state: &RenderState) -> (f32, f32) {
    use std::{
        collections::HashMap,
        sync::OnceLock,
        time::{Duration, Instant},
    };
    // The render state belongs to the window. Never reuse another window's monitor result.
    static RANGES: OnceLock<Mutex<HashMap<usize, (Instant, (f32, f32, f32))>>> = OnceLock::new();
    let key = std::sync::Arc::as_ptr(&state.native_window) as usize;
    let mut ranges = RANGES.get_or_init(Default::default).lock().unwrap();
    if ranges.len() > 64 {
        ranges.clear();
    }
    let cached = ranges
        .entry(key)
        .or_insert((Instant::now() - Duration::from_secs(1), (1., 1., 1.)));
    if cached.0.elapsed() > Duration::from_millis(500) {
        *cached = (Instant::now(), query_display_range(state));
    }
    let (headroom, content_scale, white) = cached.1;
    state.renderer.write().set_output_white_scale(
        if state.target_format == wgpu::TextureFormat::Rgba16Float
            && state.target_color_space == wgpu::SurfaceColorSpace::ExtendedSrgbLinear
        {
            white
        } else {
            1.
        },
    );
    (headroom, content_scale)
}

/// (headroom for 203-nit-normalized content, texture scale before shader,
/// output white scale in the final linear framebuffer).
fn query_display_range(state: &RenderState) -> (f32, f32, f32) {
    if state.target_format != wgpu::TextureFormat::Rgba16Float
        || state.target_color_space != wgpu::SurfaceColorSpace::ExtendedSrgbLinear
    {
        return (1., 1., 1.);
    }
    let window = state
        .native_window
        .read()
        .as_ref()
        .and_then(std::sync::Weak::upgrade);
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        (
            eframe::egui_wgpu::native_hdr::headroom(window.as_deref()),
            1.,
            1.,
        )
    }
    #[cfg(target_os = "windows")]
    {
        use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
        let handle = window
            .as_ref()
            .and_then(|w| w.window_handle().ok())
            .and_then(|h| match h.as_raw() {
                RawWindowHandle::Win32(h) => Some(windows::Win32::Foundation::HWND(
                    h.hwnd.get() as *mut std::ffi::c_void
                )),
                _ => None,
            });
        let range = handle
            .and_then(super::hdr_windows::display_range_for_window)
            .or_else(super::hdr_windows::display_range);
        range.map_or((1., 1., 1.), |(headroom, white)| {
            (headroom, (203. / 80.) / white, white)
        })
    }
    #[cfg(target_os = "linux")]
    {
        use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
        let is_wayland = window
            .as_ref()
            .and_then(|w| w.window_handle().ok())
            .is_some_and(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)));
        let range = is_wayland
            .then(|| window.as_ref()?.current_monitor()?.name())
            .flatten()
            .and_then(|name| super::hdr_wayland::display_range(&name));
        range.map_or((1., 1., 1.), |(headroom, white)| {
            (headroom, (203. / 80.) / white, white)
        })
    }
    #[cfg(target_os = "android")]
    {
        let _ = window;
        let Some(display) = crate::platform::mobile::hdr_display() else {
            return (1., 1., 1.);
        };
        // Android exposes live ratio, but no public API gives current SDR white
        // in nits. Never derive it by dividing static desiredMax by live ratio.
        // PQ content keeps the explicit 203-nit reference; absent ratio uses a
        // documented nominal-white fallback only for an HDR-capable display.
        let headroom = display
            .hdr_sdr_ratio
            .or_else(|| display.desired_max_nits.map(|nits| nits / 203.))
            .unwrap_or(1.);
        if headroom <= 1. {
            (1., 1., 1.)
        } else {
            (headroom, 1., 203. / 80.)
        }
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "windows",
        target_os = "linux",
        target_os = "android"
    )))]
    {
        let _ = window;
        (1., 1., 1.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hdr_shoulder_is_independent_of_crop_contents() {
        let pixel = vec![2., 1., 0.5, 1.];
        let mut full = pixel.clone();
        full.extend_from_slice(&[100., 100., 100., 1.]);
        assert_eq!(
            half_pixels(
                &HdrImage {
                    size: [1, 1],
                    rgba: pixel
                },
                4.,
                1.
            ),
            half_pixels(
                &HdrImage {
                    size: [2, 1],
                    rgba: full
                },
                4.,
                1.
            )[..8]
        );
    }
    #[test]
    #[ignore = "requires a native GPU adapter; exercises the actual egui HDR shader"]
    fn gpu_renderer_keeps_values_above_sdr_white() {
        fn block_on<F: std::future::Future>(future: F) -> F::Output {
            struct Wake(std::thread::Thread);
            impl std::task::Wake for Wake {
                fn wake(self: std::sync::Arc<Self>) {
                    self.0.unpark();
                }
            }
            let waker = std::task::Waker::from(std::sync::Arc::new(Wake(std::thread::current())));
            let mut cx = std::task::Context::from_waker(&waker);
            let mut future = std::pin::pin!(future);
            loop {
                match future.as_mut().poll(&mut cx) {
                    std::task::Poll::Ready(v) => return v,
                    std::task::Poll::Pending => {
                        std::thread::park_timeout(std::time::Duration::from_millis(10))
                    }
                }
            }
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("GPU adapter required for explicit HDR smoke test");
        let (device, queue) =
            block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut renderer = eframe::egui_wgpu::Renderer::new(
            &device,
            wgpu::TextureFormat::Rgba16Float,
            eframe::egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        );
        // Also exercise the OS reference-white uniform on the real shader.
        renderer.set_output_white_scale(2.);
        let extent = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = |label, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage,
                view_formats: &[],
            })
        };
        let fade = 128. / 255.;
        // Cover both linear native pixels and ordinary egui gamma-premultiplied
        // textures. Alpha/fade must be applied in linear space on FP16 surfaces.
        for (name, hdr_alpha, tint, expected) in [
            (
                "opaque HDR",
                Some(1.),
                egui::Color32::WHITE,
                [8., 4., 2., 1.],
            ),
            (
                "transparent HDR",
                Some(0.5),
                egui::Color32::WHITE,
                [4., 2., 1., 0.5],
            ),
            (
                "faded HDR",
                Some(0.5),
                egui::Color32::from_white_alpha(128),
                [4. * fade, 2. * fade, fade, 0.5 * fade],
            ),
            (
                "transparent SDR",
                None,
                egui::Color32::WHITE,
                [2. * fade, 2. * fade, 2. * fade, fade],
            ),
            (
                "faded SDR",
                None,
                egui::Color32::from_white_alpha(128),
                [
                    2. * fade * fade,
                    2. * fade * fade,
                    2. * fade * fade,
                    fade * fade,
                ],
            ),
        ] {
            let source = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(name),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: if hdr_alpha.is_some() {
                    wgpu::TextureFormat::Rgba16Float
                } else {
                    wgpu::TextureFormat::Rgba8Unorm
                },
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            if let Some(alpha) = hdr_alpha {
                // Same bounded upload as new()/active(); ample headroom leaves the
                // reference pixel within FP16 roundoff of 4/2/1 before alpha.
                upload_half_texture(
                    &queue,
                    &source,
                    &HdrImage {
                        size: [1, 1],
                        rgba: vec![4., 2., 1., alpha],
                    },
                    10_000.,
                    1.,
                );
            } else {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &source,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &egui::Color32::from_white_alpha(128).to_array(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    extent,
                );
            }
            let view = source.create_view(&Default::default());
            let id = if hdr_alpha.is_some() {
                renderer.register_native_linear_texture(&device, &view, wgpu::FilterMode::Nearest)
            } else {
                renderer.register_native_texture(&device, &view, wgpu::FilterMode::Nearest)
            };
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(1.));
            let mut mesh = egui::Mesh::with_texture(id);
            mesh.add_rect_with_uv(rect, rect, tint);
            let jobs = vec![egui::ClippedPrimitive {
                clip_rect: rect,
                primitive: egui::epaint::Primitive::Mesh(mesh),
            }];
            let screen = eframe::egui_wgpu::ScreenDescriptor {
                size_in_pixels: [1, 1],
                pixels_per_point: 1.,
            };
            let output = texture(
                "HDR test target",
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let target = output.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            renderer.update_buffers(&device, &queue, &mut encoder, &jobs, &screen);
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
            }
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 256,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &output,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                extent,
            );
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                send.send(r).unwrap();
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receive.recv().unwrap().unwrap();
            let mapped = readback.slice(..).get_mapped_range().unwrap();
            let values: Vec<_> = mapped[..8]
                .chunks_exact(2)
                .map(|p| half::f16::from_le_bytes([p[0], p[1]]).to_f32())
                .collect();
            for (actual, expected) in values.iter().zip(expected) {
                assert!((actual - expected).abs() < 0.02, "{name}: {values:?}");
            }
            drop(mapped);
            readback.unmap();
            renderer.free_texture(&id);
        }
    }

    #[test]
    fn half_upload_reuses_bounded_rows_without_seams() {
        let width = 8192;
        let height = 33;
        let rgba = (0..width * height)
            .flat_map(|n| [2., (n % 257) as f32 / 256., 0.5, 0.75])
            .collect();
        let image = HdrImage {
            size: [width, height],
            rgba,
        };
        let expected = half_pixels(&image, 4., 1.25);
        let mut next_row = 0;
        let mut calls = 0;
        for_half_chunks(&image, 4., 1.25, |y, rows, pixels| {
            assert_eq!(y as usize, next_row);
            assert!(pixels.len() <= HALF_UPLOAD_BYTES);
            assert_eq!(pixels.len(), width * rows as usize * 8);
            let start = y as usize * width * 8;
            assert_eq!(pixels, &expected[start..start + pixels.len()]);
            next_row += rows as usize;
            calls += 1;
        });
        assert_eq!(next_row, height);
        assert_eq!(calls, 3);
    }

    #[test]
    fn fp16_texture_preserves_extended_values_and_premultiplies_alpha() {
        let image = HdrImage {
            size: [1, 1],
            rgba: vec![4., 0., 0., 0.5],
        };
        let bytes = half_pixels(&image, 8., 1.);
        let pixels: Vec<_> = bytes
            .chunks_exact(2)
            .map(|p| half::f16::from_le_bytes([p[0], p[1]]).to_f32())
            .collect();
        assert_eq!(pixels, vec![2., 0., 0., 0.5]);
    }
    #[test]
    fn display_shoulder_bounds_bright_highlights() {
        let image = HdrImage {
            size: [1, 1],
            rgba: vec![50., 50., 50., 1.],
        };
        let bytes = half_pixels(&image, 4., 1.);
        let white = half::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32();
        assert!(white > 1. && white <= 4.);
    }
}
