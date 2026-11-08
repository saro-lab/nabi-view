//! Find installed fallback fonts without blocking the first image frame.
use eframe::egui;
use std::{
    collections::HashSet,
    sync::{Arc, OnceLock},
};

pub fn install(ctx: egui::Context) {
    std::thread::spawn(move || {
        static FONTS: OnceLock<egui::FontDefinitions> = OnceLock::new();
        let fonts = FONTS.get_or_init(system_fonts).clone();
        ctx.set_fonts(fonts);
        ctx.request_repaint();
    });
}

fn system_fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    #[cfg(target_os = "android")]
    for directory in ["/system/fonts", "/product/fonts"] {
        database.load_fonts_dir(directory);
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    for path in super::mobile::font_paths() {
        let _ = database.load_font_file(path);
    }
    let mut faces: Vec<_> = database
        .faces()
        .filter(|face| {
            !face.families.iter().any(|(name, _)| {
                name.to_ascii_lowercase()
                    .replace(" ", "")
                    .contains("lastresort")
            })
        })
        .collect();
    faces.sort_by_key(|face| {
        let family = face.families.first().map_or("", |(name, _)| name.as_str());
        (
            face.style != fontdb::Style::Normal,
            face.weight != fontdb::Weight::NORMAL,
            !(family.starts_with("Noto Sans")
                || family.starts_with(".SF")
                || family.starts_with("Geeza")),
            family.to_owned(),
        )
    });
    let mut selected = HashSet::new();
    // Whole-script probes avoid choosing a font that only contains a combining mark.
    for probe in [
        "العربيةفارسیاردو",
        "עברית",
        "हिन्दी",
        "বাংলা",
        "தமிழ்",
        "ไทย",
        "한국어",
        "日本語",
        "简体中文",
        "繁體中文",
        "Tiếng Việt",
        "Українська",
        "Ελληνικά",
    ] {
        for face in &faces {
            let usable = database
                .with_face_data(face.id, |data, index| {
                    let Ok(font) = ttf_parser::Face::parse(data, index) else {
                        return false;
                    };
                    probe
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .all(|c| font.glyph_index(c).is_some())
                        && ab_glyph::FontRef::try_from_slice_and_index(data, index).is_ok()
                })
                .unwrap_or(false);
            if !usable {
                continue;
            }
            #[cfg(test)]
            if std::env::var_os("NABI_TEXT_PREVIEW").is_some() {
                println!("{probe}: {:?}", face.families);
            }
            if selected.insert(face.id)
                && let Some(data) = database.with_face_data(face.id, |data, index| {
                    let mut font = egui::FontData::from_owned(data.to_vec());
                    font.index = index;
                    font
                })
            {
                let name = format!("system-fallback-{}", selected.len());
                fonts.font_data.insert(name.clone(), Arc::new(data));
                for family in fonts.families.values_mut() {
                    family.push(name.clone());
                }
            }
            break;
        }
    }
    fonts
}

#[cfg(test)]
mod tests;
