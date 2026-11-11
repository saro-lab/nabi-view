#!/usr/bin/env python3
"""Stage public sources and locked Cargo dependencies for an offline Flatpak build."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = (
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE", "THIRD_PARTY.md",
    "src", "build", "assets", "licenses", "locales", "platforms", "vendor",
)


def validate_toolchain(manifest):
    # Accept a pinned release, not a moving channel. Keep Python 3.9 support.
    declaration = (ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
    match = re.search(r'^channel\s*=\s*"(\d+\.\d+\.\d+)"\s*$', declaration, re.MULTILINE)
    if not match:
        raise ValueError("rust-toolchain.toml must pin an exact Rust release")
    version = match.group(1)
    sources = manifest["modules"][0]["sources"]
    for arch in ("x86_64", "aarch64"):
        for component in ("rustc", "cargo", "rust-std"):
            matches = [source for source in sources
                       if source.get("dest") == f"_rust_dist/{component}"
                       and source.get("only-arches") == [arch]]
            url = (rf"https://static\.rust-lang\.org/dist/\d{{4}}-\d{{2}}-\d{{2}}/"
                   rf"{component}-{re.escape(version)}-{arch}-unknown-linux-gnu\.tar\.xz")
            if (len(matches) != 1 or matches[0].get("type") != "archive"
                    or not re.fullmatch(url, matches[0].get("url", ""))
                    or not re.fullmatch(r"[0-9a-f]{64}", matches[0].get("sha256", ""))):
                raise ValueError(f"Update the pinned Flatpak {component} archive for {arch} / Rust {version}")
    return version


def stage_sources(destination):
    # Git's excludes prevent local credentials, agent instructions, runtime files,
    # and private documents from entering the build sandbox. Only public roots
    # are considered; never copy the checkout or follow its ai symlink wholesale.
    paths = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *PUBLIC],
        cwd=ROOT,
    ).decode().split("\0")
    # A local build uses the current worktree, including deliberate deletions
    # that have not yet been staged or committed.
    deleted = subprocess.check_output(
        ["git", "ls-files", "-z", "--deleted", "--", *PUBLIC], cwd=ROOT,
    ).decode().split("\0")
    for relative in sorted(set(filter(None, paths)) - set(deleted)):
        source = ROOT / relative
        if any(path.is_symlink() for path in (source, *source.parents) if path != ROOT.parent):
            raise ValueError(f"Public source must not be a symlink: {relative}")
        if not source.is_file():
            raise ValueError(f"Missing public source: {relative}")
        output = destination / relative
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, output)
        output.chmod(0o755 if os.access(source, os.X_OK) else 0o644)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true", help="Use only already-cached Cargo sources")
    parser.add_argument("--check-toolchain", action="store_true",
                        help="Check the installed sandbox compiler against the pinned release")
    args = parser.parse_args()
    manifest = json.loads((ROOT / "platforms/linux/dev.nabi.NabiView.json").read_text())
    version = validate_toolchain(manifest)
    if args.check_toolchain:
        actual = subprocess.check_output(["rustc", "--version"], text=True).split()
        if actual[:2] != ["rustc", version]:
            raise ValueError(f"Expected rustc {version}, got {' '.join(actual)}")
        print(f"Flatpak Rust {version}: pinned compiler verified")
        return
    scratch = ROOT / "ai/tmp/flatpak"
    scratch.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix="sources-", dir=scratch))
    source = staging / "source"
    source.mkdir()
    stage_sources(source)
    command = ["cargo", "vendor", "--locked", "--versioned-dirs"]
    if args.offline:
        command.append("--offline")
    command.append("_cargo_vendor")
    config = subprocess.check_output(command, cwd=source, text=True)
    cargo_config = source / ".cargo/config.toml"
    cargo_config.parent.mkdir()
    cargo_config.write_text(config, encoding="utf-8")
    # Cargo itself checks the vendored checksums and proves resolution works
    # without the host registry before handing sources to flatpak-builder.
    subprocess.run(["cargo", "metadata", "--frozen", "--format-version=1"],
                   cwd=source, stdout=subprocess.DEVNULL, check=True)
    manifest["modules"][0]["sources"][0]["path"] = str(source)
    output = staging / "dev.nabi.NabiView.json"
    output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(output)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(f"Flatpak source preparation failed: {error}")
