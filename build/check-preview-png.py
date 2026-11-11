#!/usr/bin/env python3
"""Check a generated freedesktop thumbnail without image library dependencies."""
from pathlib import Path
import struct
import sys
import zlib

data = Path(sys.argv[1]).read_bytes()
bound = int(sys.argv[2])
assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
offset = 8
metadata = {}
dimensions = None
while offset < len(data):
    length = int.from_bytes(data[offset:offset + 4], "big")
    kind = data[offset + 4:offset + 8]
    body = data[offset + 8:offset + 8 + length]
    expected = int.from_bytes(data[offset + 8 + length:offset + 12 + length], "big")
    assert zlib.crc32(kind + body) == expected, "invalid chunk CRC"
    if kind == b"IHDR":
        dimensions = struct.unpack(">II", body[:8])
    elif kind == b"tEXt":
        key, value = body.split(b"\0", 1)
        metadata[key.decode("latin1")] = value.decode("latin1")
    offset += length + 12
assert dimensions and all(0 < value <= bound for value in dimensions), dimensions
assert metadata["Thumb::URI"].startswith("file:///"), metadata
assert int(metadata["Thumb::MTime"]) >= 0
assert int(metadata["Thumb::Size"]) > 0
assert metadata["Software"] == "NABI VIEW"
print(f"Thumbnail {dimensions[0]}x{dimensions[1]}, CRCs and freedesktop source metadata passed.")
