#!/usr/bin/env python3
# Portable alias recipe adapted from Chromium; Copyright 2026 The Chromium Authors.
# Its BSD-style license is retained in vendor/dmg-layout/CHROMIUM-LICENSE.
"""Write the installer's Finder layout without mounting a disk image."""

import argparse
import datetime
import os
from pathlib import Path
import sys

# These pure-Python build dependencies are pinned in vendor/dmg-layout/UPSTREAM.md.
# Import them directly: packaging must not require pip or a network connection.
sys.dont_write_bytecode = True
VENDOR = Path(__file__).resolve().parents[2] / "vendor" / "dmg-layout"
for package in ("ds_store", "mac_alias"):
    sys.path.insert(0, str(VENDOR / package / "src"))

from ds_store import DSStore
from mac_alias import (
    ALIAS_EJECTABLE_DISK,
    ALIAS_HFS_VOLUME_SIGNATURE,
    ALIAS_KIND_FILE,
    ALIAS_NO_CNID,
    Alias,
    TargetInfo,
    VolumeInfo,
)


VOLUME_NAME = "NABI VIEW"
APP_NAME = "NABI VIEW.app"
BACKGROUND = ".background/background.tiff"


def background_alias():
    # Resolve against the eventual volume, not the build machine's staging path
    # or inode numbers. Chromium uses this same offline alias strategy:
    # https://chromium.googlesource.com/chromium/src/+/9c363bb2863cd723978a8a245719389a7bf1c20e/chrome/installer/mac/generate_dsstore.py
    date = datetime.datetime(2000, 5, 7, 21, 10, tzinfo=datetime.timezone.utc)
    volume = VolumeInfo(
        name=VOLUME_NAME,
        creation_date=date,
        fs_type=ALIAS_HFS_VOLUME_SIGNATURE,
        disk_type=ALIAS_EJECTABLE_DISK,
        attribute_flags=0,
        fs_id=b"\0\0",
        posix_path="/Volumes/" + VOLUME_NAME,
    )
    target = TargetInfo(
        kind=ALIAS_KIND_FILE,
        filename="background.tiff",
        folder_cnid=ALIAS_NO_CNID,
        cnid=ALIAS_NO_CNID,
        creation_date=date,
        creator_code=b"\0\0\0\0",
        type_code=b"\0\0\0\0",
        carbon_path=VOLUME_NAME + ":" + BACKGROUND.replace("/", ":"),
        posix_path="/" + BACKGROUND,
    )
    return Alias(volume=volume, target=target).to_bytes()


def write_layout(directory):
    if not directory.is_dir():
        raise ValueError("DMG staging directory does not exist")
    if not (directory / APP_NAME).is_dir():
        raise ValueError("DMG staging directory must contain NABI VIEW.app")
    applications = directory / "Applications"
    if not applications.is_symlink() or os.readlink(applications) != "/Applications":
        raise ValueError("DMG staging directory must contain Applications -> /Applications")
    background = directory / BACKGROUND
    if not background.is_file() or background.stat().st_size == 0:
        raise ValueError("DMG staging directory must contain a nonempty .background/background.tiff")

    with DSStore.open(str(directory / ".DS_Store"), "w+") as store:
        store["."]["vSrn"] = ("long", 1)
        store["."]["icvl"] = ("type", b"icnv")
        store["."]["bwsp"] = {
            "WindowBounds": "{{240, 140}, {660, 462}}",
            "ShowToolbar": False,
            "ShowStatusBar": False,
            "ShowPathbar": False,
            "ShowSidebar": False,
            "ContainerShowSidebar": False,
            "ShowTabView": False,
            "PreviewPaneVisibility": False,
            "SidebarWidth": 180,
        }
        store["."]["icvp"] = {
            "viewOptionsVersion": 1,
            "backgroundType": 2,
            "backgroundImageAlias": background_alias(),
            "backgroundColorRed": 1.0,
            "backgroundColorGreen": 1.0,
            "backgroundColorBlue": 1.0,
            "gridOffsetX": 0.0,
            "gridOffsetY": 0.0,
            "gridSpacing": 100.0,
            "arrangeBy": "none",
            "showIconPreview": False,
            "showItemInfo": False,
            "labelOnBottom": True,
            "textSize": 13.0,
            "iconSize": 112.0,
        }
        store[APP_NAME]["Iloc"] = (180, 230)
        store["Applications"]["Iloc"] = (480, 230)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="prepared DMG staging directory")
    args = parser.parse_args()
    try:
        write_layout(args.directory)
    except (OSError, ValueError) as error:
        parser.exit(1, "DMG layout: {}\n".format(error))


if __name__ == "__main__":
    main()
