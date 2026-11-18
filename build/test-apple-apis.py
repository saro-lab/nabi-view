#!/usr/bin/env python3
"""Exercise the packaging gate without needing Apple SDK tools."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("apple_apis", Path(__file__).with_name("check-apple-apis.py"))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class AppleApiCheck(unittest.TestCase):
    def test_nested_extension_and_fat_slice_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "main").write_bytes(b"\xcf\xfa\xed\xfe" + b"public API")
            extension = root / "PlugIns" / "preview"
            extension.parent.mkdir()
            extension.write_bytes(b"\xca\xfe\xba\xbe" + b"\0" * 128 + b"_CGSSetWindowBackgroundBlurRadius\0")
            with self.assertRaisesRegex(ValueError, "preview: CGSSetWindowBackgroundBlurRadius"):
                checker.check(root)
            extension.write_bytes(b"\xca\xfe\xba\xbe" + b"public API")
            (root / "notices.txt").write_bytes(checker.FORBIDDEN[0])
            self.assertEqual(checker.check(root), 2)

    def test_connection_reference_alone_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "main"
            binary.write_bytes(b"\xcf\xfa\xed\xfe_CGSMainConnectionID\0")
            with self.assertRaisesRegex(ValueError, "CGSMainConnectionID"):
                checker.check(binary)

    def test_empty_or_missing_inputs_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "No Mach-O"):
                checker.check(Path(directory))
            with self.assertRaisesRegex(ValueError, "No Mach-O"):
                checker.check(Path(directory) / "missing")


if __name__ == "__main__":
    unittest.main()
