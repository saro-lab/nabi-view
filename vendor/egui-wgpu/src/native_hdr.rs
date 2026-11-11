//! Nabi View's per-window Apple EDR headroom query. wgpu 30 configures the
//! surface color space itself; the guarded legacy setter remains for regression tests.
#![allow(unsafe_code)] // Audited Objective-C/CoreGraphics FFI, confined to this module.
use std::ffi::{c_char, c_void};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
type Object = *mut c_void;
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Object;
    fn sel_registerName(name: *const c_char) -> Object;
    fn objc_msgSend();
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceExtendedLinearSRGB: Object;
    fn CGColorSpaceCreateWithName(name: Object) -> Object;
    fn CGColorSpaceRelease(space: Object);
}
unsafe fn get(object: Object, selector: &std::ffi::CStr) -> Object {
    let send: unsafe extern "C" fn(Object, Object) -> Object =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { send(object, sel_registerName(selector.as_ptr())) }
}
unsafe fn responds(object: Object, selector: &std::ffi::CStr) -> bool {
    let send: unsafe extern "C" fn(Object, Object, Object) -> bool =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe {
        send(
            object,
            sel_registerName(c"respondsToSelector:".as_ptr()),
            sel_registerName(selector.as_ptr()),
        )
    }
}
/// Called on the UI thread after wgpu has attached its CAMetalLayer.
pub fn configure(window: &winit::window::Window) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let view = match handle.as_raw() {
        RawWindowHandle::AppKit(h) => h.ns_view.as_ptr(),
        RawWindowHandle::UiKit(h) => h.ui_view.as_ptr(),
        _ => return,
    };
    // SAFETY: winit's live window owns the view; setup occurs on the main thread.
    unsafe {
        let space = CGColorSpaceCreateWithName(kCGColorSpaceExtendedLinearSRGB);
        if space.is_null() {
            return;
        }
        set_layer(get(view, c"layer"), space, 0);
        CGColorSpaceRelease(space);
    }
}
unsafe fn set_layer(layer: Object, space: Object, depth: usize) {
    if layer.is_null() || depth > 8 {
        return;
    }
    // CALayer (including NSViewBackingLayer) also exposes wantsEDR on recent
    // Apple OSes, but only the Metal render layer owns this colorspace property.
    // Check the actual receiver and selector before sending an Objective-C message.
    let is_kind: unsafe extern "C" fn(Object, Object, Object) -> bool =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let metal_class = unsafe { objc_getClass(c"CAMetalLayer".as_ptr()) };
    if !metal_class.is_null()
        && unsafe {
            is_kind(
                layer,
                sel_registerName(c"isKindOfClass:".as_ptr()),
                metal_class,
            )
        }
        && unsafe { responds(layer, c"setColorspace:") }
    {
        let send: unsafe extern "C" fn(Object, Object, Object) =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { send(layer, sel_registerName(c"setColorspace:".as_ptr()), space) };
    }
    let children = unsafe { get(layer, c"sublayers") };
    if children.is_null() {
        return;
    }
    let count = unsafe { get(children, c"count") } as usize;
    let at: unsafe extern "C" fn(Object, Object, usize) -> Object =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    for i in 0..count.min(64) {
        unsafe {
            set_layer(
                at(children, sel_registerName(c"objectAtIndex:".as_ptr()), i),
                space,
                depth + 1,
            )
        };
    }
}
/// Active-window screen headroom; a missing/SDR screen selects the SDR fallback.
/// Must be called from the UI thread. The value may change with display brightness.
pub fn headroom(window: Option<&winit::window::Window>) -> f32 {
    // SAFETY: Objective-C selectors are tested before calling platform-dependent APIs.
    unsafe {
        #[cfg(target_os = "macos")]
        let (class, selector) = (
            c"NSScreen",
            c"maximumExtendedDynamicRangeColorComponentValue",
        );
        #[cfg(target_os = "ios")]
        let (class, selector) = (c"UIScreen", c"currentEDRHeadroom");
        let view = window
            .and_then(|w| w.window_handle().ok())
            .and_then(|h| match h.as_raw() {
                RawWindowHandle::AppKit(h) => Some(h.ns_view.as_ptr()),
                RawWindowHandle::UiKit(h) => Some(h.ui_view.as_ptr()),
                _ => None,
            });
        let screen = view
            .map(|view| {
                let window = get(view, c"window");
                #[cfg(target_os = "ios")]
                let window = if responds(window, c"windowScene") {
                    get(window, c"windowScene")
                } else {
                    window
                };
                get(window, c"screen")
            })
            .filter(|s| !s.is_null())
            .unwrap_or_else(|| get(objc_getClass(class.as_ptr()), c"mainScreen"));
        if screen.is_null() || !responds(screen, selector) {
            return 1.;
        }
        let send: unsafe extern "C" fn(Object, Object) -> f64 =
            std::mem::transmute(objc_msgSend as *const ());
        let value = send(screen, sel_registerName(selector.as_ptr())) as f32;
        if value.is_finite() { value.max(1.) } else { 1. }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFEqual(a: Object, b: Object) -> u8;
    }

    struct Layer(Object);
    impl Layer {
        fn new(class: &std::ffi::CStr) -> Self {
            // SAFETY: these are retained QuartzCore model objects; they do not
            // create a window or depend on the display server/event loop.
            let object = unsafe { get(objc_getClass(class.as_ptr()), c"new") };
            assert!(!object.is_null(), "QuartzCore layer class is available");
            Self(object)
        }
        fn add(&self, child: &Self) {
            let send: unsafe extern "C" fn(Object, Object, Object) =
                unsafe { std::mem::transmute(objc_msgSend as *const ()) };
            // SAFETY: CALayer and CAMetalLayer implement addSublayer: and
            // retain the live child. Each +new retain is released by Drop.
            unsafe { send(self.0, sel_registerName(c"addSublayer:".as_ptr()), child.0) };
        }
    }
    impl Drop for Layer {
        fn drop(&mut self) {
            let send: unsafe extern "C" fn(Object, Object) =
                unsafe { std::mem::transmute(objc_msgSend as *const ()) };
            unsafe { send(self.0, sel_registerName(c"release".as_ptr())) };
        }
    }

    #[test]
    fn generic_layer_parents_do_not_receive_metal_colorspace() {
        let parent = Layer::new(c"CALayer");
        let middle = Layer::new(c"CALayer");
        let metal = Layer::new(c"CAMetalLayer");
        parent.add(&middle);
        middle.add(&metal);
        // Since macOS 14 / iOS 17 generic CALayer also has the EDR setter.
        // Checking only that selector sent setColorspace: to the parent and
        // threw before reaching the actual Metal child during app startup.
        assert!(!unsafe { responds(parent.0, c"setColorspace:") });
        assert!(unsafe { responds(metal.0, c"setColorspace:") });
        unsafe {
            let space = CGColorSpaceCreateWithName(kCGColorSpaceExtendedLinearSRGB);
            assert!(!space.is_null());
            set_layer(parent.0, space, 0);
            let configured = get(metal.0, c"colorspace");
            let equal = !configured.is_null() && CFEqual(configured, space) != 0;
            CGColorSpaceRelease(space);
            assert!(
                equal,
                "the nested Metal child receives extended-linear sRGB"
            );
        }
    }
}
