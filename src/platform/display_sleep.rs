//! Process-scoped idle display inhibition; never changes OS power preferences.
use eframe::egui;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Default)]
pub struct DisplaySleep {
    sender: Option<Sender<bool>>,
    results: Option<Receiver<(bool, Result<(), String>)>>,
    requested: Option<bool>,
    error: Option<String>,
}

impl DisplaySleep {
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn update(&mut self, enabled: bool, ctx: &egui::Context) {
        if self.requested != Some(enabled) {
            self.requested = Some(enabled);
            self.error = None;
            if self.sender.is_none() && enabled {
                let (sender, commands) = mpsc::channel();
                let (results, receiver) = mpsc::channel();
                let ctx = ctx.clone();
                // Windows execution state belongs to the calling thread. Keep
                // acquisition, release and the lifetime on this same worker.
                // Linux D-Bus calls must not block image display or UI input.
                match std::thread::Builder::new()
                    .name("display-sleep".into())
                    .spawn(move || {
                        run_worker(commands, platform::acquire, |enabled, result| {
                            let _ = results.send((enabled, result));
                            ctx.request_repaint();
                        });
                    }) {
                    Ok(_) => {
                        self.sender = Some(sender);
                        self.results = Some(receiver);
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            if let Some(sender) = &self.sender
                && sender.send(enabled).is_err()
            {
                self.sender = None;
                self.error = Some("Display sleep worker stopped".into());
            }
        }
        if let Some(results) = &self.results {
            for (requested, result) in results.try_iter() {
                if requested == enabled {
                    let error = result.err();
                    if self.error != error {
                        self.error = error;
                        ctx.request_repaint();
                    }
                }
            }
        }
    }
}

fn run_worker<G>(
    commands: Receiver<bool>,
    mut acquire: impl FnMut() -> Result<G, String>,
    mut report: impl FnMut(bool, Result<(), String>),
) {
    let mut guard = None;
    while let Ok(enabled) = commands.recv() {
        let result = if enabled && guard.is_none() {
            acquire().map(|value| guard = Some(value))
        } else {
            if !enabled {
                guard = None;
            }
            Ok(())
        };
        report(enabled, result);
    }
    // Dropping the owner closes the channel and releases the native request.
}

#[cfg(all(target_os = "macos", not(test)))]
mod platform {
    unsafe extern "C" {
        fn nabi_display_sleep_acquire(assertion: *mut u32) -> i32;
        fn nabi_display_sleep_release(assertion: u32);
    }
    pub struct Guard(u32);
    pub fn acquire() -> Result<Guard, String> {
        let mut assertion = 0;
        let status = unsafe { nabi_display_sleep_acquire(&mut assertion) };
        if status == 0 {
            Ok(Guard(assertion))
        } else {
            Err(format!("IOPMAssertionCreateWithName: {status:#x}"))
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe { nabi_display_sleep_release(self.0) };
        }
    }
}

#[cfg(all(windows, not(test)))]
mod platform {
    use windows::Win32::System::Power::{
        ES_CONTINUOUS, ES_DISPLAY_REQUIRED, SetThreadExecutionState,
    };
    pub struct Guard;
    pub fn acquire() -> Result<Guard, String> {
        let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED) };
        if previous.0 != 0 {
            Ok(Guard)
        } else {
            Err("SetThreadExecutionState failed".into())
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }
}

#[cfg(all(target_os = "linux", not(test)))]
mod platform {
    use std::time::Duration;
    use zbus::blocking::{Connection, Proxy, connection::Builder};

    const SCREENSAVER: (&str, &str, &str) = (
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
        "UnInhibit",
    );
    const GNOME: (&str, &str, &str) = (
        "org.gnome.SessionManager",
        "/org/gnome/SessionManager",
        "Uninhibit",
    );
    pub struct Guard {
        connection: Connection,
        service: (&'static str, &'static str, &'static str),
        cookie: u32,
    }
    pub fn acquire() -> Result<Guard, String> {
        let connection = Builder::session()
            .map_err(|e| e.to_string())?
            .method_timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| e.to_string())?;
        let mut errors = Vec::new();
        for service in [SCREENSAVER, GNOME] {
            let result: zbus::Result<u32> = (|| {
                let proxy = Proxy::new(&connection, service.0, service.1, service.0)?;
                if service == GNOME {
                    // Inhibit becoming idle, not logout or explicit suspend.
                    proxy.call(
                        "Inhibit",
                        &("dev.nabi.NabiView", 0u32, "Viewing images", 8u32),
                    )
                } else {
                    proxy.call("Inhibit", &("dev.nabi.NabiView", "Viewing images"))
                }
            })();
            match result {
                Ok(cookie) => {
                    return Ok(Guard {
                        connection,
                        service,
                        cookie,
                    });
                }
                Err(error) => errors.push(error.to_string()),
            }
        }
        Err(errors.join("; "))
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Ok(proxy) = Proxy::new(
                &self.connection,
                self.service.0,
                self.service.1,
                self.service.0,
            ) {
                let _: zbus::Result<()> = proxy.call(self.service.2, &(self.cookie,));
            }
            // The dedicated bus connection also releases inhibition on exit.
        }
    }
}

#[cfg(all(any(target_os = "ios", target_os = "android"), not(test)))]
mod platform {
    pub struct Guard;
    pub fn acquire() -> Result<Guard, String> {
        crate::platform::mobile::keep_awake(true);
        Ok(Guard)
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            crate::platform::mobile::keep_awake(false);
        }
    }
}

#[cfg(all(
    not(any(
        target_os = "macos",
        windows,
        target_os = "linux",
        target_os = "ios",
        target_os = "android"
    )),
    not(test)
))]
mod platform {
    pub fn acquire() -> Result<(), String> {
        Err("Display inhibition is unavailable on this platform".into())
    }
}

#[cfg(test)]
mod platform {
    // Headless UI tests must not alter the machine's power state.
    pub fn acquire() -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn requests_are_released_on_disable_and_owner_exit() {
        let alive = Arc::new(AtomicUsize::new(0));
        let counter = alive.clone();
        let (sender, commands) = mpsc::channel();
        let (report, results) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_worker(
                commands,
                || {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(Guard(counter.clone()))
                },
                |enabled, result| {
                    report.send((enabled, result)).unwrap();
                },
            )
        });
        for (enabled, expected) in [(true, 1), (true, 1), (false, 0), (true, 1)] {
            sender.send(enabled).unwrap();
            let (reported, result) = results.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(reported, enabled);
            result.unwrap();
            assert_eq!(alive.load(Ordering::SeqCst), expected);
        }
        drop(sender);
        worker.join().unwrap();
        assert_eq!(alive.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn failed_request_can_be_disabled_and_retried() {
        let (sender, commands) = mpsc::channel();
        for enabled in [true, false, true] {
            sender.send(enabled).unwrap();
        }
        drop(sender);
        let mut attempts = 0;
        let mut results = Vec::new();
        run_worker(
            commands,
            || {
                attempts += 1;
                if attempts == 1 {
                    Err("unavailable".into())
                } else {
                    Ok(())
                }
            },
            |_, result| results.push(result),
        );
        assert_eq!(attempts, 2);
        assert!(results[0].is_err());
        assert!(results[1].is_ok() && results[2].is_ok());
    }
}
