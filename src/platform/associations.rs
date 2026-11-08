//! OS-owned associations, queried off the UI thread.
#[cfg(any(windows, target_os = "linux"))]
use std::process::Command;
use std::{collections::HashMap, sync::mpsc};
type AssociationStates = HashMap<String, Option<bool>>;
type AssociationResult = (AssociationStates, String);
#[derive(Default)]
pub struct Associations {
    pub states: AssociationStates,
    pub message: String,
    last_refresh: Option<std::time::Instant>,
    receiver: Option<mpsc::Receiver<AssociationResult>>,
}
impl Associations {
    pub fn refresh(&mut self, extension: Option<String>) {
        if self.receiver.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.last_refresh = Some(std::time::Instant::now());
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            let message = extension
                .map(|ext| connect(&ext).err().unwrap_or_default())
                .unwrap_or_default();
            let states = crate::core::media::EXTENSIONS
                .iter()
                .map(|ext| (ext.to_string(), is_default(ext)))
                .collect();
            let _ = tx.send((states, message));
        });
    }
    pub fn poll(&mut self) {
        if let Some(rx) = &self.receiver
            && let Ok((states, message)) = rx.try_recv()
        {
            self.states = states;
            self.message = message;
            self.receiver = None;
        }
        if self.receiver.is_none()
            && self
                .last_refresh
                .is_some_and(|time| time.elapsed().as_secs() >= 3)
        {
            self.refresh(None);
        }
    }
}
#[cfg(any(windows, target_os = "linux"))]
fn command(name: &str) -> Command {
    let cmd = Command::new(name);
    #[cfg(windows)]
    let cmd = {
        use std::os::windows::process::CommandExt;
        let mut cmd = cmd;
        cmd.creation_flags(0x08000000);
        cmd
    };
    cmd
}
#[cfg(any(windows, target_os = "linux"))]
fn run(name: &str, args: &[&str]) -> Result<(), String> {
    let output = command(name)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}
