//! CPU priority for dedicated, non-visible workers.

/// Call once from a newly started background worker, never from the UI or a
/// worker that also decodes the current image. Android Rust threads inherit the
/// spawning thread's priority, which can otherwise leave thumbnails and cache
/// encoding competing with the viewer at the same elevated CPU priority.
///
/// This only lowers the calling thread's CPU priority. It does not change the
/// process, Android task groups, I/O priority, or other platforms' scheduling.
/// An unavailable OS adjustment leaves the worker's normal behavior intact.
pub(crate) fn background() {
    #[cfg(target_os = "android")]
    unsafe {
        unsafe extern "C" {
            fn nabi_android_background_worker();
        }
        // The native wrapper obtains this worker's TID, accepts no external
        // target, and never raises an already lower-priority thread.
        nabi_android_background_worker();
    }
}
