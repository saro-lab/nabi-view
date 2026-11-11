//! Nabi View's logical-character and Unicode bidi adapter around upstream shaping.
use super::*;

/// Merge raster parts before distributing the advance across source characters.
/// The backing string and glyph slots stay in logical order, including ligatures.
#[expect(clippy::too_many_arguments)]
pub(super) fn finish_cluster(
    fonts: &mut FontsImpl,
    ctx: &ShapingContext,
    paragraph: &mut Paragraph,
    text: &str,
    bytes: Range<usize>,
    glyph_count: usize,
    metrics: &StyledMetrics,
    origin_px: f32,
    rtl: bool,
) {
    let Some(text) = text.get(bytes) else {
        return;
    };
    let count = text.chars().count();
    if count == 0 {
        return;
    }
    let start = paragraph.glyphs.len().saturating_sub(glyph_count);
    let origin = origin_px / ctx.pixels_per_point;
    let allocation =
        fonts.compose_cluster(&paragraph.glyphs[start..], ctx.pixels_per_point, origin);
    let width = (paragraph.cursor_x_px - origin_px) / ctx.pixels_per_point;
    let advance = width / count as f32;
    paragraph.glyphs.truncate(start);
    for (index, chr) in text.chars().enumerate() {
        let mut glyph = ctx.glyph(
            chr,
            0,
            advance * ctx.pixels_per_point,
            metrics,
            if index == 0 {
                allocation
            } else {
                GlyphAllocation::default()
            },
        );
        glyph.pos.x = origin + index as f32 * advance;
        glyph.rtl = rtl;
        glyph.cluster_start = index == 0;
        if rtl && index == 0 {
            // After visual reordering the first logical char is at the right.
            glyph.uv_rect.offset.x -= width - advance;
        }
        paragraph.glyphs.push(glyph);
    }
}

pub(super) fn reorder_rows(rows: &mut [PlacedRow], job: &LayoutJob) {
    if job.text.is_ascii() {
        return;
    }
    let bidi = unicode_bidi::BidiInfo::new(&job.text, None);
    if !bidi.has_rtl() {
        return;
    }
    let mut byte_start = 0;
    for placed in rows {
        let row = Arc::make_mut(&mut placed.row);
        let visible: String = row.glyphs.iter().map(|glyph| glyph.chr).collect();
        let end = (byte_start + visible.len()).min(job.text.len());
        if let Some(para) = bidi
            .paragraphs
            .iter()
            .find(|p| p.range.start <= byte_start && byte_start < p.range.end)
        {
            let levels: Vec<_> = if job.text.get(byte_start..end) == Some(visible.as_str()) {
                let levels = bidi.reordered_levels(para, byte_start..end);
                visible
                    .char_indices()
                    .map(|(index, _)| levels[byte_start + index])
                    .collect()
            } else {
                // Elision inserts an overflow glyph; resolve that visible text.
                let line = unicode_bidi::BidiInfo::new(&visible, Some(para.level));
                visible
                    .char_indices()
                    .map(|(index, _)| line.levels[index])
                    .collect()
            };
            let order = unicode_bidi::BidiInfo::reorder_visual(&levels);
            // Preserve letter spacing/justification as well as glyph advances.
            let widths: Vec<_> = row
                .glyphs
                .iter()
                .enumerate()
                .map(|(i, glyph)| {
                    row.glyphs
                        .get(i + 1)
                        .map_or(glyph.advance_width, |next| next.pos.x - glyph.pos.x)
                        .max(0.0)
                })
                .collect();
            let mut x = row.glyphs.first().map_or(0.0, |glyph| glyph.pos.x);
            for index in order {
                let glyph = &mut row.glyphs[index];
                glyph.rtl = levels[index].is_rtl();
                glyph.pos.x = x;
                x += widths[index];
            }
        }
        byte_start = end + usize::from(placed.ends_with_newline);
    }
}

