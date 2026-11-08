//! Match ordered OS language preferences without OS calls.
use super::Language;
pub fn resolve(preferred: &[String]) -> Language {
    preferred
        .iter()
        .find_map(|tag| match_tag(tag))
        .unwrap_or(Language::English)
}
fn match_tag(tag: &str) -> Option<Language> {
    let tag = tag
        .trim()
        .split(['.', '@'])
        .next()?
        .replace('_', "-")
        .to_ascii_lowercase();
    let parts: Vec<_> = tag.split('-').collect();
    let language = match *parts.first()? {
        "iw" => "he",
        "in" => "id",
        other => other,
    };
    if let Some(script) = parts
        .iter()
        .skip(1)
        .take_while(|part| part.len() != 1)
        .find(|part| part.len() == 4)
    {
        let expected = match language {
            "zh" => None,
            "ru" | "uk" => Some("cyrl"),
            "ar" | "fa" | "ur" => Some("arab"),
            "he" => Some("hebr"),
            "hi" => Some("deva"),
            "bn" => Some("beng"),
            "ta" => Some("taml"),
            "th" => Some("thai"),
            "el" => Some("grek"),
            "ko" => Some("kore"),
            "ja" => Some("jpan"),
            _ => Some("latn"),
        };
        if expected.is_some_and(|expected| *script != expected)
            || (language == "zh" && !matches!(*script, "hans" | "hant"))
        {
            return None;
        }
    }
    let code = match language {
        "zh" => {
            if parts.contains(&"hant") {
                "zh-Hant"
            } else if parts.contains(&"hans") {
                "zh-Hans"
            } else if parts.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) {
                "zh-Hant"
            } else {
                "zh-Hans"
            }
        }
        "pt" => {
            if parts
                .iter()
                .skip(1)
                .any(|part| matches!(*part, "pt" | "ao" | "mz" | "cv" | "gw" | "st" | "tl"))
            {
                "pt-PT"
            } else {
                "pt-BR"
            }
        }
        language => language,
    };
    let value = Language::from_code(code);
    (!matches!(value, Language::Other(_) | Language::System)).then_some(value)
}
