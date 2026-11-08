//! User UI language preferences, never keyboard layout or regional number settings.
use crate::i18n::Language;
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

type CachedLanguages = Option<(Instant, Vec<String>)>;
static CACHE: OnceLock<Mutex<CachedLanguages>> = OnceLock::new();

pub fn preferred_languages() -> Vec<String> {
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some((at, languages)) = &*cache
        && at.elapsed() < Duration::from_secs(2)
    {
        return languages.clone();
    }
    let languages = native_languages();
    *cache = Some((Instant::now(), languages.clone()));
    languages
}

pub fn set_application_language(language: &Language) {
    #[cfg(target_os = "android")]
    {
        unsafe extern "C" {
            fn nabi_mobile_set_language(code: *const std::ffi::c_char);
        }
        let code = if matches!(language, Language::System) {
            ""
        } else {
            language.code()
        };
        if let Ok(code) = std::ffi::CString::new(code) {
            unsafe {
                nabi_mobile_set_language(code.as_ptr());
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = language;
    if let Some(cache) = CACHE.get() {
        *cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
fn native_languages() -> Vec<String> {
    unsafe extern "C" {
        fn nabi_locale_languages(buffer: *mut u8, capacity: usize) -> usize;
    }
    let size = unsafe { nabi_locale_languages(std::ptr::null_mut(), 0) };
    if size == 0 || size > 64 * 1024 {
        return Vec::new();
    }
    let mut buffer = vec![0_u8; size];
    let actual = unsafe { nabi_locale_languages(buffer.as_mut_ptr(), buffer.len()) };
    if actual == 0 || actual > buffer.len() {
        return Vec::new();
    }
    String::from_utf8_lossy(&buffer[..actual - 1])
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(windows)]
fn native_languages() -> Vec<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserPreferredUILanguages(
            flags: u32,
            count: *mut u32,
            buffer: *mut u16,
            size: *mut u32,
        ) -> i32;
    }
    let (mut count, mut size) = (0, 0);
    if unsafe { GetUserPreferredUILanguages(8, &mut count, std::ptr::null_mut(), &mut size) } == 0
        || size > 64 * 1024
    {
        return Vec::new();
    }
    let mut buffer = vec![0_u16; size as usize];
    if unsafe { GetUserPreferredUILanguages(8, &mut count, buffer.as_mut_ptr(), &mut size) } == 0 {
        return Vec::new();
    }
    buffer
        .split(|c| *c == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

#[cfg(not(any(windows, target_os = "macos", target_os = "ios", target_os = "android")))]
fn native_languages() -> Vec<String> {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|s| !s.is_empty()));
    if locale
        .as_deref()
        .is_some_and(|s| matches!(s.split('.').next(), Some("C" | "POSIX")))
    {
        return vec!["en".into()];
    }
    let mut languages = std::env::var("LANGUAGE")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    languages.extend(locale);
    languages
}

/// Android 13 owns per-app language choices, including changes made in OS Settings.
/// Older devices continue to use the portable preference stored in settings.json.
#[cfg(target_os = "android")]
pub fn restore_preference(persisted: &Language) -> Language {
    if matches!(persisted, Language::Other(_)) {
        return persisted.clone();
    }
    unsafe extern "C" {
        fn nabi_mobile_saved_language(
            persisted: *const std::ffi::c_char,
            buffer: *mut u8,
            capacity: usize,
        ) -> usize;
    }
    let Ok(code) = std::ffi::CString::new(persisted.code()) else {
        return persisted.clone();
    };
    let mut buffer = [0_u8; 512];
    let size =
        unsafe { nabi_mobile_saved_language(code.as_ptr(), buffer.as_mut_ptr(), buffer.len()) };
    if size == 0 || size > buffer.len() {
        return persisted.clone();
    }
    let Ok(tag) = std::str::from_utf8(&buffer[..size - 1]) else {
        return persisted.clone();
    };
    if tag == "system" {
        Language::System
    } else {
        crate::i18n::resolve(&[tag.to_owned()])
    }
}
