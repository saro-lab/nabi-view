//! UI language selection and immutable, embedded message catalogs.
mod catalogs;
mod language;
mod resolve;
#[cfg(test)]
mod tests;
pub use language::Language;
pub(crate) use resolve::resolve;
use std::{cell::Cell, collections::BTreeMap, sync::OnceLock};
static CATALOGS: [OnceLock<BTreeMap<String, String>>; catalogs::SOURCES.len()] =
    [const { OnceLock::new() }; catalogs::SOURCES.len()];
thread_local! { static CURRENT: Cell<usize> = const { Cell::new(0) }; }
fn catalog(index: usize) -> &'static BTreeMap<String, String> {
    CATALOGS[index].get_or_init(|| {
        serde_json::from_str(catalogs::SOURCES[index]).expect("validated locale catalog")
    })
}
pub fn current() -> Language {
    Language::ALL[CURRENT.get() + 1].clone()
}
pub fn is_rtl() -> bool {
    current().is_rtl()
}
pub fn set_language(preference: Language) {
    let locale = match preference {
        Language::System => resolve(&crate::platform::locale::preferred_languages()),
        Language::Other(_) => Language::English,
        language => language,
    };
    CURRENT.set(
        Language::ALL[1..]
            .iter()
            .position(|item| *item == locale)
            .unwrap_or(0),
    );
}
#[cfg(test)]
fn message_id(key: &str) -> &str {
    static ALIASES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    ALIASES
        .get_or_init(|| {
            serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/i18n/test_aliases.json"
            )))
            .expect("message aliases")
        })
        .get(key)
        .map(String::as_str)
        .unwrap_or(key)
}
pub fn tr(key: &str) -> &str {
    let mut key = key;
    if key == "reveal-in-file-manager" {
        if cfg!(target_os = "macos") {
            key = "show-in-finder";
        } else if cfg!(windows) {
            key = "show-in-file-explorer";
        }
    }
    catalog(CURRENT.get())
        .get(key)
        .filter(|s| !s.is_empty())
        .or_else(|| catalog(0).get(key))
        .map(String::as_str)
        .unwrap_or(key)
}
/// Named values are inserted once and never interpreted as message syntax.
pub fn format(key: &str, arguments: &[(&str, &str)]) -> String {
    let message = tr(key);
    let mut result = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start + 1..].find('}') else {
            result.push_str(&rest[start..]);
            return result;
        };
        let name = &rest[start + 1..start + 1 + end];
        if let Some((_, value)) = arguments.iter().find(|(key, _)| *key == name) {
            // Keep paths, numbers and native language names together in RTL sentences.
            // Source metadata stays untouched; this affects display templates only.
            if is_rtl() && name != "label" {
                result.push('\u{2068}');
            }
            result.push_str(value);
            if is_rtl() && name != "label" {
                result.push('\u{2069}');
            }
        } else {
            result.push_str(&rest[start..start + end + 2]);
        }
        rest = &rest[start + end + 2..];
    }
    result.push_str(rest);
    result
}

pub fn value_label(label: &str, value: &str) -> String {
    format("value-label", &[("label", tr(label)), ("value", value)])
}

#[cfg(test)]
pub(crate) fn test_tr(key: &str) -> &str {
    tr(message_id(key))
}
