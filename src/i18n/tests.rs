use super::*;

#[test]
fn system_matching_respects_order_script_region_and_fallback() {
    for (tags, expected) in [
        (vec!["ko-KR"], Language::Korean),
        (vec!["ko_KR.UTF-8"], Language::Korean),
        (vec!["fr-CA"], Language::French),
        (vec!["xx", "ja-JP"], Language::Japanese),
        (vec!["zh-TW"], Language::ChineseTraditional),
        (vec!["zh-HK"], Language::ChineseTraditional),
        (vec!["zh-Hans-HK"], Language::ChineseSimplified),
        (vec!["zh-Hant-CN"], Language::ChineseTraditional),
        (vec!["zh-SG"], Language::ChineseSimplified),
        (vec!["pt-PT"], Language::PortuguesePortugal),
        (vec!["pt-Latn-PT"], Language::PortuguesePortugal),
        (vec!["pt-AO"], Language::PortuguesePortugal),
        (vec!["zh-Latn", "ja"], Language::Japanese),
        (vec!["ar-Latn", "fr"], Language::French),
        (vec!["pt"], Language::PortugueseBrazil),
        (vec!["iw-IL"], Language::Hebrew),
        (vec!["in-ID"], Language::Indonesian),
        (vec!["C", "unknown"], Language::English),
        (vec![], Language::English),
    ] {
        assert_eq!(
            resolve(&tags.into_iter().map(str::to_owned).collect::<Vec<_>>()),
            expected
        );
    }
}

#[test]
fn language_preferences_preserve_legacy_and_future_settings() {
    let settings = crate::core::settings::Settings::default();
    assert_eq!(settings.language, Language::System);
    let mut value = serde_json::to_value(&settings).unwrap();
    value.as_object_mut().unwrap().remove("language");
    assert_eq!(
        serde_json::from_value::<crate::core::settings::Settings>(value.clone())
            .unwrap()
            .language,
        Language::System
    );
    for code in ["system", "ko", "en", "ar", "zh-Hant", "future-Lang"] {
        value["language"] = code.into();
        let restored: crate::core::settings::Settings =
            serde_json::from_value(value.clone()).unwrap();
        assert_eq!(restored.language.code(), code);
        assert_eq!(serde_json::to_value(&restored).unwrap()["language"], code);
    }
}

#[test]
fn translation_fallback_and_named_values_do_not_reinterpret_user_text() {
    set_language(Language::Korean);
    assert_eq!(tr("settings"), "설정");
    set_language(Language::English);
    assert_eq!(tr("settings"), "Settings");
    assert_eq!(tr("external diagnostic"), "external diagnostic");
    assert_eq!(
        format("value-label", &[("label", "{value}"), ("value", "100%")]),
        "{value}: 100%"
    );
    set_language(Language::Other("future".into()));
    assert_eq!(tr("settings"), "Settings");
}

#[test]
fn ui_direction_is_independent_of_image_reading_direction() {
    let settings = crate::core::settings::Settings::default();
    for language in Language::ALL.iter().skip(1) {
        set_language(language.clone());
        assert_eq!(
            is_rtl(),
            matches!(language.code(), "ar" | "fa" | "ur" | "he")
        );
        assert!(!settings.rtl);
    }
}

#[test]
fn all_catalogs_have_valid_ids_and_matching_placeholders() {
    fn parameters(text: &str) -> std::collections::BTreeSet<&str> {
        text.split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name))
            .collect()
    }
    let english = catalog(0);
    for (index, _) in catalogs::SOURCES.iter().enumerate() {
        assert_eq!(
            catalog(index).keys().collect::<Vec<_>>(),
            english.keys().collect::<Vec<_>>(),
            "Incomplete catalog: {index}"
        );
        for (key, value) in catalog(index) {
            let source = english
                .get(key)
                .unwrap_or_else(|| panic!("Unknown translation ID: {key}"));
            assert!(!value.is_empty(), "Empty translation: {index}/{key}");
            assert_eq!(
                parameters(source),
                parameters(value),
                "Wrong parameters: {index}/{key}"
            );
        }
    }
}