pub fn per_type() -> bool {
    cfg!(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux"
    ))
}
#[cfg(windows)]
fn is_default(ext: &str) -> Option<bool> {
    #[link(name = "shlwapi")]
    unsafe extern "system" {
        fn AssocQueryStringW(
            flags: u32,
            string: u32,
            assoc: *const u16,
            extra: *const u16,
            out: *mut u16,
            len: *mut u32,
        ) -> i32;
    }
    let extension: Vec<u16> = format!(".{ext}\0").encode_utf16().collect();
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    let result = unsafe {
        AssocQueryStringW(
            0,
            2,
            extension.as_ptr(),
            std::ptr::null(),
            buf.as_mut_ptr(),
            &mut len,
        )
    };
    if result != 0 {
        return None;
    }
    let executable = std::env::current_exe().ok()?;
    let associated = String::from_utf16_lossy(&buf[..len.saturating_sub(1) as usize]);
    Some(associated.eq_ignore_ascii_case(&executable.to_string_lossy()))
}
#[cfg(windows)]
fn connect(_: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let open = format!("\"{}\" \"%1\"", exe.display());
    let root = "HKCU\\Software\\Classes\\NabiView.Image";
    run(
        "reg",
        &[
            "add",
            &format!("{root}\\DefaultIcon"),
            "/ve",
            "/t",
            "REG_SZ",
            "/d",
            &format!("\"{}\",0", exe.display()),
            "/f",
        ],
    )?;
    run(
        "reg",
        &[
            "add",
            &format!("{root}\\shell\\open\\command"),
            "/ve",
            "/t",
            "REG_SZ",
            "/d",
            &open,
            "/f",
        ],
    )?;
    let capabilities = "HKCU\\Software\\NabiView\\Capabilities";
    run(
        "reg",
        &[
            "add",
            capabilities,
            "/v",
            "ApplicationName",
            "/t",
            "REG_SZ",
            "/d",
            "NABI VIEW",
            "/f",
        ],
    )?;
    for ext in crate::core::media::EXTENSIONS {
        run(
            "reg",
            &[
                "add",
                &format!("{capabilities}\\FileAssociations"),
                "/v",
                &format!(".{ext}"),
                "/t",
                "REG_SZ",
                "/d",
                "NabiView.Image",
                "/f",
            ],
        )?;
    }
    run(
        "reg",
        &[
            "add",
            "HKCU\\Software\\RegisteredApplications",
            "/v",
            "NABI VIEW",
            "/t",
            "REG_SZ",
            "/d",
            "Software\\NabiView\\Capabilities",
            "/f",
        ],
    )?;
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: isize,
            op: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> isize;
    }
    let uri: Vec<u16> = "ms-settings:defaultapps?registeredAppUser=Nabi%20View\0"
        .encode_utf16()
        .collect();
    let result = unsafe {
        ShellExecuteW(
            0,
            std::ptr::null(),
            uri.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if result <= 32 {
        Err("Could not open Windows default apps settings".into())
    } else {
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn mime(ext: &str) -> &str {
    match ext {
        "jpg" | "jpeg" | "jfif" => "image/jpeg",
        "png" | "apng" => "image/png",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "ico" => "image/vnd.microsoft.icon",
        "pbm" => "image/x-portable-bitmap",
        "pgm" => "image/x-portable-graymap",
        "ppm" => "image/x-portable-pixmap",
        "tga" => "image/x-tga",
        "bmp" => "image/bmp",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "jxl" => "image/jxl",
        _ => "",
    }
}
#[cfg(target_os = "linux")]
fn is_default(ext: &str) -> Option<bool> {
    if mime(ext).is_empty() {
        return None;
    }
    let out = command("xdg-mime")
        .args(["query", "default", mime(ext)])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim() == "dev.nabi.NabiView.desktop")
}
#[cfg(target_os = "linux")]
fn connect(ext: &str) -> Result<(), String> {
    if mime(ext).is_empty() {
        return Err("No MIME association is available for this format".into());
    }
    let dir = dirs::data_dir()
        .ok_or("Missing data directory")?
        .join("applications");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let escaped = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    let icons = dirs::data_dir()
        .ok_or("Missing data directory")?
        .join("icons/hicolor/256x256/apps");
    std::fs::create_dir_all(&icons).map_err(|e| e.to_string())?;
    std::fs::write(
        icons.join("dev.nabi.NabiView.png"),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/png/nabi-icon-256.png"
        )),
    )
    .map_err(|e| e.to_string())?;
    let types = crate::core::media::EXTENSIONS
        .iter()
        .map(|x| mime(x))
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(";");
    std::fs::write(dir.join("dev.nabi.NabiView.desktop"),format!("[Desktop Entry]\nType=Application\nName=NABI VIEW\nIcon=dev.nabi.NabiView\nExec=\"{escaped}\" %f\nMimeType={types};\nTerminal=false\n")).map_err(|e|e.to_string())?;
    run(
        "xdg-mime",
        &["default", "dev.nabi.NabiView.desktop", mime(ext)],
    )
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn nabi_default_for_extension(ext: *const std::ffi::c_char) -> i32;
    fn nabi_connect_extension(ext: *const std::ffi::c_char) -> i32;
}
#[cfg(target_os = "macos")]
fn is_default(ext: &str) -> Option<bool> {
    let ext = std::ffi::CString::new(ext).ok()?;
    match unsafe { nabi_default_for_extension(ext.as_ptr()) } {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    }
}
#[cfg(target_os = "macos")]
fn connect(ext: &str) -> Result<(), String> {
    let ext = std::ffi::CString::new(ext).map_err(|e| e.to_string())?;
    if unsafe { nabi_connect_extension(ext.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(
            "Could not change the default app. Use Finder > Get Info > Open with > Change All."
                .into(),
        )
    }
}
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn is_default(_: &str) -> Option<bool> {
    None
}
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn connect(_: &str) -> Result<(), String> {
    Err("Open your device's app settings to choose an image viewer.".into())
}
