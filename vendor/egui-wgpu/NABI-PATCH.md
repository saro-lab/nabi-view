# Nabi View changes to egui-wgpu 0.37.0

Upstream release: https://crates.io/crates/egui-wgpu/0.37.0
Upstream source: https://github.com/emilk/egui/tree/0.37.0/crates/egui-wgpu
Crate SHA-256: `0309cbe1febed48af5b974e4bcc424b35e9664863a4d402697147b57a488c864`.
Original MIT OR Apache-2.0 licenses and release metadata are retained.

Prefer RGBA16Float only when the surface advertises extended-linear sRGB for
that format. Configure the explicit wgpu 30 SurfaceColorSpace rather than
depending on a legacy format-only convention. wgpu now configures the Apple
Metal layer's EDR flag and color space itself; the application no longer invokes
the legacy layer setter during window creation. The guarded setter and nested
CALayer/CAMetalLayer regression remain to preserve the startup-crash test.

Keep a weak native-window reference for per-window Apple EDR headroom queries,
including iOS UIWindowScene screens. Add an output-white-scale uniform, select
linear shader output for FP16 surfaces, and disable eight-bit dithering there.

The 0.37 upstream shader samples ordinary textures as gamma-encoded pixels.
Add register_native_linear_texture and a texture flag so application-owned
linear FP16 HDR samples retain their transfer function. Other texture APIs keep
their upstream gamma semantics, including predictable texture filtering.
Transfer functions unpremultiply and then premultiply alpha so translucent SDR
textures, HDR pixels, and vertex fade retain correct brightness on linear output.

Support eight-byte FP16 capture rows and convert them into the existing SDR
screenshot representation, converting linear premultiplied alpha into the current
gamma-premultiplied Color32 representation. Native float pixel ownership, bounded row uploads,
display-specific tone mapping, and SDR fallback remain application code.

Validation and technical investigation records are kept in private ai records.