/// Decorations follow physical runs while cursor/selection indices stay logical.
pub(super) fn visual_glyphs(row: &Row) -> alloc::borrow::Cow<'_, [Glyph]> {
    if row.glyphs.iter().any(|glyph| glyph.rtl) {
        let mut glyphs = row.glyphs.clone();
        glyphs.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
        alloc::borrow::Cow::Owned(glyphs)
    } else {
        alloc::borrow::Cow::Borrowed(&row.glyphs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{CharIndex, FontDefinitions, FontFamily, FontId, Fonts};

    fn fonts() -> Fonts {
        Fonts::new(crate::TextOptions::default(), FontDefinitions::empty())
    }

    #[test]
    fn bidi_retains_logical_text_and_orders_hebrew_and_numbers() {
        let mut fonts = fonts();
        let text = "abc אבג 123";
        let galley = fonts.with_pixels_per_point(1.0).layout_no_wrap(
            text.into(),
            FontId::proportional(18.0),
            Color32::WHITE,
        );
        let row = &galley.rows[0].row;
        assert_eq!(row.text(), text);
        assert_eq!(
            visual_glyphs(row).iter().map(|g| g.chr).collect::<String>(),
            "abc 123 גבא"
        );
        assert_eq!(row.glyphs.len(), text.chars().count());
    }

    #[test]
    fn rtl_caret_round_trips_in_logical_character_order() {
        let mut fonts = fonts();
        let galley = fonts.with_pixels_per_point(1.0).layout_no_wrap(
            "אבג".into(),
            FontId::proportional(18.0),
            Color32::WHITE,
        );
        let row = &galley.rows[0].row;
        assert!(row.x_offset(CharIndex(0)) > row.x_offset(CharIndex(3)));
        for index in 0..=3 {
            assert_eq!(
                row.char_at(row.x_offset(CharIndex(index))),
                CharIndex(index)
            );
        }
    }

    #[test]
    fn grapheme_clusters_survive_narrow_wrap_and_elision() {
        let mut fonts = fonts();
        let text = "a\u{301}b a\u{301}b";
        for width in [1.0, 18.0, 40.0] {
            let mut job = LayoutJob::simple(
                text.into(),
                FontId::proportional(18.0),
                Color32::WHITE,
                width,
            );
            let galley = fonts.with_pixels_per_point(1.0).layout_job(job.clone());
            assert_eq!(
                galley
                    .rows
                    .iter()
                    .map(|row| row.row.text())
                    .collect::<String>(),
                text
            );
            assert!(
                galley
                    .rows
                    .iter()
                    .all(|row| row.glyphs.first().is_none_or(|g| g.cluster_start))
            );
            job.wrap.max_rows = 1;
            let elided = fonts.with_pixels_per_point(1.0).layout_job(job);
            assert!(elided.rows.iter().all(|row| {
                row.glyphs
                    .last()
                    .is_none_or(|g| g.chr != '…' || g.cluster_start)
            }));
        }
    }

    #[test]
    fn multi_glyph_cluster_composes_color_and_keeps_one_source_slot() {
        let mut fonts = FontsImpl::new(crate::TextOptions::default(), FontDefinitions::empty());
        let metrics = StyledMetrics::default();
        let ctx = ShapingContext {
            family: fonts.family_key(&FontFamily::Proportional),
            pixels_per_point: 1.0,
            font_size: 18.0,
            line_height: 18.0,
            extra_letter_spacing: 0.0,
            section_index: 0,
            font_metrics: metrics.clone(),
            is_first_glyph_in_section: false,
            prev_cluster: Some(0),
        };
        // The atlas reserves texel (0,0) as white. Two positioned parts model a
        // single source character decomposed by the font's shaper.
        let allocation = GlyphAllocation {
            uv_rect: crate::text::glyph_atlas::UvRect {
                min: [0, 0],
                max: [1, 1],
                offset: Vec2::ZERO,
                size: Vec2::splat(1.0),
            },
            is_color: true,
        };
        let parts = vec![
            ctx.glyph('ำ', 0, 6.0, &metrics, allocation),
            ctx.glyph('ำ', 6, 6.0, &metrics, allocation),
        ];
        let cached = fonts.compose_cluster(&parts, 1.0, 0.0);
        assert!(cached.is_color);
        assert_eq!(cached.uv_rect.size, vec2(7.0, 1.0));
        assert_eq!(fonts.compose_cluster(&parts, 1.0, 0.0), cached);
        let mut paragraph = Paragraph::from_section_index(0);
        paragraph.glyphs = parts;
        paragraph.cursor_x_px = 12.0;
        finish_cluster(
            &mut fonts,
            &ctx,
            &mut paragraph,
            "ำ",
            0.."ำ".len(),
            2,
            &metrics,
            0.0,
            false,
        );
        assert_eq!(paragraph.glyphs.len(), 1);
        assert_eq!(paragraph.glyphs[0].chr, 'ำ');
        assert_eq!(paragraph.glyphs[0].advance_width, 12.0);
        assert_eq!(paragraph.glyphs[0].uv_rect, cached.uv_rect);
    }
}
