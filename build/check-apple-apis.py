#!/usr/bin/env python3
"""Reject known private WindowServer references in every Mach-O bundle slice."""
import argparse
import mmap
from pathlib import Path


MACHO_MAGIC = {
    b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe",
    b"\xfe\xed\xfa\xcf", b"\xcf\xfa\xed\xfe",
    b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca",
    b"\xca\xfe\xba\xbf", b"\xbf\xba\xfe\xca",
}
FORBIDDEN = (b"CGSSetWindowBackgroundBlurRadius", b"CGSMainConnectionID")


def check(path):
    """Scan thin/fat Mach-O bytes, including import tables and symbol strings."""
    count = 0
    failures = []
    candidates = sorted(path.rglob("*")) if path.is_dir() else [path]
    for candidate in candidates:
        if candidate.is_symlink() or not candidate.is_file():
            continue
        with candidate.open("rb") as source:
            if source.read(4) not in MACHO_MAGIC:
                continue
            count += 1
            with mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as data:
                for symbol in FORBIDDEN:
                    if data.find(symbol) >= 0:
                        failures.append(f"{candidate}: {symbol.decode()}")
    if failures:
        raise ValueError("Private Apple API references:\n" + "\n".join(failures))
    if not count:
        raise ValueError(f"No Mach-O binaries found: {path}")
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path, help="Mach-O executable or app bundle")
    args = parser.parse_args()
    try:
        count = check(args.path)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    print(f"Apple private API check passed ({count} Mach-O files): {args.path}")


if __name__ == "__main__":
    main()
