# Third-party notices

NABI VIEW's original code is licensed under the MIT License in `LICENSE`.
Dependencies, bundled fonts and vendored code retain their own licenses and
copyright notices; the project's MIT license does not replace those terms.

The application includes `THIRD_PARTY_NOTICES.txt`, generated from the locked
Cargo dependency graph, published source packages, bundled font metadata and
the upstream notices retained in `licenses/upstream/`. It includes platform
specific, build and test dependencies, so not every listed package is linked into
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
- `vendor/epaint` and `vendor/egui-wgpu` contain the patched 0.37.0 upstream
  crates and retain their original MIT/Apache-2.0 licenses. They remain third-party
  code even though workspace membership enables their regression tests.
  The MIT-licensed `ds_store` and `mac_alias` helpers and the Chromium
  BSD-style DMG layout recipe retain their notices under `vendor/dmg-layout/`.
  These Python helpers are build tools, not application resources.

## Notice provenance and review scope

Inventory refreshed 2026-11-12 against Cargo.lock: 482 third-party Rust
packages, including transitive, platform-specific, build and test dependencies,
plus bundled native codecs, fonts and vendored packaging helpers. This is the
resolved dependency inventory, not a claim that every component is linked into
every platform's application binary.

58 packages in the current inventory have supplemental upstream notices
recorded with source URLs and SHA-256 hashes in
`licenses/upstream/index.json`. Some upstream notices declare licenses by
name; the generated catalog also supplies their standard terms.

`dispatch` 0.2.0 declares MIT, but no separate upstream license file was found
at its published source revision. Its published author, license declaration and
reference terms are included in the generated catalog. Original upstream
attribution remains flagged for review; reference terms are not represented as
recovered originals. The former `block` 0.1.6 and `hexf-parse` 0.2.1 review entries
are absent from the current resolved graph.

References:

- https://www.mozilla.org/en-US/MPL/2.0/FAQ/ (questions 8 and 11)
- https://www.mozilla.org/en-US/MPL/2.0/ (sections 3.1-3.3)
- https://openfontlicense.org/ofl-faq/ (bundling fonts with software)
- https://ubuntu.com/legal/font-licence
- https://www.apache.org/licenses/LICENSE-2.0 (section 4)

- `vendor/epaint` uses upstream 0.37.0 HarfRust, skrifa and vello text handling
  with local bidirectional layout, RTL caret and cluster corrections. Details
  are in `vendor/epaint/PATCH.md`. Their dependencies retain the licenses listed
  in the generated notices. System fallback fonts are loaded locally and are
  not copied into the application bundle.
- `vendor/egui-wgpu` 0.37.0 adds the application's explicit wgpu 30 surface
  color-space selection, native linear HDR texture handling, output white scale
  and FP16 capture support. Details are in `vendor/egui-wgpu/NABI-PATCH.md`.
