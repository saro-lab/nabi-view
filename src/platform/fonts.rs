//! Use the platform's UI fonts without blocking the first image frame.
use eframe::egui;
use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

/// Han ideographs differ by region, so the UI language's CJK font goes first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Han {
    Korean,
    Japanese,
    Simplified,
    Traditional,
}

impl Han {
    fn for_language(language: &crate::i18n::Language) -> Option<Self> {
        match language.code() {
            "ko" => Some(Self::Korean),
            "ja" => Some(Self::Japanese),
            "zh-Hans" => Some(Self::Simplified),
            "zh-Hant" => Some(Self::Traditional),
            _ => None,
        }
    }

    fn key(first: Option<Self>) -> u8 {
        first.map_or(1, |han| han as u8 + 2)
    }
}

/// 0 until `install` runs, so headless UI tests keep egui's bundled fonts.
static INSTALLED: AtomicU8 = AtomicU8::new(0);

pub fn install(ctx: egui::Context) {
    let first = Han::for_language(&crate::i18n::current());
    let key = Han::key(first);
    INSTALLED.store(key, Ordering::SeqCst);
    std::thread::spawn(move || {
        let fonts = system_fonts(first);
        // A later language change may have queued a newer font order.
        if INSTALLED.load(Ordering::SeqCst) == key {
            ctx.set_fonts(fonts);
            ctx.request_repaint();
        }
    });
}

/// Reorders CJK fonts after the UI language changes.
pub fn sync_language(ctx: &egui::Context) {
    let installed = INSTALLED.load(Ordering::SeqCst);
    if installed != 0 && installed != Han::key(Han::for_language(&crate::i18n::current())) {
        install(ctx.clone());
    }
}

fn database() -> &'static fontdb::Database {
    static DATABASE: OnceLock<fontdb::Database> = OnceLock::new();
    DATABASE.get_or_init(|| {
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
        database
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Ui,
    Monospace,
    Arabic,
    Hebrew,
    Devanagari,
    Bengali,
    Tamil,
    Thai,
    Han(Han),
    Latin,
    Cyrillic,
    Greek,
}

impl Role {
    /// Whole-script probes avoid choosing a font that only contains a combining mark.
    fn probe(self) -> &'static str {
        match self {
            Self::Ui => "Aa",
            Self::Monospace => "Aa0",
            Self::Arabic => "العربيةفارسیاردو",
            Self::Hebrew => "עברית",
            Self::Devanagari => "हिन्दी",
            Self::Bengali => "বাংলা",
            Self::Tamil => "தமிழ்",
            Self::Thai => "ไทย",
            Self::Han(Han::Korean) => "한국어",
            Self::Han(Han::Japanese) => "日本語",
            Self::Han(Han::Simplified) => "简体中文",
            Self::Han(Han::Traditional) => "繁體中文",
            Self::Latin => "Tiếng Việt",
            Self::Cyrillic => "Українська",
            Self::Greek => "Ελληνικά",
        }
    }

    /// The fonts each platform uses for its own interface, best first.
    fn preferred(self) -> &'static [&'static str] {
        #[cfg(target_os = "windows")]
        return match self {
            Self::Ui | Self::Latin | Self::Cyrillic | Self::Greek => &["Segoe UI"],
            Self::Monospace => &["Cascadia Mono", "Consolas"],
            Self::Arabic | Self::Hebrew => &["Segoe UI"],
            Self::Devanagari | Self::Bengali | Self::Tamil => &["Nirmala UI"],
            Self::Thai => &["Leelawadee UI"],
            Self::Han(Han::Korean) => &["Malgun Gothic"],
            Self::Han(Han::Japanese) => &["Yu Gothic UI", "Meiryo UI", "Meiryo"],
            Self::Han(Han::Simplified) => &["Microsoft YaHei UI", "Microsoft YaHei"],
            Self::Han(Han::Traditional) => &["Microsoft JhengHei UI", "Microsoft JhengHei"],
        };
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        return match self {
            Self::Ui | Self::Latin | Self::Cyrillic | Self::Greek => &[
                ".SF NS",
                "SF Pro",
                "SF Pro Text",
                ".SF UI Text",
                "Helvetica Neue",
            ],
            Self::Monospace => &["SF Mono", "Menlo"],
            Self::Arabic => &["SF Arabic", "Geeza Pro"],
            Self::Hebrew => &["SF Hebrew", "Arial Hebrew"],
            Self::Devanagari => &["Kohinoor Devanagari", "Devanagari Sangam MN"],
            Self::Bengali => &["Kohinoor Bangla", "Bangla Sangam MN"],
            Self::Tamil => &["Tamil Sangam MN"],
            Self::Thai => &["Thonburi"],
            Self::Han(Han::Korean) => &["Apple SD Gothic Neo"],
            Self::Han(Han::Japanese) => &["Hiragino Sans", "Hiragino Kaku Gothic ProN"],
            Self::Han(Han::Simplified) => &["PingFang SC", "Hiragino Sans GB", "Heiti SC"],
            Self::Han(Han::Traditional) => &["PingFang TC", "Heiti TC"],
        };
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios")))]
        return match self {
            Self::Ui | Self::Latin | Self::Cyrillic | Self::Greek => {
                &["Roboto", "Noto Sans", "Cantarell", "Ubuntu", "DejaVu Sans"]
            }
            Self::Monospace => &["Noto Sans Mono", "DejaVu Sans Mono", "Droid Sans Mono"],
            Self::Arabic => &["Noto Sans Arabic", "Noto Naskh Arabic"],
            Self::Hebrew => &["Noto Sans Hebrew"],
            Self::Devanagari => &["Noto Sans Devanagari"],
            Self::Bengali => &["Noto Sans Bengali"],
            Self::Tamil => &["Noto Sans Tamil"],
            Self::Thai => &["Noto Sans Thai"],
            Self::Han(Han::Korean) => &["Noto Sans CJK KR", "Noto Sans KR", "NanumGothic"],
            Self::Han(Han::Japanese) => &["Noto Sans CJK JP", "Noto Sans JP"],
            Self::Han(Han::Simplified) => &["Noto Sans CJK SC", "Noto Sans SC"],
            Self::Han(Han::Traditional) => &["Noto Sans CJK TC", "Noto Sans TC"],
        };
    }
}

