//! Android picker reservations outlive a return to the viewer's home screen.
//! Keep cancellation independent of the mutex held while copying a document.
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

pub(super) struct OpenGate {
    // Even nonzero values are live; the low bit cancels that reservation.
    request: AtomicU64,
    importing: AtomicU64,
    publication: Mutex<()>,
}

impl OpenGate {
    pub(super) const fn new() -> Self {
        Self {
            request: AtomicU64::new(1),
            importing: AtomicU64::new(0),
            publication: Mutex::new(()),
        }
    }

    /// Called only after Java successfully reserves its single picker/worker.
    pub(super) fn start<T>(&self, clear_completed: impl FnOnce() -> T) -> T {
        let _publication = self.publication.lock().unwrap();
        let next = (self.request.load(Ordering::Relaxed) | 1)
            .wrapping_add(1)
            .max(2);
        self.request.store(next, Ordering::Release);
        clear_completed()
    }

    /// The caller clears only completion mailboxes, never the copying batch.
    pub(super) fn cancel<T>(&self, clear_completed: impl FnOnce() -> T) -> T {
        let _publication = self.publication.lock().unwrap();
        self.request.fetch_or(1, Ordering::AcqRel);
        clear_completed()
    }

    pub(super) fn request(&self) -> Option<u64> {
        let request = self.request.load(Ordering::Acquire);
        (request != 0 && request & 1 == 0).then_some(request)
    }

    pub(super) fn current(&self, request: u64) -> bool {
        request != 0 && request & 1 == 0 && self.request.load(Ordering::Acquire) == request
    }

    pub(super) fn begin(&self) -> Option<u64> {
        let request = self.request()?;
        self.importing.store(request, Ordering::Release);
        Some(request)
    }

    pub(super) fn importing(&self) -> Option<u64> {
        let request = self.importing.load(Ordering::Acquire);
        self.current(request).then_some(request)
    }

    pub(super) fn reader<R: std::io::Read>(&self, request: u64, source: R) -> ImportReader<'_, R> {
        ImportReader {
            gate: self,
            request,
            source,
        }
    }

    /// Only the final rename and mailbox publication run here. Provider I/O,
    /// serialization, fsync and disposal of copied files stay outside this gate.
    pub(super) fn publish<T>(&self, request: u64, publish: impl FnOnce() -> T) -> Option<T> {
        let _publication = self.publication.lock().unwrap();
        self.current(request).then(publish)
    }
}

pub(super) struct ImportReader<'a, R> {
    gate: &'a OpenGate,
    request: u64,
    source: R,
}

