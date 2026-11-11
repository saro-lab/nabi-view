#!/usr/bin/env python3
"""Build and embed independent OS previews; never register or enable them locally."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
TYPES = [
    ("public.avif", "AVIF", ["avif"], "image/avif"),
    ("public.jpeg-xl", "JPEG XL", ["jxl"], "image/jxl"),
    ("dev.nabi.qoi", "Quite OK Image", ["qoi"], "image/qoi"),
    ("dev.nabi.openexr", "OpenEXR", ["exr"], "image/x-exr"),
    ("dev.nabi.radiance", "Radiance HDR", ["hdr"], "image/vnd.radiance"),
    ("com.microsoft.dds", "DirectDraw Surface", ["dds"], "image/vnd-ms.dds"),
    ("dev.nabi.farbfeld", "farbfeld", ["ff"], "image/farbfeld"),
]

def run(args):
    subprocess.run([str(arg) for arg in args], cwd=ROOT, check=True)

def target_directory():
    return Path(json.loads(subprocess.check_output(["cargo", "metadata", "--offline", "--locked", "--no-deps", "--format-version=1"], cwd=ROOT))["target_directory"])

def build(target):
    run(["cargo", "build", "--locked", "--release", "-p", "nabi-preview", "--target", target])

def macos(app, target, no_build):
    if sys.platform != "darwin":
        raise ValueError("macOS extension packaging requires a Mac")
    targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"] if target == "universal" else [target]
    if any(item not in ("aarch64-apple-darwin", "x86_64-apple-darwin") for item in targets):
        raise ValueError("Unsupported macOS target")
    output = target_directory()
    if not no_build:
        for item in targets:
            build(item)
    version = json.loads(subprocess.check_output(["cargo", "metadata", "--offline", "--locked", "--no-deps", "--format-version=1"], cwd=ROOT))["packages"]
    version = next(item["version"] for item in version if item["name"] == "nabi-view")
    app.mkdir(parents=True, exist_ok=True)
    info_path = app / "Contents/Info.plist"
    info = plistlib.loads(info_path.read_bytes()) if info_path.exists() else {"CFBundleIdentifier": "dev.nabi.NabiView"}
    declarations = [{"UTTypeIdentifier": uti, "UTTypeDescription": label,
                     "UTTypeConformsTo": ["public.image"],
                     "UTTypeTagSpecification": {"public.filename-extension": extensions, "public.mime-type": mime}}
                    for uti, label, extensions, mime in TYPES]
    old = {item["UTTypeIdentifier"]: item for item in info.get("UTImportedTypeDeclarations", [])}
    old.update({item["UTTypeIdentifier"]: item for item in declarations})
    info["UTImportedTypeDeclarations"] = list(old.values())
    info_path.parent.mkdir(parents=True, exist_ok=True)
    info_path.write_bytes(plistlib.dumps(info))
    for name, suffix, principal in [("NabiThumbnail", "thumbnail", "NabiThumbnailProvider"), ("NabiPreview", "preview", "NabiPreviewController")]:
        extension = app / "Contents/PlugIns" / (name + ".appex")
        binary = extension / "Contents/MacOS" / name
        frameworks = extension / "Contents/Frameworks"
        binary.parent.mkdir(parents=True, exist_ok=True)
        frameworks.mkdir(parents=True, exist_ok=True)
        dylib = frameworks / "libnabi_preview.dylib"
        libraries = [output / item / "release/libnabi_preview.dylib" for item in targets]
        if len(libraries) == 2:
            run(["xcrun", "lipo", "-create", *libraries, "-output", dylib])
        else:
            shutil.copyfile(libraries[0], dylib)
        run(["install_name_tool", "-id", "@rpath/libnabi_preview.dylib", dylib])
        command = ["xcrun", "clang", "-fobjc-arc", "-fblocks", "-O2", "-mmacosx-version-min=11.0"]
        for item in targets:
            command += ["-arch", "arm64" if item.startswith("aarch64") else "x86_64"]
        command += [ROOT / "platforms/preview/macos/Preview.m", "-L" + str(frameworks), "-lnabi_preview",
                    "-Wl,-rpath,@executable_path/../Frameworks", "-framework", "AppKit", "-framework", "QuickLookUI",
                    "-framework", "QuickLookThumbnailing", "-o", binary]
        if suffix == "thumbnail":
            command += ["-DNABI_THUMBNAIL=1"]
        run(command)
        attributes = {"QLSupportedContentTypes": [item[0] for item in TYPES]}
        if suffix == "thumbnail":
            attributes["QLThumbnailMinimumDimension"] = 0
        else:
            attributes["QLSupportsSearchableItems"] = False
        plist = {"CFBundleExecutable": name, "CFBundleName": name, "CFBundleDisplayName": "NABI VIEW " + suffix,
                 "CFBundleIdentifier": info["CFBundleIdentifier"] + "." + suffix,
                 "CFBundleInfoDictionaryVersion": "6.0", "CFBundlePackageType": "XPC!",
                 "CFBundleShortVersionString": version, "CFBundleVersion": info.get("CFBundleVersion", "1"),
                 "LSMinimumSystemVersion": "11.0", "NSExtension": {
                     "NSExtensionPointIdentifier": "com.apple.quicklook." + suffix,
                     "NSExtensionPrincipalClass": principal, "NSExtensionAttributes": attributes}}
        (extension / "Contents/Info.plist").write_bytes(plistlib.dumps(plist))
        run(["codesign", "--force", "--sign", "-", dylib])
        run(["codesign", "--force", "--sign", "-", "--entitlements", ROOT / "platforms/preview/macos/extension.entitlements", extension])
        run(["codesign", "--verify", "--strict", extension])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=["macos"])
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    macos(args.app.resolve(), args.target, args.no_build)

if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"OS preview packaging failed: {error}")
