#!/usr/bin/env python3
"""Package an already-built Windows/Linux binary and its local resources."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile

from package_resources import copy_resources


ROOT = Path(__file__).resolve().parents[1]
TARGETS = (
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-gnu",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
)
MARKER = ".nabi-package.json"
APP_NAME = "NABI VIEW"


def owned_package(path, target):
    if path.is_symlink() or path.resolve() != path or not path.is_dir():
        return False
    try:
        return json.loads((path / MARKER).read_text(encoding="utf-8")) == {
            "format": 1,
            "target": target,
        }
    except (OSError, ValueError):
        return False


def make_archive(app, archive, windows):
    if windows:
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as output:
            for path in sorted(app.rglob("*")):
                if path.is_file():
                    output.write(path, path.relative_to(app.parent).as_posix())
    else:
        # tar preserves the executable bit, including when packaging on Windows.
        def permissions(info):
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mode = 0o755 if info.isdir() or info.name in (f"{APP_NAME}/nabi-view", f"{APP_NAME}/previews/nabi-thumbnailer", f"{APP_NAME}/previews/install.sh") else 0o644
            return info

        with tarfile.open(archive, "w:gz") as output:
            output.add(app, arcname=APP_NAME, filter=permissions)


def cargo_target_directory():
    metadata = subprocess.check_output(
        ["cargo", "metadata", "--offline", "--no-deps", "--locked", "--format-version=1"], cwd=ROOT
    )
    return Path(json.loads(metadata)["target_directory"]).resolve()


def package(target, binary=None):
    build_dir = cargo_target_directory()
    windows = "-windows-" in target
    if binary is None:
        binary = build_dir / target / "release" / ("nabi-view.exe" if windows else "nabi-view")
    binary = binary.resolve(strict=True)
    if not binary.is_file():
        raise ValueError(f"Not a binary file: {binary}")
    notices = ROOT / "target/notices/THIRD_PARTY_NOTICES.txt"
    if not notices.is_file():
        raise ValueError("Missing third-party notices. Run python3 build/notices.py first.")
    package_dir = build_dir / "package"
    if package_dir.is_symlink():
        raise ValueError(f"Unexpected output directory: {package_dir}")
    package_dir.mkdir(parents=True, exist_ok=True)
    target_dir = package_dir / target
    if target_dir.is_symlink():
        raise ValueError(f"Unexpected output directory: {target_dir}")
    target_dir.mkdir(exist_ok=True)
    if target_dir.resolve().parent != package_dir:
        raise ValueError(f"Unexpected output directory: {target_dir}")
    app_output = target_dir / APP_NAME
    archive_output = target_dir / f"nabi-view-{target}.{'zip' if windows else 'tar.gz'}"
    if os.path.lexists(app_output) and not owned_package(app_output, target):
        raise ValueError(f"Refusing to replace an unrecognized package: {app_output}")
    if os.path.lexists(archive_output) and (
        archive_output.is_symlink() or not archive_output.is_file()
        or not owned_package(app_output, target)
    ):
        raise ValueError(f"Refusing to replace an unrecognized archive: {archive_output}")

    staging = Path(tempfile.mkdtemp(prefix=".nabi-portable-", dir=target_dir))
    keep_staging = False
    try:
        app = staging / APP_NAME
        app.mkdir()
        executable = app / ("nabi-view.exe" if windows else "nabi-view")
        shutil.copy2(binary, executable)
        executable.chmod(0o755)
        previews = app / "previews"
        if windows and target.endswith("msvc"):
            previews.mkdir()
            for name in ("nabi_preview.dll", "nabi-shell-preview.dll"):
                shutil.copy2(build_dir / target / "release" / name, previews / name)
            shutil.copy2(ROOT / "platforms/preview/windows/register.ps1", previews / "register.ps1")
        elif not windows:
            previews.mkdir()
            shutil.copy2(build_dir / target / "release/nabi-thumbnailer", previews / "nabi-thumbnailer")
            (previews / "nabi-thumbnailer").chmod(0o755)
            for name in ("install.sh", "nabi-view.thumbnailer"):
                shutil.copy2(ROOT / "platforms/preview/linux" / name, previews / name)
            (previews / "install.sh").chmod(0o755)
        copy_resources(ROOT, app)
        shutil.copy2(notices, app / notices.name)
        for name in ("LICENSE", "THIRD_PARTY.md"):
            shutil.copy2(ROOT / name, app / name)
        (app / MARKER).write_text(
            json.dumps({"format": 1, "target": target}) + "\n", encoding="utf-8"
        )
        archive = staging / archive_output.name
        make_archive(app, archive, windows)

        backups = []
        installed = []
        try:
            for source, destination in ((app, app_output), (archive, archive_output)):
                if destination.exists():
                    backup = staging / f"previous-{len(backups)}"
                    destination.rename(backup)
                    backups.append((backup, destination))
                source.rename(destination)
                installed.append((destination, source))
        except OSError:
            # Move new files back before restoring the previous complete package.
            try:
                for destination, source in reversed(installed):
                    destination.rename(source)
                for backup, destination in reversed(backups):
                    backup.rename(destination)
            except OSError as recovery_error:
                keep_staging = True
                raise RuntimeError(
                    f"Could not restore the previous package; recovery files remain at {staging}"
                ) from recovery_error
            raise
    finally:
        if not keep_staging:
            # Only remove the temporary directory created by this invocation.
            if staging.is_symlink() or staging.resolve().parent != target_dir:
                raise ValueError(f"Refusing to clean an unexpected staging path: {staging}")
            shutil.rmtree(staging)
    return app_output, archive_output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--binary", type=Path, help="Built executable; defaults to the selected Cargo target's release binary")
    args = parser.parse_args()
    try:
        for path in package(args.target, args.binary):
            print(path)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
