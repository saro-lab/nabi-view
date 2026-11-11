"""Copy the explicit public resource set into a release package."""

import argparse
from pathlib import Path
import shutil


SAMPLES = (
    "01 Alpine Light.png",
    "02 Pelagic.png",
    "03 Dune Forms.png",
    "04 After Dark.png",
    "05 Evergreen.png",
)
ICONS = ("nabi-icon.icns", "nabi-icon.ico", "nabi-icon.svg") + tuple(
    f"png/nabi-icon-{size}.png"
    for size in (16, 20, 24, 32, 40, 48, 64, 128, 256, 512, 1024)
)


def copy_resources(root, destination, *, macos=False):
    root = Path(root)
    destination = Path(destination)
    resources = [(f"assets/field-notes/{name}", f"field-notes/{name}") for name in SAMPLES]
    if macos:
        resources.append(("assets/icons/nabi-icon.icns", "NabiView.icns"))
    else:
        resources.extend((f"assets/icons/{name}", f"icons/{name}") for name in ICONS)

    # Git ignores do not apply to filesystem copies. Never copy a whole asset
    # directory, or follow a resource link into local files outside this set.
    for source, _ in resources:
        path = root
        for component in Path(source).parts:
            path = path / component
            if path.is_symlink():
                raise ValueError(f"Release resource must not be a symlink: {path}")
        if not path.is_file():
            raise ValueError(f"Missing release resource: {path}")

    for source, relative in resources:
        output = destination / relative
        output.parent.mkdir(parents=True, exist_ok=True)
        # Copy content only, without local extended attributes or metadata.
        shutil.copyfile(root / source, output)
        output.chmod(0o644)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--macos", action="store_true")
    args = parser.parse_args()
    try:
        copy_resources(Path(__file__).resolve().parents[1], args.destination, macos=args.macos)
    except (OSError, ValueError) as error:
        parser.exit(1, f"Resource packaging failed: {error}\n")


if __name__ == "__main__":
    main()
