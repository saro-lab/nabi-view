# Nabi View Unicode layout extension

Source: epaint 0.37.0, https://github.com/emilk/egui/tree/0.37.0/crates/epaint
Original MIT/Apache-2.0 licenses and upstream source notices are retained.

The upstream HarfRust shaping, font fallback, TTC/variation handling, rasterizers,
and color glyph support remain in use. This patch adds Unicode bidirectional run
segmentation and visual line placement while preserving source strings and one
logical glyph slot per Unicode scalar for cursor and selection indexing.

Shaping runs also respect script and embedding-direction boundaries. Shaped
clusters retain their visual glyph parts in a cached RGBA atlas composition, then
share the advance across their logical characters. Line wrapping and elision do
not divide these clusters. RTL caret hit testing follows visual positions;
backgrounds and decorations follow visual runs. Atlas compositions are discarded
with the atlas cache and use the existing glyph-size limit.

Additional dependencies: unicode-bidi 0.3.18 (MIT/Apache-2.0), unicode-script 0.5.9
(MIT/Apache-2.0), verified against crates.io on 2026-11-12. Upstream already uses
harfrust 0.12.0. No font data is redistributed by this patch. See the generated
distribution notices for complete dependency license texts.

Upstream test gates are corrected for this workspace: fixed bundled-font metrics
require `default_fonts`, and reentrant-lock panic tests require debug assertions
(release locks intentionally do not detect deadlocks). The shared rasterizer
test-key helper is available in both font configurations.
