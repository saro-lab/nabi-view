//! A user cache limit is a ceiling, never a reservation of system memory.
use crate::platform::system_memory::{self, Snapshot};
use eframe::egui;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

const MIB: usize = 1024 * 1024;
const UNKNOWN_BUDGET: usize = 256 * MIB;
const RECOVERY_STEP: usize = 64 * MIB;
const RECOVERY_SAMPLES: u8 = 5;

pub struct Monitor {
    latest: Arc<Mutex<Option<Option<Snapshot>>>>,
    // Dropping this sender wakes and stops the worker, including in idle windows.
    _stop: Option<mpsc::Sender<()>>,
}

impl Monitor {
    pub fn new(ctx: egui::Context) -> Self {
        let latest = Arc::new(Mutex::new(None));
        let worker_latest = Arc::clone(&latest);
        let (stop, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("memory-monitor".into())
            .spawn(move || {
                loop {
                    *worker_latest.lock().unwrap() = Some(system_memory::snapshot());
                    ctx.request_repaint();
                    if receiver.recv_timeout(Duration::from_secs(1))
                        != Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        break;
                    }
                }
            });
        Self {
            latest,
            _stop: spawned.ok().map(|_| stop),
        }
    }

    pub fn take(&self) -> Option<Option<Snapshot>> {
        self.latest.lock().unwrap().take()
    }

    #[cfg(test)]
    pub fn disabled() -> Self {
        Self {
            latest: Arc::new(Mutex::new(None)),
            _stop: None,
        }
    }
}

pub struct Policy {
    ceiling: usize,
    sampled: bool,
    sample_valid: bool,
    pressure: bool,
    healthy_samples: u8,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            ceiling: UNKNOWN_BUDGET,
            sampled: false,
            sample_valid: false,
            pressure: false,
            healthy_samples: 0,
        }
    }
}

impl Policy {
    pub fn budget(&self, configured: usize) -> usize {
        configured.min(self.ceiling)
    }

    pub fn background_allowed(&self) -> bool {
        self.sample_valid && !self.pressure
    }

    /// Called once per OS sample, not per frame. The snapshot excludes memory
    /// already in use, so include retained cache bytes when computing headroom.
    pub fn observe(&mut self, snapshot: Option<Snapshot>, cached: usize) {
        let Some(snapshot) = snapshot.filter(|s| s.total > 0 && s.available <= s.total) else {
            self.sample_valid = false;
            self.healthy_samples = 0;
            self.ceiling = self.ceiling.min(UNKNOWN_BUDGET);
            return;
        };
        self.sample_valid = true;
        let reserve = (snapshot.total / 10).max(256 * MIB as u64);
        if snapshot.available <= reserve {
            // Keep only what the viewer needs to draw. This may be below the
            // smallest configurable limit, without rewriting saved settings.
            self.pressure = true;
            self.sampled = true;
            self.healthy_samples = 0;
            self.ceiling = 0;
            return;
        }
        if self.pressure {
            let recovery = reserve.saturating_add((snapshot.total / 20).max(128 * MIB as u64));
            if snapshot.available < recovery {
                self.healthy_samples = 0;
                return;
            }
            self.healthy_samples += 1;
            if self.healthy_samples < RECOVERY_SAMPLES {
                return;
            }
            self.pressure = false;
            self.healthy_samples = 0;
        }
        // Leave most of the remaining headroom to decoders, other windows and
        // other applications. This is an estimate, not an allocation guarantee.
        let headroom = (snapshot.available - reserve) / 4;
        let target = (cached as u64)
            .saturating_add(headroom)
            .min(snapshot.total / 4)
            .min(usize::MAX as u64) as usize;
        self.ceiling = if self.sampled {
            target.min(self.ceiling.saturating_add(RECOVERY_STEP))
        } else {
            target
        };
        self.sampled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(available_mib: u64) -> Option<Snapshot> {
        Some(Snapshot {
            total: 16 * 1024 * MIB as u64,
            available: available_mib * MIB as u64,
        })
    }

    #[test]
    fn pressure_overrides_user_limit_and_recovery_is_delayed_and_gradual() {
        let mut policy = Policy::default();
        policy.observe(memory(12 * 1024), 0);
        assert_eq!(policy.budget(2048 * MIB), 2048 * MIB);
        policy.observe(memory(500), 1800 * MIB);
        assert_eq!(policy.budget(2048 * MIB), 0);
        assert!(!policy.background_allowed());
        for _ in 0..4 {
            policy.observe(memory(8 * 1024), 20 * MIB);
            assert!(!policy.background_allowed());
        }
        policy.observe(memory(8 * 1024), 20 * MIB);
        assert!(policy.background_allowed());
        assert_eq!(policy.budget(2048 * MIB), 64 * MIB);
        policy.observe(memory(8 * 1024), 20 * MIB);
        assert_eq!(policy.budget(2048 * MIB), 128 * MIB);
        assert_eq!(policy.budget(32 * MIB), 32 * MIB);
    }

    #[test]
    fn recovery_hysteresis_and_failed_samples_do_not_restart_prefetch() {
        let mut policy = Policy::default();
        policy.observe(memory(100), 300 * MIB);
        for _ in 0..4 {
            policy.observe(memory(8192), 20 * MIB);
        }
        policy.observe(memory(2000), 20 * MIB); // Above low threshold, below recovery.
        policy.observe(memory(8192), 20 * MIB);
        assert!(!policy.background_allowed());
        policy.observe(None, 20 * MIB);
        for _ in 0..4 {
            policy.observe(memory(8192), 20 * MIB);
        }
        assert!(!policy.background_allowed());
        assert_eq!(policy.budget(2048 * MIB), 0);
    }

    #[test]
    fn unknown_memory_is_conservative_and_small_devices_reduce_the_budget() {
        let mut policy = Policy::default();
        assert!(!policy.background_allowed());
        assert_eq!(policy.budget(2048 * MIB), 256 * MIB);
        policy.observe(
            Some(Snapshot {
                total: 2 * 1024 * MIB as u64,
                available: 1024 * MIB as u64,
            }),
            0,
        );
        assert_eq!(policy.budget(2048 * MIB), 192 * MIB);
        assert!(policy.background_allowed());
        policy.observe(None, 0);
        assert!(!policy.background_allowed());
        assert_eq!(policy.budget(2048 * MIB), 192 * MIB);
    }
}
