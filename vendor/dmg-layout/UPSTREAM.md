# Vendored DMG layout dependencies

These MIT-licensed, pure-Python packages are used only by
`build/macos/dmg-layout.py` to write Finder metadata at build time. They are not
included in the application or its disk image. No pip installation is needed.

| Package | Version | Pinned upstream commit |
| --- | --- | --- |
| [ds_store](https://github.com/dmgbuild/ds_store) | 1.3.1 | [906169b4c9ce12e2d3e462ef487ea29031d2843f](https://github.com/dmgbuild/ds_store/tree/906169b4c9ce12e2d3e462ef487ea29031d2843f) |
| [mac_alias](https://github.com/dmgbuild/mac_alias) | 2.2.2 | [c5c6fa8f59792a6e1b3812086e540857ef31be45](https://github.com/dmgbuild/mac_alias/tree/c5c6fa8f59792a6e1b3812086e540857ef31be45) |

The `src/` package files, upstream README, LICENSE, `setup.cfg`, and
`pyproject.toml` are copied unchanged from those commits. Tests, documentation
sources, and repository tooling are omitted. These versions support Python 3.7+
and preserve the project's Python 3.9+ build requirement. Each package's LICENSE
is retained beside its sources.

The portable background alias strategy follows Chromium's
[generate_dsstore.py](https://chromium.googlesource.com/chromium/src/+/9c363bb2863cd723978a8a245719389a7bf1c20e/chrome/installer/mac/generate_dsstore.py):
use the final volume name and volume-relative background path with unknown
catalog IDs, without recording a build-machine path or requiring a mounted disk.
The adapted alias recipe is Copyright 2026 The Chromium Authors, under the
BSD-style license retained in `CHROMIUM-LICENSE` from the same pinned revision.
The Chromium command-line implementation is not vendored.

Upstream versions and source contents verified on 2026-11-07.