impl<R: std::io::Read> std::io::Read for ImportReader<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if !self.gate.current(self.request) {
            return Err(std::io::Error::other("Document opening cancelled."));
        }
        self.source.read(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};

    #[test]
    fn home_before_begin_rejects_late_callbacks_until_a_new_reservation() {
        let gate = OpenGate::new();
        gate.start(|| {});
        let old = gate.request().unwrap();
        gate.cancel(|| {});
        assert!(gate.begin().is_none());
        assert!(
            gate.publish(old, || panic!("cancelled request published"))
                .is_none()
        );
        // A rejected Java reservation never calls start and cannot undo home.
        assert!(gate.request().is_none());
        gate.start(|| {});
        let new = gate.begin().unwrap();
        assert_ne!(old, new);
        assert!(gate.current(new));
        assert!(!gate.current(old));
    }

    #[test]
    fn home_does_not_wait_for_a_worker_holding_the_import_batch() {
        let gate = Arc::new(OpenGate::new());
        let copying_batch = Arc::new(Mutex::new(()));
        gate.start(|| {});
        let request = gate.begin().unwrap();
        let held_batch = copying_batch.lock().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let cancel = gate.clone();
        let thread = std::thread::spawn(move || {
            cancel.cancel(|| {});
            sent.send(()).unwrap();
        });
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(gate.importing().is_none());
        assert!(
            gate.publish(request, || panic!("late copy published"))
                .is_none()
        );
        drop(held_batch);
        thread.join().unwrap();
    }

    #[test]
    fn home_clears_an_already_published_completion_and_blocks_late_reuse() {
        let gate = OpenGate::new();
        let completion = Mutex::new(None);
        gate.start(|| {});
        let request = gate.begin().unwrap();
        gate.publish(request, || *completion.lock().unwrap() = Some("photo"));
        gate.cancel(|| {
            completion.lock().unwrap().take();
        });
        assert!(completion.lock().unwrap().is_none());
        assert!(
            gate.publish(request, || *completion.lock().unwrap() = Some("reuse"))
                .is_none()
        );
        gate.start(|| {});
        // An old finish must not become current when a new picker is reserved.
        assert!(gate.importing().is_none());
        let new = gate.begin().unwrap();
        gate.publish(new, || *completion.lock().unwrap() = Some("new photo"));
        assert_eq!(*completion.lock().unwrap(), Some("new photo"));
    }

    #[test]
    fn cancellation_and_atomic_source_publication_have_one_order() {
        let gate = Arc::new(OpenGate::new());
        let completion = Arc::new(Mutex::new(None));
        gate.start(|| {});
        let request = gate.begin().unwrap();
        let (entered, publishing) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let worker_gate = gate.clone();
        let worker_completion = completion.clone();
        let worker = std::thread::spawn(move || {
            worker_gate.publish(request, || {
                entered.send(()).unwrap();
                released.recv().unwrap();
                *worker_completion.lock().unwrap() = Some("committed");
            })
        });
        publishing.recv_timeout(Duration::from_secs(2)).unwrap();
        let cancel_gate = gate.clone();
        let cancel_completion = completion.clone();
        let cancellation = std::thread::spawn(move || {
            cancel_gate.cancel(|| {
                cancel_completion.lock().unwrap().take();
            })
        });
        release.send(()).unwrap();
        worker.join().unwrap();
        cancellation.join().unwrap();
        assert!(gate.request().is_none());
        assert!(completion.lock().unwrap().is_none());
    }

    #[test]
    fn home_during_a_real_batch_copy_stops_the_next_read_without_waiting() {
        use crate::core::documents::ImportBatch;
        use std::sync::mpsc;
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let gate = Arc::new(OpenGate::new());
        gate.start(|| {});
        let request = gate.begin().unwrap();
        let batch = Arc::new(Mutex::new(ImportBatch::new_folder(root.path()).unwrap()));
        let (entered, copying) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        struct SlowSource(mpsc::Sender<()>, mpsc::Receiver<()>, bool);
        impl std::io::Read for SlowSource {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                assert!(!self.2, "cancelled provider read again");
                self.2 = true;
                self.0.send(()).unwrap();
                self.1.recv().unwrap();
                output[..4].copy_from_slice(b"data");
                Ok(4)
            }
        }
        let worker_gate = gate.clone();
        let worker_batch = batch.clone();
        let worker = std::thread::spawn(move || {
            worker_batch
                .lock()
                .unwrap()
                .add_with_modified(
                    "page.png",
                    worker_gate.reader(request, SlowSource(entered, resumed, false)),
                    None,
                )
                .unwrap();
            assert!(
                worker_gate
                    .publish(request, || panic!("cancelled copy published"))
                    .is_none()
            );
        });
        copying.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(batch.try_lock().is_err());
        gate.cancel(|| {});
        resume.send(()).unwrap();
        worker.join().unwrap();
        assert!(batch.lock().unwrap().files().is_empty());
    }

    #[test]
    fn cancelled_prepared_source_keeps_the_previous_list_and_owned_cache() {
        use crate::core::documents::{ImportBatch, source_record};
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let sources = root.path().join("document-sources");
        let cache = root.path().join("cache");
        let mut current = ImportBatch::new_folder(&cache).unwrap();
        current
            .set_source(&sources, "nabi-android-pictures", "Photos")
            .unwrap();
        current.add("old.png", &b"old"[..]).unwrap();
        current.commit_source().unwrap();
        let source = current.source().unwrap().root.clone();
        let retained = current.first().unwrap().to_owned();
        let mut cancelled = ImportBatch::new_folder(&cache).unwrap();
        cancelled
            .set_source(&sources, "nabi-android-pictures", "Photos")
            .unwrap();
        cancelled.add("cancelled.png", &b"new"[..]).unwrap();
        let gate = OpenGate::new();
        gate.start(|| {});
        let request = gate.begin().unwrap();
        let prepared = cancelled.prepare_source_commit().unwrap();
        gate.cancel(|| {});
        assert!(
            gate.publish(request, || cancelled.commit_prepared_source(prepared))
                .is_none()
        );
        drop(cancelled);
        assert_eq!(
            source_record(&source).unwrap().pages,
            vec![std::path::PathBuf::from("old.png")]
        );
        assert_eq!(std::fs::read(retained).unwrap(), b"old");
    }
}
