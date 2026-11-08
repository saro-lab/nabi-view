//! Local Unicode shaping extension. Logical characters remain in source order.
//! HarfRust supplies glyph clusters; UAX #9 supplies the visual order of each line.
use super::*;
use unicode_script::{Script, UnicodeScript};

pub(super) fn layout_section(
    fonts: &mut FontsImpl,
    job: &LayoutJob,
    section_index: u32,
    section: &LayoutSection,
    paragraphs: &mut Vec<Paragraph>,
) -> bool {
    let text = &job.text[section.byte_range.clone()];
    if text.is_ascii() {
        return false;
    }
    let font = fonts.font(&section.format.font_id);
    let line_height = section
        .format
        .line_height
        .unwrap_or_else(|| font.row_height());
    let paragraph = paragraphs.last_mut().unwrap();
    paragraph.cursor_x += section.leading_space;
    if paragraph.glyphs.is_empty() {
        paragraph.empty_paragraph_height = line_height;
    }
    let lines: Vec<&str> = if job.break_on_newline {
        text.split('\n').collect()
    } else {
        vec![text]
    };
    for (line_index, text) in lines.into_iter().enumerate() {
        if line_index != 0 {
            let mut paragraph = Paragraph::from_section_index(section_index);
            paragraph.empty_paragraph_height = line_height;
            paragraphs.push(paragraph);
        }
        let paragraph = paragraphs.last_mut().unwrap();
        let bidi = unicode_bidi::BidiInfo::new(text, None);
        let mut runs: Vec<(
            usize,
            usize,
            Option<Arc<crate::text::font::FontImpl>>,
            bool,
            Script,
        )> = Vec::new();
        for (start, chr) in text.char_indices() {
            let face = font.face_for_char(chr);
            let rtl = bidi.levels.get(start).is_some_and(|level| level.is_rtl());
            let mut script = chr.script();
            if matches!(script, Script::Common | Script::Inherited) {
                script = runs.last().map_or(script, |run| run.4);
            }
            if let Some(last) = runs.last_mut() {
                let same_face = match (&last.2, &face) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                };
                if same_face
                    && rtl == last.3
                    && (script == last.4 || matches!(last.4, Script::Common | Script::Inherited))
                {
                    last.1 = start + chr.len_utf8();
                    last.4 = script;
                    continue;
                }
            }
            runs.push((start, start + chr.len_utf8(), face, rtl, script));
        }
        for (start, end, face, rtl, _) in runs {
            let run = &text[start..end];
            if let Some(face) = face.as_ref() {
                if let Some(clusters) = face.shape(run, rtl).filter(|clusters| !clusters.is_empty())
                {
                    for cluster in clusters {
                        let chars: Vec<_> = run[cluster.range].chars().collect();
                        let count = chars.len();
                        let advance = cluster.info.advance_width / count as f32;
                        for (index, chr) in chars.into_iter().enumerate() {
                            let mut uv = if index == 0 {
                                cluster.info.uv_rect
                            } else {
                                Default::default()
                            };
                            if rtl && index == 0 {
                                uv.offset.x -= cluster.info.advance_width - advance;
                            }
                            paragraph.glyphs.push(Glyph {
                                chr,
                                rtl,
                                cluster_start: index == 0,
                                pos: pos2(paragraph.cursor_x, f32::NAN),
                                advance_width: advance,
                                line_height,
                                font_impl_height: face.row_height(),
                                font_impl_ascent: face.ascent(),
                                font_height: font.row_height(),
                                font_ascent: font.ascent(),
                                uv_rect: uv,
                                section_index,
                            });
                            paragraph.cursor_x += advance;
                        }
                        paragraph.cursor_x += section.format.extra_letter_spacing;
                    }
                    continue;
                }
            }
            // Unsupported fonts and control-only runs retain the original fallback behavior.
            for chr in run.chars() {
                let (face, info) = font.font_impl_and_glyph_info(chr);
                paragraph.glyphs.push(Glyph {
                    chr,
                    rtl,
                    cluster_start: true,
                    pos: pos2(paragraph.cursor_x, f32::NAN),
                    advance_width: info.advance_width,
                    line_height,
                    font_impl_height: face.map_or(0.0, |f| f.row_height()),
                    font_impl_ascent: face.map_or(0.0, |f| f.ascent()),
                    font_height: font.row_height(),
                    font_ascent: font.ascent(),
                    uv_rect: info.uv_rect,
                    section_index,
                });
                paragraph.cursor_x += info.advance_width;
            }
        }
    }
    true
}

pub(super) fn reorder_rows(rows: &mut [Row], job: &LayoutJob) {
    if job.text.is_ascii() {
        return;
    }
    let bidi = unicode_bidi::BidiInfo::new(&job.text, None);
    if !bidi.has_rtl() {
        return;
    }
    let mut byte_start = 0;
    for row in rows {
        let byte_count: usize = row.glyphs.iter().map(|g| g.chr.len_utf8()).sum();
        let end = (byte_start + byte_count).min(job.text.len());
        if let Some(para) = bidi
            .paragraphs
            .iter()
            .find(|p| p.range.start <= byte_start && byte_start < p.range.end)
        {
            // Elision replaces the tail with an ellipsis. Resolve that visible row separately.
            let visible: String = row.glyphs.iter().map(|g| g.chr).collect();
            let levels: Vec<_> = if job.text.get(byte_start..end) == Some(visible.as_str()) {
                let levels = bidi.reordered_levels(para, byte_start..end);
                visible
                    .char_indices()
                    .map(|(index, _)| levels[byte_start + index])
                    .collect()
            } else {
                let line = unicode_bidi::BidiInfo::new(&visible, Some(para.level));
                visible
                    .char_indices()
                    .map(|(index, _)| line.levels[index])
                    .collect()
            };
            let order = unicode_bidi::BidiInfo::reorder_visual(&levels);
            let mut x = row.rect.left();
            for index in order {
                let glyph = &mut row.glyphs[index];
                glyph.rtl = levels[index].is_rtl();
                glyph.pos.x = x;
                x += glyph.advance_width;
            }
            row.rect.max.x = x;
        }
        byte_start = end + usize::from(row.ends_with_newline);
    }
}
