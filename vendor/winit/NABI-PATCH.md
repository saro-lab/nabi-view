# Nabi View changes to winit 0.30.13

Upstream release: https://crates.io/crates/winit/0.30.13
Upstream source: https://github.com/rust-windowing/winit/tree/v0.30.13
Crate SHA-256: `a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d`.
Original Apache-2.0 license, notices and release metadata are retained.

On Android, exit the event loop on `MainEvent::Destroy`, including when paused.
`NativeActivity.onDestroy` waits for `android_main` to return; ignoring Destroy
leaves Android's UI thread blocked indefinitely. Normal `LoopExiting` dispatch
lets eframe save and release resources before the native thread returns.

Permit another event loop only after the old Android event loop is dropped.
Android may create a new NativeActivity within the same process. Other native
platforms keep their existing single-event-loop policy.

On iOS, bind newly created windows to the foreground `UIWindowScene` when the
host has adopted UIKit's scene lifecycle. Use the scene coordinate space for
the initial view bounds and enumerate scene-owned windows for occlusion and
termination events. Legacy hosts keep their original window path. The existing
UIApplication aggregate notifications remain the event-loop lifecycle source,
so scene delegates do not dispatch duplicate resumed/suspended events. This
patch supports Nabi View's single-scene host, not independent multi-scene loops.

Report the full iOS content view bounds as the inner drawable size and position.
Safe-area insets remain a UI layout concern in the host. Subtracting those
insets from the drawable stretched Metal output across the full view while
touches retained full-view coordinates, misaligning controls in both portrait
and landscape. This change is confined to the iOS backend.

Modified files: `src/platform_impl/android/mod.rs`, `src/event_loop.rs`,
`src/platform_impl/ios/window.rs`, `src/platform_impl/ios/app_state.rs`,
`Cargo.toml` (UIKit scene features).
Application host cleanup and in-place locale/RTL configuration handling live in
Nabi View, outside this crate. The iOS scene manifest and delegate also live in
the application host. Technical review: 2026-11-15.

UIKit migration reference:
https://developer.apple.com/documentation/uikit/transitioning-to-the-uikit-scene-based-life-cycle

On macOS, make background blur unsupported (a no-op) and remove the private
WindowServer FFI declarations and their now-unused imports. Upstream 0.30.13
links those APIs even when the application never enables blur. Nabi View uses
an opaque window and does not request this effect. No runtime symbol lookup or
hidden private API fallback is used. Modified files:
`src/platform_impl/macos/ffi.rs`, `src/platform_impl/macos/window_delegate.rs`.
Technical review: 2026-11-18. App Review Guideline 2.5.1:
https://developer.apple.com/app-store/review/guidelines/#software-requirements
