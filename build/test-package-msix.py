#!/usr/bin/env python3
"""Regression checks for inputs that would produce an unusable Store package."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location("msix", Path(__file__).with_name("package-msix.py"))
msix = importlib.util.module_from_spec(spec)
spec.loader.exec_module(msix)


class PackagingTests(unittest.TestCase):
    def test_store_version_rejects_reserved_revision_and_out_of_range(self):
        for version in ("1.0.0", "1.0.0.1", "0.1.0.0", "65536.0.0.0", "1.65536.0.0", "1.0.0.0/../other"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                msix.validate_version(version)
        self.assertEqual(msix.validate_version("1.2.3.0"), "1.2.3.0")

    def test_example_identity_cannot_be_submitted(self):
        import json
        identity = json.loads((msix.ROOT / "platforms/windows/store-identity.example.json").read_text())
        with self.assertRaises(ValueError):
            msix.validate_identity(identity)

    def test_manifest_escapes_publisher_and_quotes_file_path(self):
        identity = {"name": "Example.NabiView", "publisher": "CN=Example & Company",
                    "publisher_display_name": "Example & Company", "display_name": "NABI VIEW"}
        msix.validate_identity(identity)
        xml = ET.tostring(msix.manifest(identity, "1.0.0.0", "x64").getroot())
        parsed = ET.fromstring(xml)
        self.assertEqual(parsed.find("Identity", msix.NS).attrib["Publisher"], identity["publisher"])
        association = parsed.find(".//uap3:FileTypeAssociation", msix.NS)
        self.assertEqual(association.attrib["Parameters"], '"%1"')
        types = [node.text for node in association.findall(".//uap:FileType", msix.NS)]
        self.assertIn(".png", types)
        self.assertIn(".avif", types)
        self.assertEqual(len(types), len(set(types)))

    def test_binary_architecture_and_runtime_checks(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "test.exe"
            data = bytearray(128)
            data[:2] = b"MZ"
            data[0x3c:0x40] = (64).to_bytes(4, "little")
            data[64:70] = b"PE\0\0\x64\x86"
            binary.write_bytes(data)
            msix.validate_binary(binary, "x64")
            with self.assertRaises(ValueError):
                msix.validate_binary(binary, "arm64")
            binary.write_bytes(data + b"VCRUNTIME140.dll\0")
            with self.assertRaises(ValueError):
                msix.validate_binary(binary, "x64")


if __name__ == "__main__":
    unittest.main()
