#!/usr/bin/env python3
"""Regression checks for rejecting broken native release packages."""
import importlib.util
import struct
import tempfile
import unittest
import zipfile
from pathlib import Path

spec = importlib.util.spec_from_file_location("android_elf", Path(__file__).with_name("check-android-elf.py"))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def elf(alignment=16384, machine=183):
    data = bytearray(120)
    data[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<HH", data, 16, 3, machine)
    struct.pack_into("<Q", data, 32, 64)
    struct.pack_into("<HH", data, 54, 56, 1)
    struct.pack_into("<IIQQQQQQ", data, 64, 1, 5, 0, 0, 0, 120, 120, alignment)
    return bytes(data)


class NativeReleaseChecks(unittest.TestCase):
    def test_accepts_supported_64_bit_libraries(self):
        self.assertEqual(checker.check_elf(elf(), "arm"), "arm64-v8a")
        self.assertEqual(checker.check_elf(elf(machine=62), "intel"), "x86_64")

    def test_rejects_4_kb_load_alignment(self):
        with self.assertRaisesRegex(ValueError, "not 16 KB"):
            checker.check_elf(elf(alignment=4096), "legacy")

    def test_rejects_truncated_header(self):
        with self.assertRaises(ValueError):
            checker.check_elf(elf()[:80], "broken")

    def test_bundle_rejects_missing_requested_abi(self):
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory) / "test.aab"
            with zipfile.ZipFile(bundle, "w") as archive:
                archive.writestr("base/lib/arm64-v8a/libnabi.so", elf())
            with self.assertRaisesRegex(ValueError, "Expected ABIs"):
                checker.check_package(bundle, ["arm64-v8a", "x86_64"])

    def test_rejects_library_in_wrong_abi_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory) / "test.aab"
            with zipfile.ZipFile(bundle, "w") as archive:
                archive.writestr("base/lib/x86_64/libnabi.so", elf())
            with self.assertRaisesRegex(ValueError, "path ABI"):
                checker.check_package(bundle)

    def test_apk_requires_zip_alignment_as_well_as_elf_alignment(self):
        with tempfile.TemporaryDirectory() as directory:
            apk = Path(directory) / "test.apk"
            with zipfile.ZipFile(apk, "w") as archive:
                archive.writestr("lib/arm64-v8a/libnabi.so", elf())
            with self.assertRaisesRegex(ValueError, "ZIP aligned"):
                checker.check_package(apk)
            name = "lib/arm64-v8a/libnabi.so"
            entry = zipfile.ZipInfo(name)
            padding = 16384 - 30 - len(name)
            entry.extra = struct.pack("<HH", 0xCAFE, padding - 4) + bytes(padding - 4)
            with zipfile.ZipFile(apk, "w") as archive:
                archive.writestr(entry, elf())
            self.assertEqual(checker.check_package(apk), (1, {"arm64-v8a"}))


if __name__ == "__main__":
    unittest.main()
