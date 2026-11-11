use super::*;
use egui::text::CharIndex;

#[test]
fn multilingual_layout_keeps_logical_text_and_shapes_clusters() {
    let fonts = system_fonts(None);
    let ctx = egui::Context::default();
    ctx.set_fonts(fonts);
    let mut previews = Vec::new();
    let _ = ctx.run_pass(Default::default(), |ctx| {
        for text in [
            "العربية 123 English",
            "עברית / photo.png",
            "हिन्दी",
            "বাংলা",
            "தமிழ்",
            "ไทย",
            "한국어 日本語 简体中文 繁體中文",
        ] {
            let galley = ctx.fonts_mut(|fonts| {
                fonts.layout_no_wrap(
                    text.to_owned(),
                    egui::FontId::proportional(24.),
                    egui::Color32::WHITE,
                )
            });
            previews.push(galley.clone());
            assert_eq!(galley.text(), text);
            assert_eq!(
                galley.rows.iter().map(|row| row.text()).collect::<String>(),
                text
            );
            assert_eq!(
                galley.rows.iter().map(|r| r.glyphs.len()).sum::<usize>(),
                text.chars().count()
            );
            assert!(galley.size().x > 0. && galley.size().y > 0.);
            assert!(
                galley
                    .rows
                    .iter()
                    .all(|row| row.glyphs.iter().all(|g| g.pos.x.is_finite()))
            );
        }
        let text = "עברית";
        let galley = ctx.fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                text.to_owned(),
                egui::FontId::proportional(24.),
                egui::Color32::WHITE,
            )
        });
        let row = &galley.rows[0];
        assert!(row.glyphs.first().unwrap().pos.x > row.glyphs.last().unwrap().pos.x);
        assert_eq!(row.char_at(row.x_offset(CharIndex(0))), CharIndex(0));
        let end = CharIndex(text.chars().count());
        assert_eq!(row.char_count_excluding_newline(), end);
        assert_eq!(row.char_at(row.x_offset(end)), end);
    });
    if std::env::var_os("NABI_TEXT_PREVIEW").is_some() {
        let atlas = ctx.fonts(|fonts| fonts.image());
        let mut image = image::RgbImage::from_pixel(1200, 7 * 72, image::Rgb([21, 23, 25]));
        for (row, galley) in previews.iter().enumerate() {
            for line in &galley.rows {
                for quad in line.visuals.mesh.vertices.as_chunks::<4>().0 {
                    let left = quad.iter().map(|v| v.pos.x).fold(f32::INFINITY, f32::min);
                    let top = quad.iter().map(|v| v.pos.y).fold(f32::INFINITY, f32::min);
                    let right = quad
                        .iter()
                        .map(|v| v.pos.x)
                        .fold(f32::NEG_INFINITY, f32::max);
                    let bottom = quad
                        .iter()
                        .map(|v| v.pos.y)
                        .fold(f32::NEG_INFINITY, f32::max);
                    let u0 = quad.iter().map(|v| v.uv.x).fold(f32::INFINITY, f32::min);
                    let v0 = quad.iter().map(|v| v.uv.y).fold(f32::INFINITY, f32::min);
                    let u1 = quad
                        .iter()
                        .map(|v| v.uv.x)
                        .fold(f32::NEG_INFINITY, f32::max);
                    let v1 = quad
                        .iter()
                        .map(|v| v.uv.y)
                        .fold(f32::NEG_INFINITY, f32::max);
                    for y in top.floor() as i32..bottom.ceil() as i32 {
                        for x in left.floor() as i32..right.ceil() as i32 {
                            let px = x + 20;
                            let py = y + 20 + row as i32 * 72;
                            if px < 0 || py < 0 || px >= 1200 || py >= 7 * 72 {
                                continue;
                            }
                            let u = (u0 + (x as f32 + 0.5 - left) / (right - left) * (u1 - u0))
                                .floor()
                                .max(0.) as usize;
                            let v = (v0 + (y as f32 + 0.5 - top) / (bottom - top) * (v1 - v0))
                                .floor()
                                .max(0.) as usize;
                            if u >= atlas.size[0] || v >= atlas.size[1] {
                                continue;
                            }
                            let alpha = f32::from(atlas.pixels[v * atlas.size[0] + u].a()) / 255.0;
                            let old = image.get_pixel(px as u32, py as u32).0;
                            image.put_pixel(
                                px as u32,
                                py as u32,
                                image::Rgb(
                                    old.map(|c| (c as f32 * (1. - alpha) + 240. * alpha) as u8),
                                ),
                            );
                        }
                    }
                }
            }
        }
        let path = crate::test_support::root().join("multilingual-text.png");
        image.save(&path).unwrap();
        println!("{}", path.display());
    }
}