fn fallback_roles(first: Option<Han>) -> Vec<Role> {
    let mut han = vec![
        Han::Korean,
        Han::Japanese,
        Han::Simplified,
        Han::Traditional,
    ];
    if let Some(first) = first {
        han.retain(|item| *item != first);
        han.insert(0, first);
    }
    let mut roles = vec![
        Role::Arabic,
        Role::Hebrew,
        Role::Devanagari,
        Role::Bengali,
        Role::Tamil,
        Role::Thai,
    ];
    roles.extend(han.into_iter().map(Role::Han));
    roles.extend([Role::Latin, Role::Cyrillic, Role::Greek]);
    roles
}

fn usable(database: &fontdb::Database, face: &fontdb::FaceInfo, probe: &str) -> bool {
    database
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
        .unwrap_or(false)
}

/// Upright regular faces first, then any other weight of the same family.
fn style_rank(face: &fontdb::FaceInfo) -> (bool, u16) {
    (
        face.style != fontdb::Style::Normal,
        face.weight.0.abs_diff(fontdb::Weight::NORMAL.0),
    )
}

fn has_family(face: &fontdb::FaceInfo, family: &str) -> bool {
    face.families
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(family))
}

/// Picks the platform's preferred face for `role`, else any installed face that covers it.
fn choose(
    database: &fontdb::Database,
    generic: &[&fontdb::FaceInfo],
    role: Role,
) -> Option<fontdb::ID> {
    let probe = role.probe();
    for family in role.preferred() {
        let mut faces: Vec<_> = database
            .faces()
            .filter(|face| has_family(face, family))
            .collect();
        faces.sort_by_key(|face| style_rank(face));
        if let Some(face) = faces.into_iter().find(|face| usable(database, face, probe)) {
            return Some(face.id);
        }
    }
    // Without a known UI face, egui's bundled Ubuntu/Hack stay primary.
    if matches!(role, Role::Ui | Role::Monospace) {
        return None;
    }
    generic
        .iter()
        .find(|face| usable(database, face, probe))
        .map(|face| face.id)
}

fn font_data(database: &fontdb::Database, id: fontdb::ID) -> Option<egui::FontData> {
    database.with_face_data(id, |data, index| {
        let mut font = egui::FontData::from_owned(data.to_vec());
        font.index = index;
        font
    })
}

fn system_fonts(first: Option<Han>) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let database = database();
    let mut generic: Vec<_> = database
        .faces()
        .filter(|face| {
            !face.families.iter().any(|(name, _)| {
                name.to_ascii_lowercase()
                    .replace(" ", "")
                    .contains("lastresort")
            })
        })
        .collect();
    generic.sort_by_key(|face| {
        let family = face.families.first().map_or("", |(name, _)| name.as_str());
        (
            style_rank(face),
            !(family.starts_with("Noto Sans")
                || family.starts_with(".SF")
                || family.starts_with("Geeza")),
            family.to_owned(),
        )
    });
    // Each face is loaded once; a UI face that also covers a script is reused as its fallback.
    let mut loaded = HashMap::new();
    let mut load = |fonts: &mut egui::FontDefinitions, id| -> Option<String> {
        if let Some(name) = loaded.get(&id) {
            return Some(String::clone(name));
        }
        let data = font_data(database, id)?;
        let name = format!("system-{}", loaded.len() + 1);
        fonts.font_data.insert(name.clone(), Arc::new(data));
        loaded.insert(id, name.clone());
        Some(name)
    };
    // The system UI faces replace egui's bundled Ubuntu/Hack as the primary fonts.
    for (role, family) in [
        (Role::Ui, egui::FontFamily::Proportional),
        (Role::Monospace, egui::FontFamily::Monospace),
    ] {
        if let Some(name) = choose(database, &generic, role).and_then(|id| load(&mut fonts, id)) {
            fonts.families.entry(family).or_default().insert(0, name);
        }
    }
    for role in fallback_roles(first) {
        let Some(id) = choose(database, &generic, role) else {
            continue;
        };
        #[cfg(test)]
        if std::env::var_os("NABI_TEXT_PREVIEW").is_some() {
            println!(
                "{role:?}: {:?}",
                database.face(id).map(|face| &face.families)
            );
        }
        let Some(name) = load(&mut fonts, id) else {
            continue;
        };
        for family in fonts.families.values_mut() {
            if !family.contains(&name) {
                family.push(name.clone());
            }
        }
    }
    fonts
}

#[cfg(test)]
mod tests;
