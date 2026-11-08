#!/usr/bin/env python3
"""Normalize and verify a freshly generated, unmounted makehybrid HFS+ image.

DiscRecording synthesizes FinderInfo even for clean, signed bundles. Remove that
metadata from this bundle's regular files/directories, compare its complete
contents and modes with the source, and extract it for codesign verification.
This intentionally supports only the simple images emitted by this build: no
journal, extended attributes, resource forks, hard links or overflow extents.

Format reference (reviewed 2026-11-07): Apple TN1150 and the macOS SDK's
<hfs/hfs_format.h>. This is an original, bounded parser, not a general HFS tool.
https://developer.apple.com/library/archive/technotes/tn/tn1150.html
"""

import argparse
import os
from pathlib import Path
import stat
import struct
import subprocess
import unicodedata


def require(condition, message):
    if not condition:
        raise ValueError(message)


def u16(data, offset=0):
    return struct.unpack_from(">H", data, offset)[0]


def u32(data, offset=0):
    return struct.unpack_from(">I", data, offset)[0]


def normalized(path):
    return unicodedata.normalize("NFD", str(path))


class Image:
    def __init__(self, handle):
        self.handle = handle
        self.size = os.fstat(handle.fileno()).st_size
        first = self.read(0, 512)
        require(first[:2] == b"ER", "Expected a makehybrid Apple partition map")
        sector = u16(first, 2)
        require(sector in (512, 2048), "Unsupported partition-map sector size")
        entry = self.read(sector, 512)
        count = u32(entry, 4)
        require(entry[:2] == b"PM" and 1 <= count <= 64, "Invalid partition map")
        volumes = []
        for index in range(1, count + 1):
            entry = self.read(index * sector, 512)
            require(entry[:2] == b"PM" and u32(entry, 4) == count, "Invalid map entry")
            if entry[48:80].split(b"\0", 1)[0] == b"Apple_HFS":
                volumes.append((u32(entry, 8) * sector, u32(entry, 12) * sector))
        require(len(volumes) == 1, "Expected exactly one HFS+ partition")
        self.start, self.length = volumes[0]
        require(self.start + self.length <= self.size, "Partition exceeds image")
        header = self.read(self.start + 1024, 512)
        require(header[:4] == b"H+\0\4", "Expected HFS+ version 4")
        require(not u32(header, 4) & (1 << 13), "Journaled images are unsupported")
        self.block_size = u32(header, 40)
        require(self.block_size >= 512 and self.block_size & (self.block_size - 1) == 0,
                "Invalid allocation block size")
        self.blocks = u32(header, 44)
        require(self.blocks * self.block_size == self.length, "Invalid volume size")
        require(not any(header[352:432]), "Extended-attribute trees are unsupported")
        require(not any(header[432:512]), "Startup files are unsupported")
        self.catalog = self.fork(header[272:352])
        self.catalog_ranges = [(offset, offset + size) for offset, size in self.catalog[1]]
        extents = self.fork(header[192:272])
        if extents[0]:
            require(u32(self.fork_read(extents, 0, 64), 20) == 0,
                    "Extent-overflow records are unsupported")
        self.entries = self.read_catalog()

    def read(self, offset, size):
        require(0 <= offset <= self.size and 0 <= size <= self.size - offset,
                "Image read exceeds file bounds")
        self.handle.seek(offset)
        data = self.handle.read(size)
        require(len(data) == size, "Truncated image")
        return data

    def fork(self, data):
        require(len(data) == 80, "Truncated fork record")
        size, _, blocks = struct.unpack_from(">QII", data)
        extents = []
        total = 0
        empty_seen = False
        for offset in range(16, 80, 8):
            start, count = struct.unpack_from(">II", data, offset)
            if not count:
                require(start == 0, "Invalid empty extent")
                empty_seen = True
                continue
            require(not empty_seen and start + count <= self.blocks,
                    "Invalid or overflowing extent")
            extents.append((self.start + start * self.block_size, count * self.block_size))
            total += count
        require(total == blocks and size <= blocks * self.block_size,
                "Overflow extents or invalid fork size")
        return size, extents

    def fork_parts(self, fork, offset, size):
        require(0 <= offset <= fork[0] and 0 <= size <= fork[0] - offset,
                "Read exceeds fork size")
        for physical, length in fork[1]:
            if offset >= length:
                offset -= length
                continue
            count = min(size, length - offset)
            if count:
                yield physical + offset, count
                size -= count
            offset = 0
            if not size:
                return
        require(size == 0, "Truncated fork extents")

    def fork_read(self, fork, offset, size):
        return b"".join(self.read(pos, length) for pos, length in self.fork_parts(fork, offset, size))

    def chunks(self, fork):
        for offset in range(0, fork[0], 1024 * 1024):
            yield self.fork_read(fork, offset, min(1024 * 1024, fork[0] - offset))

    def read_catalog(self):
        header = self.fork_read(self.catalog, 0, 64)
        require(header[8] == 1, "Invalid catalog header node")
        node_size = u16(header, 32)
        require(512 <= node_size <= 32768 and node_size & (node_size - 1) == 0,
                "Invalid catalog node size")
        total_nodes = u32(header, 36)
        require(total_nodes * node_size <= self.catalog[0], "Catalog exceeds its fork")
        node_id = u32(header, 24)
        last_leaf = u32(header, 28)
        previous = 0
        visited = set()
        records = 0
        entries = {}
        while node_id:
            require(node_id < total_nodes and node_id not in visited, "Invalid leaf chain")
            visited.add(node_id)
            node = self.fork_read(self.catalog, node_id * node_size, node_size)
            require(node[8:10] == b"\xff\x01" and u32(node, 4) == previous,
                    "Invalid catalog leaf")
            count = u16(node, 10)
            require(count <= (node_size - 14) // 4, "Invalid catalog record count")
            offsets = [u16(node, node_size - 2 * (i + 1)) for i in range(count + 1)]
            require(offsets == sorted(set(offsets)) and offsets[0] >= 14
                    and offsets[-1] <= node_size - 2 * (count + 1), "Invalid record offsets")
            for index in range(count):
                begin, end = offsets[index:index + 2]
                key_length = u16(node, begin)
                require(key_length >= 6 and key_length % 2 == 0
                        and begin + 2 + key_length + 2 <= end, "Invalid catalog key")
                name_length = u16(node, begin + 6)
                require(key_length == 6 + 2 * name_length, "Invalid catalog name length")
                name = node[begin + 8:begin + 2 + key_length].decode("utf-16be")
                record_offset = begin + 2 + key_length
                record = node[record_offset:end]
                kind = u16(record)
                records += 1
                require(kind in (1, 2, 3, 4), "Unsupported catalog record")
                if kind in (3, 4):
                    continue
                require(len(record) == (88 if kind == 1 else 248), "Invalid catalog record size")
                require(name not in ("", ".", "..") and "/" not in name and "\0" not in name,
                        "Unsupported filename")
                cnid = u32(record, 8)
                require(cnid not in entries, "Duplicate catalog ID")
                mode = u16(record, 42)
                require(stat.S_IFMT(mode) in (stat.S_IFDIR, stat.S_IFREG, stat.S_IFLNK),
                        "Unsupported file type")
                require((kind == 1) == stat.S_ISDIR(mode), "Catalog type/mode mismatch")
                entries[cnid] = {"parent": u32(node, begin + 2), "name": name,
                                 "mode": mode, "record": record,
                                 "offset": node_id * node_size + record_offset,
                                 "fork": self.fork(record[88:168]) if kind == 2 else None}
            previous, node_id = node_id, u32(node)
        require(previous == last_leaf and records == u32(header, 20), "Incomplete leaf chain")
        require(2 in entries and entries[2]["parent"] == 1, "Missing volume root")
        return entries

    def paths(self):
        result = {}
        for cnid, entry in self.entries.items():
            parts = []
            ancestors = set()
            current = cnid
            while current != 2:
                require(current not in ancestors and current in self.entries,
                        "Invalid catalog parent chain")
                ancestors.add(current)
                item = self.entries[current]
                parts.append(item["name"])
                current = item["parent"]
                require(current in self.entries and stat.S_ISDIR(self.entries[current]["mode"]),
                        "Parent is not a directory")
            path = normalized(Path(*reversed(parts)))
            require(path not in result, "Duplicate image path")
            result[path] = entry
        return result


def normalize_and_extract(image_path, source, output):
    source = source.absolute()
    require(source.is_dir() and not source.is_symlink() and source.suffix == ".app",
            "Source must be an app bundle directory")
    require(not os.path.lexists(output), "Verification output must not already exist")
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(source)], check=True)
    staging = source.parent
    source_paths = {".": staging}
    for parent, directories, files in os.walk(staging, followlinks=False):
        for name in directories + files:
            path = Path(parent) / name
            key = normalized(path.relative_to(staging))
            require(key not in source_paths, "Source has colliding normalized names")
            source_paths[key] = path
    with image_path.open("r+b") as handle:
        image = Image(handle)
        paths = image.paths()
        prefix = normalized(source.name)
        bundle = {key: value for key, value in paths.items()
                  if key == prefix or key.startswith(prefix + "/")}
        require(paths.keys() == source_paths.keys(), "Image/staging file lists differ")
        require("Applications" in paths and stat.S_ISLNK(paths["Applications"]["mode"])
                and os.readlink(staging / "Applications") == "/Applications",
                "Staging must contain the Applications symlink")
        changes = []
        for key, entry in paths.items():
            path = source_paths[key]
            mode = path.lstat().st_mode
            require(stat.S_IFMT(mode) | stat.S_IMODE(mode) == entry["mode"],
                    "Image/source type or permissions differ: " + key)
            record = entry["record"]
            require(not u16(record, 2) & ~2 and not any(record[40:42]),
                    "Unsupported catalog flags: " + key)
            if entry["fork"] is not None:
                resource = image.fork(record[168:248])
                require(resource == (0, []), "Resource forks are unsupported: " + key)
                for begin, length in entry["fork"][1]:
                    require(all(begin + length <= low or begin >= high
                                for low, high in image.catalog_ranges), "File overlaps catalog")
                if stat.S_ISLNK(mode):
                    target = os.readlink(path).encode("utf-8")
                    require(image.fork_read(entry["fork"], 0, entry["fork"][0]) == target,
                            "Image/source symlink target differs: " + key)
                    require(record[48:56] == b"slnkrhap", "Invalid symlink FinderInfo")
                else:
                    require(path.stat().st_size == entry["fork"][0], "File sizes differ: " + key)
                    with path.open("rb") as original:
                        require(all(original.read(len(chunk)) == chunk
                                    for chunk in image.chunks(entry["fork"])),
                                "Image/source file contents differ: " + key)
            if key in bundle and not stat.S_ISLNK(mode) and any(record[48:80]):
                changes.extend(image.fork_parts(image.catalog, entry["offset"] + 48, 32))
        # Only the temporary image is modified, after every source comparison passed.
        for position, length in changes:
            handle.seek(position)
            handle.write(bytes(length))
        handle.flush()
        os.fsync(handle.fileno())
        for key, entry in sorted(bundle.items(), key=lambda pair: (pair[0].count("/"), pair[0])):
            relative = Path(key).relative_to(prefix)
            destination = output / relative
            mode = entry["mode"]
            if stat.S_ISDIR(mode):
                destination.mkdir()
            elif stat.S_ISLNK(mode):
                target = image.fork_read(entry["fork"], 0, entry["fork"][0]).decode("utf-8")
                destination.symlink_to(target)
                # Preserve symlink FinderInfo: its type/creator are HFS link metadata.
                os.chmod(destination, stat.S_IMODE(mode), follow_symlinks=False)
                subprocess.run(["xattr", "-wsx", "com.apple.FinderInfo",
                                entry["record"][48:80].hex(), str(destination)], check=True)
            else:
                with destination.open("xb") as extracted:
                    for chunk in image.chunks(entry["fork"]):
                        extracted.write(chunk)
            if not stat.S_ISLNK(mode):
                require(image.fork_read(image.catalog, entry["offset"] + 48, 32) == bytes(32),
                        "FinderInfo normalization failed")
                os.chmod(destination, stat.S_IMODE(mode))
        print(f"HFS+ payload verified: {len(paths)} entries; bundle FinderInfo normalized; extracted to {output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("source_app", type=Path)
    parser.add_argument("verification_app", type=Path)
    args = parser.parse_args()
    try:
        normalize_and_extract(args.image, args.source_app, args.verification_app)
    except (OSError, ValueError, struct.error, UnicodeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"HFS+ image verification failed: {error}\n")


if __name__ == "__main__":
    main()