#[test]
#[ignore = "Requires system fonts for all supported scripts; run on the native desktop for font coverage QA"]
fn installed_fonts_cover_every_translated_ui_character() {
    let ctx = egui::Context::default();
    ctx.set_fonts(system_fonts(None));
    let keys: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!(
        concat!(env!("CARGO_MANIFEST_DIR"), "/locales/en.json")
    ))
    .unwrap();
    let _ = ctx.run_pass(Default::default(), |ctx| {
        for language in crate::i18n::Language::ALL.iter().skip(1) {
            crate::i18n::set_language(language.clone());
            let text = keys
                .keys()
                .map(|key| crate::i18n::tr(key))
                .chain(std::iter::once(language.native_name()))
                .collect::<String>();
            let chars: std::collections::BTreeSet<_> =
                text.chars().filter(|c| !c.is_whitespace()).collect();
            let missing: String = ctx.fonts_mut(|fonts| {
                chars
                    .into_iter()
                    .filter(|c| !fonts.has_glyph(&egui::FontId::proportional(16.), *c))
                    .collect()
            });
            assert!(
                missing.is_empty(),
                "Missing {} glyphs: {missing}",
                language.code()
            );
        }
    });
}

#[test]
fn installed_platform_ui_fonts_win_over_alphabetical_fallbacks() {
    let database = database();
    let generic: Vec<_> = database.faces().collect();
    let roles = [Role::Ui, Role::Monospace]
        .into_iter()
        .chain(fallback_roles(None));
    for role in roles {
        let Some(installed) = role.preferred().iter().find(|family| {
            database
                .faces()
                .any(|face| has_family(face, family) && usable(database, face, role.probe()))
        }) else {
            continue;
        };
        let face = choose(database, &generic, role)
            .and_then(|id| database.face(id))
            .unwrap();
        assert!(
            has_family(face, installed),
            "{role:?} chose {:?} instead of {installed}",
            face.families
        );
        assert_eq!(face.style, fontdb::Style::Normal, "{role:?}");
    }
}

#[test]
fn ui_language_puts_its_han_font_first() {
    let order = |first| {
        fallback_roles(first)
            .into_iter()
            .filter_map(|role| match role {
                Role::Han(han) => Some(han),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(order(None)[0], Han::Korean);
    for han in [
        Han::Korean,
        Han::Japanese,
        Han::Simplified,
        Han::Traditional,
    ] {
        let mut expected = order(None);
        expected.retain(|item| *item != han);
        expected.insert(0, han);
        assert_eq!(order(Some(han)), expected);
    }
    for (code, han) in [
        ("ko", Han::Korean),
        ("ja", Han::Japanese),
        ("zh-Hans", Han::Simplified),
        ("zh-Hant", Han::Traditional),
    ] {
        let language = crate::i18n::Language::ALL
            .iter()
            .find(|language| language.code() == code)
            .unwrap();
        assert_eq!(Han::for_language(language), Some(han));
    }
}

#[test]
fn bidi_positions_preserve_numbers_and_shaped_cluster_boundaries() {
    let ctx = egui::Context::default();
    ctx.set_fonts(system_fonts(None));
    let _ = ctx.run_pass(Default::default(), |ctx| {
        let font_id = egui::FontId::proportional(20.);
        let text = "abc אבג 123";
        let galley = ctx.fonts_mut(|fonts| {
            fonts.layout_no_wrap(text.to_owned(), font_id.clone(), egui::Color32::WHITE)
        });
        let mut visual = galley.rows[0].glyphs.clone();
        visual.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
        assert_eq!(
            visual.iter().map(|g| g.chr).collect::<String>(),
            "abc 123 גבא"
        );
        for text in ["क्षि क्षि क्षि", "لا لا لا"] {
            if !ctx.fonts_mut(|fonts| fonts.has_glyphs(&font_id, text)) {
                continue;
            }
            let galley = ctx.fonts_mut(|fonts| {
                fonts.layout_no_wrap(text.to_owned(), font_id.clone(), egui::Color32::WHITE)
            });
            // Segoe UI draws lam-alef as two contextual glyphs rather than one ligature,
            // so only the Devanagari conjunct must form a multi-character cluster.
            assert!(
                text.starts_with('ل')
                    || galley.rows[0]
                        .glyphs
                        .iter()
                        .any(|glyph| !glyph.cluster_start),
                "{text} was not shaped into clusters"
            );
            for width in [1., 18., 40., 100.] {
                let mut job = egui::text::LayoutJob::simple(
                    text.to_owned(),
                    font_id.clone(),
                    egui::Color32::WHITE,
                    width,
                );
                let wrapped = ctx.fonts_mut(|fonts| fonts.layout_job(job.clone()));
                assert!(
                    wrapped
                        .rows
                        .iter()
                        .all(|row| row.glyphs.first().is_none_or(|glyph| glyph.cluster_start))
                );
                job.wrap.max_rows = 1;
                let elided = ctx.fonts_mut(|fonts| fonts.layout_job(job));
                assert!(elided.rows.iter().all(|row| {
                    row.glyphs
                        .last()
                        .is_none_or(|glyph| glyph.chr != '…' || glyph.cluster_start)
                }));
            }
        }
    });
}
