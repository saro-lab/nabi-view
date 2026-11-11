#!/usr/bin/env python3
"""Reidentify and sign nested Quick Look extensions for a Mac Store candidate."""
import argparse
import datetime
import os
from pathlib import Path
import plistlib
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path)
    parser.add_argument("--identity", required=True)
    parser.add_argument("--signed", action="store_true")
    args = parser.parse_args()
    parent = plistlib.loads((args.app / "Contents/Info.plist").read_bytes())
    for name, suffix, variable in (("NabiThumbnail", "thumbnail", "NABI_MACOS_THUMBNAIL_PROFILE"),
                                   ("NabiPreview", "preview", "NABI_MACOS_PREVIEW_PROFILE")):
        extension = args.app / "Contents/PlugIns" / (name + ".appex")
        info_path = extension / "Contents/Info.plist"
        info = plistlib.loads(info_path.read_bytes())
        bundle = parent["CFBundleIdentifier"] + "." + suffix
        info.update(CFBundleIdentifier=bundle, CFBundleVersion=parent["CFBundleVersion"],
                    CFBundleShortVersionString=parent["CFBundleShortVersionString"])
        info_path.write_bytes(plistlib.dumps(info))
        entitlements = {"com.apple.security.app-sandbox": True}
        if args.signed:
            source = Path(os.environ[variable])
            profile = plistlib.loads(subprocess.check_output(["security", "cms", "-D", "-i", str(source)]))
            allowed = profile.get("Entitlements", {})
            team = os.environ["NABI_APPLE_TEAM_ID"]
            app_id = allowed.get("com.apple.application-identifier")
            expected = [f"{prefix}.{bundle}" for prefix in profile.get("ApplicationIdentifierPrefix", [])]
            if team not in profile.get("TeamIdentifier", []) or app_id not in expected:
                raise ValueError(f"{variable} must match {bundle} and the selected team")
            if (profile.get("ProvisionedDevices") or profile.get("ProvisionsAllDevices")
                    or allowed.get("get-task-allow") or allowed.get("com.apple.security.get-task-allow")):
                raise ValueError(f"{variable} must be a Mac App Store distribution profile")
            if profile["ExpirationDate"] <= datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None):
                raise ValueError(f"{variable} has expired")
            entitlements["com.apple.application-identifier"] = app_id
            entitlements["com.apple.developer.team-identifier"] = team
            shutil.copyfile(source, extension / "Contents/embedded.provisionprofile")
        # Generated entitlements stay beside the candidate and are removed before publishing.
        temporary = args.app.parent / (name + ".entitlements.plist")
        temporary.write_bytes(plistlib.dumps(entitlements))
        try:
            library = extension / "Contents/Frameworks/libnabi_preview.dylib"
            subprocess.run(["codesign", "--force", "--sign", args.identity, str(library)], check=True)
            subprocess.run(["codesign", "--force", "--sign", args.identity, "--entitlements", str(temporary), str(extension)], check=True)
            subprocess.run(["codesign", "--verify", "--strict", str(extension)], check=True)
        finally:
            temporary.unlink()

if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"Quick Look signing failed: {error}")
