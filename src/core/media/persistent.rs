//! Encoding disposable cache files never holds up the next visible decode.
//! A small priority queue keeps recent results instead of dropping every
//! output that arrives while one file is being written.
use super::Frame;
use crate::core::disk_cache::{Cache, Key};
use std::{
    sync::{Arc, Condvar, Mutex},
    thread,
};

/// Queued outputs at most. Each holds one display frame (≤ 24 MiB RGBA).
const CAPACITY: usize = 8;

/// Which output survives when the queue is full: higher is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Priority {
    Prefetch,
    Thumbnail,
    Visible,
}

struct Write {
    priority: Priority,
    cache: Cache,
    key: Key,
    image: eframe::egui::ColorImage,
    original: [u32; 2],
}

#[derive(Default)]
struct Queue {
    items: Vec<Write>,
    busy: bool,
    closed: bool,
}

/// Admission decision for a full or duplicate queue, decided before cloning
/// pixels so a slow disk costs the decoders nothing.
#[derive(Debug, PartialEq, Eq)]
enum Admit {
    Skip,
    Append,
    Replace(usize),
}

fn admit(
    items: &[(Priority, &Key, &Cache)],
    priority: Priority,
    key: &Key,
    cache: &Cache,
) -> Admit {
    if items
        .iter()
        .any(|(_, queued, owner)| *owner == cache && queued.same_entry(key))
    {
        return Admit::Skip;
    }
    if items.len() < CAPACITY {
        return Admit::Append;
    }
    // Evict the oldest entry of the lowest priority; a newer result of equal
    // priority is closer to where the viewer is now.
    let (index, lowest) = items
        .iter()
        .enumerate()
        .map(|(index, item)| (index, item.0))
        .min_by_key(|&(index, priority)| (priority, index))
        .unwrap();
    if lowest > priority {
        Admit::Skip
    } else {
        Admit::Replace(index)
    }
}

pub(super) struct Writer(Arc<(Mutex<Queue>, Condvar)>);

impl Writer {
    pub fn new() -> Self {
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name("nabi-cache".into())
            .spawn(move || {
                crate::platform::worker_priority::background();
                let (queue, changed) = &*worker;
                loop {
                    let write = {
                        let mut q = queue.lock().unwrap();
                        q.busy = false;
                        changed.notify_all();
                        loop {
                            if q.closed {
                                return;
                            }
                            // Highest priority first, oldest among equals.
                            if let Some(index) = q
                                .items
                                .iter()
                                .enumerate()
                                .max_by_key(|(index, item)| {
                                    (item.priority, std::cmp::Reverse(*index))
                                })
                                .map(|(index, _)| index)
                            {
                                q.busy = true;
                                break q.items.remove(index);
                            }
                            q = changed.wait(q).unwrap();
                        }
                    };
                    let _ = write.cache.store(&write.key, &write.image, write.original);
                }
            })
            .expect("could not start the disk-cache worker");
        Self(shared)
    }

    pub fn store(&self, cache: Cache, key: Key, frame: &Frame, priority: Priority) {
        let (queue, changed) = &*self.0;
        let mut q = queue.lock().unwrap();
        let queued: Vec<_> = q
            .items
            .iter()
            .map(|item| (item.priority, &item.key, &item.cache))
            .collect();
        match admit(&queued, priority, &key, &cache) {
            Admit::Skip => return,
            Admit::Append => {}
            Admit::Replace(index) => {
                q.items.remove(index);
            }
        }
        q.items.push(Write {
            priority,
            cache,
            key,
            image: frame.image.clone(),
            original: frame.original_size,
        });
        changed.notify_all();
    }

    /// Blocks until every queued output is written.
    #[cfg(test)]
    pub fn flush(&self) {
        let (queue, changed) = &*self.0;
        let mut q = queue.lock().unwrap();
        while !q.items.is_empty() || q.busy {
            q = changed.wait(q).unwrap();
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let (queue, changed) = &*self.0;
        queue.lock().unwrap().closed = true;
        changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    fn fixture(count: usize) -> (tempfile::TempDir, Cache, Vec<std::path::PathBuf>) {
        let root = tempfile::tempdir().unwrap();
        let cache = Cache::new(root.path().join("cache"), 64 * 1024 * 1024);
        let sources = (0..count)
            .map(|index| {
                let path = root.path().join(format!("{index}.png"));
                std::fs::write(&path, format!("source {index}")).unwrap();
                path
            })
            .collect();
        (root, cache, sources)
    }

    fn frame() -> Frame {
        Frame {
            hdr: None,
            animation: None,
            image: egui::ColorImage::filled([4, 2], egui::Color32::from_rgb(200, 30, 10)),
            original_size: [40, 20],
        }
    }

    #[test]
    fn burst_of_outputs_is_written_and_reused_after_restart() {
        let (_root, cache, sources) = fixture(CAPACITY);
        let writer = Writer::new();
        for source in &sources {
            writer.store(
                cache.clone(),
                cache.key(source, 512).unwrap(),
                &frame(),
                Priority::Prefetch,
            );
        }
        writer.flush();
        drop(writer);
        // A new value stands in for the next process: nothing in memory.
        let restarted = Cache::new(cache.root().to_owned(), 64 * 1024 * 1024);
        for source in &sources {
            let loaded = restarted
                .load(&restarted.key(source, 512).unwrap())
                .unwrap();
            assert_eq!(loaded.original_size, [40, 20]);
        }
    }

    #[test]
    fn full_queue_keeps_important_outputs_and_skips_duplicates() {
        let (_root, cache, sources) = fixture(CAPACITY + 2);
        let keys: Vec<_> = sources.iter().map(|s| cache.key(s, 512).unwrap()).collect();
        let mut queued: Vec<(Priority, &Key, &Cache)> = Vec::new();
        assert_eq!(
            admit(&queued, Priority::Prefetch, &keys[0], &cache),
            Admit::Append
        );
        queued.push((Priority::Thumbnail, &keys[0], &cache));
        assert_eq!(
            admit(&queued, Priority::Visible, &keys[0], &cache),
            Admit::Skip
        );
        for key in &keys[1..CAPACITY] {
            queued.push((Priority::Prefetch, key, &cache));
        }
        assert_eq!(queued.len(), CAPACITY);
        // The oldest prefetch output makes room for anything at least as useful.
        assert_eq!(
            admit(&queued, Priority::Visible, &keys[CAPACITY], &cache),
            Admit::Replace(1)
        );
        assert_eq!(
            admit(&queued, Priority::Prefetch, &keys[CAPACITY], &cache),
            Admit::Replace(1)
        );
        for item in &mut queued {
            item.0 = Priority::Visible;
        }
        assert_eq!(
            admit(&queued, Priority::Prefetch, &keys[CAPACITY + 1], &cache),
            Admit::Skip
        );
    }
}
