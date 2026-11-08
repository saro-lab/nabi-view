# Nabi View text layout extension

Source: epaint 0.31.1, https://github.com/emilk/egui/tree/0.31.1/crates/epaint
Original MIT/Apache-2.0 licenses and upstream source notices are retained.

This patch adds HarfRust shaping and Unicode bidirectional line placement to the
pinned renderer. It preserves source text and one logical glyph entry per Unicode
scalar, composes multi-glyph clusters into the existing alpha atlas, preserves TTC
face indexes, and prevents wrapping inside a shaped cluster. ASCII layout keeps
the original path. RTL caret hit testing uses visual positions without reversing
the underlying string.

Dependencies: harfrust 0.12.0 (MIT), unicode-bidi 0.3.18 (MIT/Apache-2.0), and
unicode-script 0.5.9 (MIT/Apache-2.0). Check the generated distribution notices for
complete transitive license text. No font data is redistributed by this patch.
