#!/usr/bin/env python3
"""Package a Windows release using Windows SDK MakeAppx (Python 3.9+)."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
NS = {
    "": "http://schemas.microsoft.com/appx/manifest/foundation/windows10",
    "uap": "http://schemas.microsoft.com/appx/manifest/uap/windows10",
    "uap3": "http://schemas.microsoft.com/appx/manifest/uap/windows10/3",
    "rescap": "http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities",
}
TARGETS = {"x64": "x86_64-pc-windows-msvc", "arm64": "aarch64-pc-windows-msvc"}
for prefix, uri in NS.items():
    ET.register_namespace(prefix, uri)


def element(parent, name, attributes=None, text=None):
    prefix, local = name.split(":", 1) if ":" in name else ("", name)
    node = ET.SubElement(parent, f"{{{NS[prefix]}}}{local}", attributes or {})
    node.text = text
    return node


def validate_identity(identity, development=False):
    for key in ("name", "publisher", "publisher_display_name", "display_name"):
        value = identity.get(key)
        if not isinstance(value, str) or not value.strip() or "REPLACE_WITH" in value:
            raise ValueError(f"Set {key} from Partner Center in your identity JSON")
    if not re.fullmatch(r"[A-Za-z0-9.-]{3,50}", identity["name"]):
        raise ValueError("Package identity name must be 3-50 letters, digits, dots or hyphens")
    if not identity["publisher"].startswith("CN="):
        raise ValueError("Publisher must be the complete CN=... value from Partner Center")
    if not development and identity["name"] == "NabiView.Development":
        raise ValueError("Development identity cannot be used for Store submission")


def validate_version(value):
    if not re.fullmatch(r"\d+\.\d+\.\d+\.0", value):
        raise ValueError("Store version must have four components and end in .0")
    parts = [int(part) for part in value.split(".")]
    if not 1 <= parts[0] <= 65535 or any(part > 65535 for part in parts):
        raise ValueError("Version components must be <= 65535, with nonzero major")
    return value


def supported_extensions():
    source = (ROOT / "src/core/media/mod.rs").read_text(encoding="utf-8")
    match = re.search(r"pub const EXTENSIONS:.*?= &\[(.*?)\];", source, re.S)
    if not match:
        raise ValueError("Could not read supported image extensions")
    return re.findall(r'"([a-z0-9]+)"', match[1])


def manifest(identity, version, arch):
    package = ET.Element(f"{{{NS['']}}}Package", {"IgnorableNamespaces": "uap uap3 rescap"})
    element(package, "Identity", {"Name": identity["name"], "Publisher": identity["publisher"],
                                 "Version": validate_version(version), "ProcessorArchitecture": arch})
    properties = element(package, "Properties")
    element(properties, "DisplayName", text=identity["display_name"])
    element(properties, "PublisherDisplayName", text=identity["publisher_display_name"])
    element(properties, "Description", text="Local image reader")
    element(properties, "Logo", text="Assets/StoreLogo.png")
    resources = element(package, "Resources")
    for locale in json.loads((ROOT / "locales/manifest.json").read_text(encoding="utf-8")):
        element(resources, "Resource", {"Language": locale["code"]})
    dependencies = element(package, "Dependencies")
    element(dependencies, "TargetDeviceFamily", {"Name": "Windows.Desktop",
            "MinVersion": "10.0.19041.0", "MaxVersionTested": "10.0.26100.0"})
    apps = element(package, "Applications")
    app = element(apps, "Application", {"Id": "NabiView", "Executable": "nabi-view.exe",
                                        "EntryPoint": "Windows.FullTrustApplication"})
    element(app, "uap:VisualElements", {"DisplayName": identity["display_name"],
            "Description": "Local image reader", "BackgroundColor": "transparent",
            "Square150x150Logo": "Assets/Square150x150Logo.png",
            "Square44x44Logo": "Assets/Square44x44Logo.png"})
    extensions = element(app, "Extensions")
    extension = element(extensions, "uap:Extension", {"Category": "windows.fileTypeAssociation"})
    association = element(extension, "uap3:FileTypeAssociation", {
        "Name": "nabiview.images", "Parameters": '"%1"'})
    element(association, "uap:DisplayName", text="NABI VIEW Image")
    types = element(association, "uap:SupportedFileTypes")
    for ext in supported_extensions():
        element(types, "uap:FileType", text=f".{ext}")
    capabilities = element(package, "Capabilities")
    element(capabilities, "rescap:Capability", {"Name": "runFullTrust"})
    ET.indent(package)
    return ET.ElementTree(package)


def sdk_tool(name):
    override = shutil.which(name)
    if override:
        return Path(override)
    base = Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Windows Kits/10/bin"
    versions = sorted((p for p in base.glob("10.*") if p.is_dir()),
                      key=lambda p: tuple(map(int, p.name.split("."))), reverse=True)
    for version in versions:
        candidate = version / "x64" / name
        if candidate.is_file():
            return candidate
    raise ValueError(f"{name} not found. Install the Windows SDK or add it to PATH")


def validate_binary(binary, arch):
    with binary.open("rb") as stream:
        if stream.read(2) != b"MZ":
            raise ValueError("Not a Windows executable")
        stream.seek(0x3c)
        pe = struct.unpack("<I", stream.read(4))[0]
        stream.seek(pe)
        if stream.read(4) != b"PE\0\0":
            raise ValueError("Not a PE executable")
        machine = struct.unpack("<H", stream.read(2))[0]
    if machine != {"x64": 0x8664, "arm64": 0xaa64}[arch]:
        raise ValueError(f"Executable architecture does not match {arch}")
    # The wrapper builds with crt-static. Refuse a binary requiring a VC runtime
    # that is not included in this self-contained package.
    data = binary.read_bytes().lower()
    if re.search(rb"(?:vcruntime|msvcp|msvcr)\d+[^\x00/\\]*\.dll\x00", data):
        raise ValueError("Dynamic Visual C++ runtime detected; rebuild with -C target-feature=+crt-static")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    identity_args = parser.add_mutually_exclusive_group(required=True)
    identity_args.add_argument("--identity", type=Path)
    identity_args.add_argument("--development", action="store_true")
    parser.add_argument("--arch", choices=TARGETS, default="x64")
    parser.add_argument("--version", help="Four-part Store version; defaults to Cargo version + .0")
    args = parser.parse_args()
    identity = ({"name": "NabiView.Development", "publisher": "CN=NabiView Development",
                 "publisher_display_name": "NabiView Development", "display_name": "NABI VIEW Dev"}
                if args.development else json.loads(args.identity.read_text(encoding="utf-8-sig")))
    validate_identity(identity, args.development)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--offline", "--locked", "--no-deps", "--format-version=1"], cwd=ROOT))
    version = args.version or next(p["version"] for p in metadata["packages"] if p["name"] == "nabi-view") + ".0"
    validate_version(version)
    target = Path(metadata["target_directory"]).resolve()
    binary = target / TARGETS[args.arch] / "release/nabi-view.exe"
    validate_binary(binary, args.arch)
    notices = ROOT / "target/notices/THIRD_PARTY_NOTICES.txt"
    if not notices.is_file():
        raise ValueError("Run build/notices.py before packaging")
    makeappx = sdk_tool("makeappx.exe")
    output = target / "msix" / ("development" if args.development else "store")
    output.mkdir(parents=True, exist_ok=True)
    # Retain each build's staging tree for inspection, signing and local registration.
    stage = Path(tempfile.mkdtemp(prefix="package-", dir=output))
    shutil.copy2(binary, stage / "nabi-view.exe")
    shutil.copytree(ROOT / "assets/field-notes", stage / "field-notes")
    shutil.copytree(ROOT / "platforms/windows/Assets", stage / "Assets")
    shutil.copy2(notices, stage / notices.name)
    for name in ("LICENSE", "THIRD_PARTY.md"):
        shutil.copy2(ROOT / name, stage / name)
    manifest(identity, version, args.arch).write(stage / "AppxManifest.xml", encoding="utf-8", xml_declaration=True)
    destination = output / f"{identity['name']}_{version}_{args.arch}.msix"
    # Pack into a new location so a failed SDK validation preserves the last good output.
    packed = stage.with_suffix(".msix")
    subprocess.run([str(makeappx), "pack", "/d", str(stage), "/p", str(packed), "/o"], check=True)
    packed.replace(destination)
    digest = hashlib.sha256(destination.read_bytes()).hexdigest()
    destination.with_suffix(".msix.sha256").write_text(f"{digest}  {destination.name}\n", encoding="utf-8")
    print(f"MSIX: {destination}\nUnpacked: {stage}\nSHA256: {digest}")
    print("DEVELOPMENT IDENTITY: not for Store upload. Unsigned; sign/trust for local installation."
          if args.development else "Unsigned Store package: submit using the matching Partner Center identity.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"MSIX packaging failed: {error}")
