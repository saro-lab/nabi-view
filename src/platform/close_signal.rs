//! Per-user, cross-process graceful close requests. No process killing or PID files.
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct CloseSignal {
    path: PathBuf,
    seen: Arc<Mutex<Vec<u8>>>,
    requested: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl CloseSignal {
    pub fn new(path: PathBuf, ctx: eframe::egui::Context) -> Self {
        let seen = Arc::new(Mutex::new(fs::read(&path).unwrap_or_default()));
        let requested = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let (watch_path, watch_seen, watch_request, watch_stop) = (
            path.clone(),
            seen.clone(),
            requested.clone(),
            stopped.clone(),
        );
        std::thread::spawn(move || {
            while !watch_stop.load(Ordering::Relaxed) {
                {
                    let mut seen = watch_seen.lock().unwrap();
                    if let Ok(token) = fs::read(&watch_path)
                        && !token.is_empty()
                        && token != *seen
                    {
                        *seen = token;
                        watch_request.store(true, Ordering::Relaxed);
                        ctx.request_repaint();
                    }
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        });
        Self {
            path,
            seen,
            requested,
            stopped,
        }
    }

    pub fn broadcast(&mut self) -> Result<(), String> {
        let parent = self.path.parent().ok_or("Missing app state directory")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        let token = format!(
            "{}:{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        );
        file.write_all(token.as_bytes())
            .map_err(|e| e.to_string())?;
        let mut seen = self.seen.lock().map_err(|e| e.to_string())?;
        file.persist(&self.path).map_err(|e| e.to_string())?;
        *seen = token.into_bytes();
        Ok(())
    }

    pub fn poll(&mut self) -> bool {
        self.requested.swap(false, Ordering::Relaxed)
    }
}

impl Drop for CloseSignal {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Helper launched by the cross-process close test"]
    fn receiver_process() {
        let root = PathBuf::from(std::env::var_os("NABI_CLOSE_TEST_ROOT").unwrap());
        let id = std::env::var("NABI_CLOSE_TEST_ID").unwrap();
        let mut signal = CloseSignal::new(root.join("close-all"), Default::default());
        fs::write(root.join(id), b"ready").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !signal.poll() {
            assert!(
                std::time::Instant::now() < deadline,
                "close signal was not received"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn close_request_reaches_multiple_independent_processes() {
        let root = crate::test_support::root();
        fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(root).unwrap();
        let mut children = Vec::new();
        for id in ["one", "two", "three"] {
            children.push(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "platform::close_signal::tests::receiver_process",
                        "--ignored",
                    ])
                    .env("NABI_CLOSE_TEST_ROOT", dir.path())
                    .env("NABI_CLOSE_TEST_ID", id)
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !["one", "two", "three"]
            .iter()
            .all(|id| dir.path().join(id).exists())
        {
            assert!(
                std::time::Instant::now() < deadline,
                "listeners failed to start"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        CloseSignal::new(dir.path().join("close-all"), Default::default())
            .broadcast()
            .unwrap();
        for mut child in children {
            assert!(child.wait().unwrap().success());
        }
    }

    #[test]
    fn existing_windows_close_but_later_windows_ignore_old_requests() {
        let root = crate::test_support::root();
        fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(root).unwrap();
        let path = dir.path().join("state/close-all");
        let mut first = CloseSignal::new(path.clone(), Default::default());
        let mut second = CloseSignal::new(path.clone(), Default::default());
        first.broadcast().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !second.poll() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!first.poll());
        assert!(!second.poll());
        assert!(!CloseSignal::new(path, Default::default()).poll());
    }
}
