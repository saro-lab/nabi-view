//! Encoding disposable cache files never holds up the next visible decode.
use super::Frame;
use crate::core::disk_cache::{Cache, Key};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread,
};

type Write = (Cache, Key, eframe::egui::ColorImage, [u32; 2]);
pub(super) struct Writer {
    send: SyncSender<Write>,
    busy: Arc<AtomicBool>,
}
impl Writer {
    pub fn new() -> Self {
        // One output total, either queued or in flight. Skip excess work before
        // cloning pixels, so a slow disk cannot slow every subsequent decode.
        let (send, receive) = sync_channel::<Write>(1);
        let busy = Arc::new(AtomicBool::new(false));
        let working = busy.clone();
        thread::spawn(move || {
            while let Ok((cache, key, image, original)) = receive.recv() {
                let _ = cache.store(&key, &image, original);
                working.store(false, Ordering::Release);
            }
        });
        Self { send, busy }
    }
    pub fn store(&self, cache: Cache, key: Key, frame: &Frame) {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        if self
            .send
            .try_send((cache, key, frame.image.clone(), frame.original_size))
            .is_err()
        {
            self.busy.store(false, Ordering::Release);
        }
    }
}
