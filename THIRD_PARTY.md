# Third-party notices

NABI VIEW's original code is licensed under the MIT License in `LICENSE`.
Dependencies, bundled fonts and vendored code retain their own licenses and
copyright notices; the project's MIT license does not replace those terms.

The application includes `THIRD_PARTY_NOTICES.txt`, generated from the locked
Cargo dependency graph, published source packages, bundled font metadata and
the upstream notices retained in `licenses/upstream/`. It includes platform
specific and build dependencies, so not every listed package is linked into
every build. Settings > About > Open source licenses shows a flat summary of
direct dependencies and explicitly vendored code. Enable "Show full license
texts" there to read the complete notices offline, together with the project's
MIT license.

## Source availability

`option-ext` 0.2.0 is used without modifications under MPL-2.0. Its source code,
including the MPL license, is available from:

- https://crates.io/api/v1/crates/option-ext/0.2.0/download
- https://github.com/soc/option-ext/tree/272f22fc9ea1ac6b08f01704af52c4ac338df4e2

Recipients retain their MPL-2.0 rights to this component. Changes to its covered
files must remain available under MPL-2.0. This does not change the license of
NABI VIEW's separate original source files.

## Bundled components

- MIT, Apache-2.0, BSD, ISC, Zlib, Boost and Unicode components retain their
  copyright and license notices. For alternative-license declarations (`OR`),
  MIT is used where available; `zune-inflate` uses its Zlib option. Multiple
  required licenses (`AND`) are retained together. `r-efi` offers MIT as an
  alternative to LGPL; this distribution uses MIT.
- AVIF uses `libavif-sys` and bundled libavif with `libaom-sys` and libaom.
  Native licenses, third-party notices and patent grants are included in the
  generated notices. FFmpeg, ImageMagick and libheif are not bundled by this
  configuration. Operating-system image codecs remain operating-system
  components.
- The default egui fonts retain OFL-1.1, Ubuntu Font Licence 1.0, MIT and the
  Hack font's accompanying terms and attribution. The font licenses do not
  replace the application's MIT license. System fonts are read from the
  installed operating system and are not redistributed by NABI VIEW.
- `vendor/block` retains its original MIT declaration and patch notice.
  The MIT-licensed `ds_store` and `mac_alias` helpers and the Chromium
  BSD-style DMG layout recipe retain their notices under `vendor/dmg-layout/`.
  These Python helpers are build tools, not application resources.

## Notice provenance and review scope

Reviewed 2026-11-08 against the current Cargo.lock: 470 third-party Rust
packages, including transitive and platform-specific dependencies, plus the
bundled native codecs, fonts and vendored packaging helpers. No requirement
to relicense NABI VIEW's original code away from MIT was identified in this
configuration.

59 packages omit a license file from their published crate but have upstream
notices recorded with source URLs and SHA-256 hashes in
`licenses/upstream/index.json`. Some upstream notices declare licenses by
name; the generated catalog also supplies their standard terms.

`block` 0.1.6 and `dispatch` 0.2.0 declare MIT, and `hexf-parse` 0.2.1 declares
CC0-1.0, but no separate upstream license file was found for those releases.
Their published authors, license declarations and reference terms are
included in the generated catalog. Original upstream attribution remains
flagged for review; reference terms are not represented as recovered originals.

References:

- https://www.mozilla.org/en-US/MPL/2.0/FAQ/ (questions 8 and 11)
- https://www.mozilla.org/en-US/MPL/2.0/ (sections 3.1-3.3)
- https://openfontlicense.org/ofl-faq/ (bundling fonts with software)
- https://ubuntu.com/legal/font-licence
- https://www.apache.org/licenses/LICENSE-2.0 (section 4)

- `vendor/epaint` contains epaint 0.31.1 with local Unicode shaping and
  bidirectional text layout changes. Its original MIT/Apache-2.0 licenses are
  retained; modification details are in `vendor/epaint/PATCH.md`. HarfRust and
  Unicode bidirectional/script support retain the licenses listed in the
  generated dependency notices. System fallback fonts are loaded locally and
  are not copied into the application bundle.
