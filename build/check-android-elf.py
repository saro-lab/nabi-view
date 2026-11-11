#!/usr/bin/env python3
"""Check native ELF alignment/ABI and APK ZIP alignment without extracting files."""
import argparse
import struct
import zipfile
from pathlib import Path

PAGE_SIZE = 16384
MACHINES = {183: "arm64-v8a", 62: "x86_64"}


def check_elf(data, name):
    if len(data) < 64 or data[:6] != b"\x7fELF\x02\x01":
        raise ValueError(f"{name}: expected a little-endian ELF64 shared library")
    kind, machine = struct.unpack_from("<HH", data, 16)
    if kind != 3 or machine not in MACHINES:
        raise ValueError(f"{name}: unsupported shared library ABI")
    offset = struct.unpack_from("<Q", data, 32)[0]
    size, count = struct.unpack_from("<HH", data, 54)
    if size < 56 or not count or offset + size * count > len(data):
        raise ValueError(f"{name}: invalid ELF program headers")
    loads = 0
    for index in range(count):
        segment, _, file_offset, address, _, file_size, _, alignment = struct.unpack_from("<IIQQQQQQ", data, offset + size * index)
        if segment != 1:  # PT_LOAD
            continue
        loads += 1
        if alignment < PAGE_SIZE or alignment & (alignment - 1) or (address - file_offset) % PAGE_SIZE:
            raise ValueError(f"{name}: LOAD segment is not 16 KB aligned")
        if file_offset + file_size > len(data):
            raise ValueError(f"{name}: truncated LOAD segment")
    if not loads:
        raise ValueError(f"{name}: no LOAD segments")
    return MACHINES[machine]


def check_package(path, expected_abis=None):
    abis = set()
    count = 0
    if path.suffix == ".so":
        abis.add(check_elf(path.read_bytes(), path.name))
        count = 1
    else:
        with zipfile.ZipFile(path) as archive, path.open("rb") as source:
            for entry in archive.infolist():
                parts = entry.filename.split("/")
                if not entry.filename.endswith(".so") or "lib" not in parts:
                    continue
                declared_abi = parts[parts.index("lib") + 1]
                actual_abi = check_elf(archive.read(entry), entry.filename)
                if declared_abi != actual_abi:
                    raise ValueError(f"{entry.filename}: path ABI does not match ELF machine")
                if path.suffix == ".apk":
                    if entry.compress_type != zipfile.ZIP_STORED:
                        raise ValueError(f"{entry.filename}: expected uncompressed native library")
                    source.seek(entry.header_offset + 26)
                    name_size, extra_size = struct.unpack("<HH", source.read(4))
                    if (entry.header_offset + 30 + name_size + extra_size) % PAGE_SIZE:
                        raise ValueError(f"{entry.filename}: APK entry is not 16 KB ZIP aligned")
                abis.add(actual_abi)
                count += 1
    if not count:
        raise ValueError("No native libraries found")
    if expected_abis is not None and abis != set(expected_abis):
        raise ValueError(f"Expected ABIs {sorted(expected_abis)}, found {sorted(abis)}")
    return count, abis


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path, help=".so, .apk or .aab")
    parser.add_argument("--abis", help="Exact comma-separated ABI set expected in the package")
    args = parser.parse_args()
    try:
        count, abis = check_package(args.package, args.abis.split(",") if args.abis else None)
    except (ValueError, OSError, zipfile.BadZipFile, struct.error) as error:
        parser.exit(1, f"Android native check failed: {error}\n")
    print(f"Android native check: {count} ELF libraries, 16 KB LOAD alignment, ABIs: {', '.join(sorted(abis))}")
    if args.package.suffix == ".apk":
        print("APK uncompressed native entries are 16 KB ZIP aligned.")
    elif args.package.suffix == ".aab":
        print("AAB native ELF check only; verify generated APKs and bundletool PAGE_ALIGNMENT_16K before release.")


if __name__ == "__main__":
    main()
