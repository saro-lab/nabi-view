use super::*;

#[test]
fn multilingual_layout_keeps_logical_text_and_shapes_clusters() {
    let fonts = system_fonts();
    let ctx = egui::Context::default();
    ctx.set_fonts(fonts);
    let mut previews = Vec::new();
    let _ = ctx.run(Default::default(), |ctx| {
        for text in [
            "العربية 123 English",
            "עברית / photo.png",
            "हिन्दी",
            "বাংলা",
            "தமிழ்",
            "ไทย",
            "한국어 日本語 简体中文 繁體中文",
        ] {
            let galley = ctx.fonts(|fonts| {
                fonts.layout_no_wrap(
                    text.to_owned(),
                    egui::FontId::proportional(24.),
                    egui::Color32::WHITE,
                )
            });
            previews.push(galley.clone());
            assert_eq!(galley.text(), text);
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
        let galley = ctx.fonts(|fonts| {
            fonts.layout_no_wrap(
                text.to_owned(),
                egui::FontId::proportional(24.),
                egui::Color32::WHITE,
            )
        });
        let row = &galley.rows[0];
        assert!(row.glyphs.first().unwrap().pos.x > row.glyphs.last().unwrap().pos.x);
        assert_eq!(row.char_at(row.x_offset(0)), 0);
        assert_eq!(
            row.char_at(row.x_offset(row.glyphs.len())),
            row.glyphs.len()
        );
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
                            let alpha = atlas.pixels[v * atlas.size[0] + u].clamp(0., 1.);
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
    ctx.set_fonts(system_fonts());
    let keys: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!(
        concat!(env!("CARGO_MANIFEST_DIR"), "/locales/en.json")
    ))
    .unwrap();
    let _ = ctx.run(Default::default(), |ctx| {
        for language in crate::i18n::Language::ALL.iter().skip(1) {
            crate::i18n::set_language(language.clone());
            let text = keys
                .keys()
                .map(|key| crate::i18n::tr(key))
                .chain(std::iter::once(language.native_name()))
                .collect::<String>();
            let chars: std::collections::BTreeSet<_> =
                text.chars().filter(|c| !c.is_whitespace()).collect();
            let missing: String = ctx.fonts(|fonts| {
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
fn bidi_positions_preserve_numbers_and_shaped_cluster_boundaries() {
    let ctx = egui::Context::default();
    ctx.set_fonts(system_fonts());
    let _ = ctx.run(Default::default(), |ctx| {
        let font_id = egui::FontId::proportional(20.);
        let text = "abc אבג 123";
        let galley = ctx.fonts(|fonts| {
            fonts.layout_no_wrap(text.to_owned(), font_id.clone(), egui::Color32::WHITE)
        });
        let mut visual = galley.rows[0].glyphs.clone();
        visual.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
        assert_eq!(
            visual.iter().map(|g| g.chr).collect::<String>(),
            "abc 123 גבא"
        );
        for text in ["क्षि क्षि क्षि", "لا لا لا"] {
            if !ctx.fonts(|fonts| fonts.has_glyphs(&font_id, text)) {
                continue;
            }
            let galley = ctx.fonts(|fonts| {
                fonts.layout_no_wrap(text.to_owned(), font_id.clone(), egui::Color32::WHITE)
            });
            assert!(
                galley.rows[0]
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
                let wrapped = ctx.fonts(|fonts| fonts.layout_job(job.clone()));
                assert!(
                    wrapped
                        .rows
                        .iter()
                        .all(|row| row.glyphs.first().is_none_or(|glyph| glyph.cluster_start))
                );
                job.wrap.max_rows = 1;
                let elided = ctx.fonts(|fonts| fonts.layout_job(job));
                assert!(elided.rows.iter().all(|row| {
                    row.glyphs
                        .last()
                        .is_none_or(|glyph| glyph.chr != '…' || glyph.cluster_start)
                }));
            }
        }
    });
}
