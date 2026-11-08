#!/usr/bin/env python3
"""Collect local third-party notices for the locked demo build (Python 3.9+)."""
import argparse
import hashlib
import json
import os
import re
import struct
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "target/notices/THIRD_PARTY_NOTICES.txt"
SUMMARY = ROOT / "licenses/direct-dependencies.json"
UPSTREAM = ROOT / "licenses/upstream"
LICENSE_NAME = re.compile(r"^(licen[cs]es?|copying|notices?|copyright|unlicense|patents)(?:$|[-. _])", re.I)
FONT_LICENSES = {
    "Ubuntu-Light.ttf": "UFL.txt",
    "NotoEmoji-Regular.ttf": "OFL.txt",
    "Hack-Regular.ttf": "Hack-Regular.txt",
    "emoji-icon-font.ttf": "emoji-icon-font-mit-license.txt",
}


def read_text(path):
    data = path.read_bytes()
    if b"\0" in data:
        raise ValueError(f"Binary notice: {path}")
    try:
        return data.decode("utf-8-sig")
    except UnicodeDecodeError:
        return data.decode("latin-1")


def notices(pkg, root):
    """Collect actual notices, including vendored licenses and custom manifest paths."""
    explicit = pkg.get("license_file")
    files = set()
    for file in root.rglob("*"):
        rel = file.relative_to(root)
        if file.is_symlink() or not file.is_file():
            continue
        if pkg["name"] == "libsqlite3-sys" and rel.parts[0] == "sqlcipher":
            continue  # This project's bundled SQLite build does not enable SQLCipher.
        if (LICENSE_NAME.match(file.name) or
            any(part.lower() in ("licenses", "license", "licences") for part in rel.parts[:-1]) or
            str(rel) == explicit):
            files.add(file)
    if explicit and not (root / explicit).is_file():
        raise ValueError(f"Missing published license_file for {pkg['id']}: {explicit}")
    return sorted(files)


def font_metadata(path):
    """Read embedded copyright, trademark and license strings from an sfnt name table."""
    data = path.read_bytes()
    count = struct.unpack_from(">H", data, 4)[0]
    result = []
    for i in range(count):
        tag, _, off, _ = struct.unpack_from(">4sIII", data, 12 + i * 16)
        if tag != b"name":
            continue
        _, n, base = struct.unpack_from(">HHH", data, off)
        for j in range(n):
            platform, _, _, name_id, length, pos = struct.unpack_from(">6H", data, off + 6 + j * 12)
            if name_id not in (0, 7, 13, 14) or platform not in (0, 1, 3):
                continue
            start = off + base + pos
            codec = "mac_roman" if platform == 1 else "utf-16-be"
            value = data[start:start + length].decode(codec).strip()
            if value and value not in result:
                result.append(value)
        break
    return "\n".join(result)


def bundled_assets(pkg, root):
    if pkg["name"] == "epaint_default_fonts":
        for name, license_name in FONT_LICENSES.items():
            yield f"fonts/{name} (embedded attribution)", font_metadata(root / "fonts" / name)
            yield f"fonts/{name} ({license_name})", read_text(root / "fonts" / license_name)
    if pkg["name"] == "libsqlite3-sys":
        with (root / "sqlite3/sqlite3.c").open("rb") as src:
            header = src.read(16384).decode("ascii", errors="replace")
        version = re.search(r"version ([0-9.]+)\.", header).group(1)
        blessing = re.search(r"The author disclaims copyright.+?more than you give\.", header, re.S).group()
        yield f"SQLite {version} | https://sqlite.org/copyright.html", blessing.replace("\n**", "\n")


def catalog(packages):
    entries = []
    texts = {}
    missing = []
    supplements = json.loads(read_text(UPSTREAM / "index.json"))["packages"]
    # Published reference texts, without borrowing another project's copyright identity.
    roots = {p["name"]: Path(p["manifest_path"]).parent for p in packages}
    reference_files = {
        "MIT": ("image", "LICENSE-MIT"),
        "Apache-2.0": ("image", "LICENSE-APACHE"),
        "BSL-1.0": ("error-code", "LICENSE"),
        "CC0-1.0": ("wayland-protocols-plasma", "plasma-wayland-protocols/LICENSES/CC0-1.0.txt"),
    }
    references = {license_id: read_text(roots[crate] / name)
                  for license_id, (crate, name) in reference_files.items()
                  if crate in roots and (roots[crate] / name).is_file()}
    # Standard MIT terms are not another project's copyright attribution.
    if "MIT" in references:
        references["MIT"] = "MIT License\n\nPermission is hereby granted" + references["MIT"].split("Permission is hereby granted", 1)[1]
    for pkg in packages:
        root = Path(pkg["manifest_path"]).parent
        title = f"{pkg['name']} {pkg['version']}"
        files = notices(pkg, root)
        sources = [(p.relative_to(root).as_posix(), read_text(p)) for p in files]
        sources.extend(bundled_assets(pkg, root))
        supplemental = supplements.get(title, {}).get("sources", [])
        for source in supplemental:
            path = UPSTREAM / source["file"]
            if hashlib.sha256(path.read_bytes()).hexdigest() != source["sha256"]:
                raise ValueError(f"Upstream notice checksum mismatch: {path}")
            sources.append((source["url"], read_text(path)))
        lines = [title, f"  Declared license: {pkg['license'] or pkg['license_file']}",
                 f"  Package: https://crates.io/crates/{pkg['name']}/{pkg['version']}"]
        if pkg.get("repository"):
            lines.append(f"  Upstream: {pkg['repository']}")
        if pkg.get("authors"):
            lines.append("  Cargo authors: " + ", ".join(pkg["authors"]))
        if pkg["name"] == "option-ext":
            lines += [
                "  Unmodified MPL-2.0 source is available at:",
                f"  https://crates.io/api/v1/crates/option-ext/{pkg['version']}/download",
                "  The MPL-2.0 terms and source rights remain in effect for this component.",
            ]
        if not files and not supplemental:
            missing.append(title)
            lines.append("  The registry package omits the crate-level license file; review upstream before release.")
            for license_id, content in references.items():
                if license_id in re.findall(r"[A-Za-z0-9.\-]+", pkg["license"] or ""):
                    sources.append((f"{license_id} standard terms (reference; upstream attribution needs review)", content))
            vcs = root / ".cargo_vcs_info.json"
            match = re.match(r"https?://github[.]com/([^/]+/[^/]+)", pkg.get("repository") or "")
            if vcs.exists() and match:
                commit = json.loads(read_text(vcs)).get("git", {}).get("sha1")
                if commit:
                    repo = match[1].removesuffix(".git")
                    lines.append(f"  Published source revision: https://github.com/{repo}/tree/{commit}")
        elif supplemental and "MIT" in (pkg["license"] or "") and not any(
            "Permission is hereby granted" in content or "This software is provided 'as-is'" in content
            for _, content in sources
        ):
            # Some upstreams publish the grant by SPDX name and link to the terms.
            sources.append(("MIT standard terms; attribution is retained above", references["MIT"]))
        for source, content in sources:
            cleaned = content.replace("\r\n", "\n").strip()
            if not cleaned:
                continue
            # Keep distinct copyright lines even when the license boilerplate is identical.
            ref = texts.setdefault(cleaned, len(texts) + 1)
            lines.append(f"  [{ref}] {source}")
        entries.append("\n".join(lines))
    heading = [
        "NABI VIEW — Third-party software and font notices",
        "Generated from cargo metadata --locked --offline for all resolved targets.",
        "This inventory also includes platform-specific and build-time packages.",
        "Numbers beside file names refer to the full texts below.",
        "",
        f"{len(packages)} packages. {len(missing)} packages need upstream attribution review.",
    ]
    if missing:
        heading += ["Review the upstream notices of these packages before distributing the demo:", ", ".join(missing)]
    blocks = ["\n".join(heading), *entries, "FULL NOTICE TEXTS"]
    blocks.extend(f"[{ref}]\n{content}" for content, ref in texts.items())
    return "\n\n".join(blocks) + "\n", missing


def direct_dependencies(metadata):
    """Flat app credits: direct dependencies and explicitly vendored code only."""
    workspace = set(metadata["workspace_members"])
    direct = {dep["pkg"] for node in metadata["resolve"]["nodes"]
              if node["id"] in workspace for dep in node["deps"]
              if any(kind["kind"] != "dev" for kind in dep["dep_kinds"])}
    entries = []
    for pkg in metadata["packages"]:
        vendored = (ROOT / "vendor") in Path(pkg["manifest_path"]).parents
        if pkg["id"] in workspace or (pkg["id"] not in direct and not vendored):
            continue
        entries.append({"name": pkg["name"], "version": pkg["version"],
                        "license": pkg["license"] or pkg["license_file"],
                        "url": pkg["repository"] or pkg["homepage"] or ""})
    # Copied packaging helpers are not part of Cargo's graph.
    entries.extend([
        {"name": "Chromium DMG layout", "version": "", "license": "BSD-3-Clause",
         "url": "https://chromium.googlesource.com/chromium/src/+/main/chrome/installer/mac/"},
        {"name": "ds_store", "version": "", "license": "MIT",
         "url": "https://github.com/al45tair/ds_store"},
        {"name": "mac_alias", "version": "", "license": "MIT",
         "url": "https://github.com/al45tair/mac_alias"},
    ])
    entries.sort(key=lambda entry: (entry["name"].casefold(), entry["version"]))
    return json.dumps(entries, ensure_ascii=False, indent=2) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--update-summary", action="store_true",
                        help="Refresh the embedded direct-dependency credits before rebuilding")
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--offline"],
        cwd=ROOT,
    ))
    summary = direct_dependencies(metadata)
    if args.update_summary:
        SUMMARY.write_text(summary, encoding="utf-8")
        print(SUMMARY)
        return
    if not SUMMARY.is_file() or SUMMARY.read_text(encoding="utf-8") != summary:
        raise SystemExit("Direct-dependency credits are out of date. Run "
                         "python3 build/notices.py --update-summary, then rebuild the app.")
    workspace = set(metadata["workspace_members"])
    packages = sorted((p for p in metadata["packages"] if p["id"] not in workspace),
                      key=lambda p: (p["name"], p["version"], p["id"]))
    result, missing = catalog(packages)
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=OUTPUT.parent, delete=False) as out:
        temporary = Path(out.name)
        try:
            out.write(result)
            out.close()
            temporary.chmod(0o644)
            os.replace(temporary, OUTPUT)
        finally:
            temporary.unlink(missing_ok=True)
    print(f"{OUTPUT}: {len(packages)} crates, {len(missing)} upstream notices to review")


if __name__ == "__main__":
    main()
