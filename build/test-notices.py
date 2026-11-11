#!/usr/bin/env python3
"""Protect app credits when patched upstream crates join the test workspace."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("notices", Path(__file__).with_name("notices.py"))
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)


class WorkspaceCreditsTests(unittest.TestCase):
    def metadata(self):
        locations = {
            "app": "Cargo.toml",
            "preview": "platforms/preview/rust/Cargo.toml",
            "epaint": "vendor/epaint/Cargo.toml",
            "egui-wgpu": "vendor/egui-wgpu/Cargo.toml",
            "image": "registry/image/Cargo.toml",
            "harfrust": "registry/harfrust/Cargo.toml",
            "test-only": "registry/test-only/Cargo.toml",
        }
        packages = [{"id": name, "name": name, "version": "1.0.0", "license": "MIT",
                     "repository": "https://example.invalid/" + name, "homepage": None,
                     "manifest_path": str(notices.ROOT / path)}
                    for name, path in locations.items()]

        def node(name, dependencies):
            return {"id": name, "deps": [{"pkg": dep, "dep_kinds": [{"kind": kind}]}
                                        for dep, kind in dependencies]}

        return {"packages": packages,
                "workspace_members": ["app", "preview", "epaint", "egui-wgpu"],
                "resolve": {"nodes": [node("app", [("image", None), ("test-only", "dev")]),
                                      node("preview", [("app", None)]),
                                      node("epaint", [("harfrust", None)]),
                                      node("egui-wgpu", [("epaint", None)])]}}

    def test_vendor_workspace_members_remain_third_party(self):
        metadata = self.metadata()
        first_party = notices.first_party_members(metadata)
        self.assertEqual(first_party, {"app", "preview"})
        catalog_ids = {pkg["id"] for pkg in metadata["packages"] if pkg["id"] not in first_party}
        self.assertTrue({"epaint", "egui-wgpu", "harfrust"} <= catalog_ids)

    def test_vendor_dependencies_do_not_become_direct_app_credits(self):
        entries = json.loads(notices.direct_dependencies(self.metadata()))
        cargo_names = {entry["name"] for entry in entries if entry["version"]}
        self.assertEqual(cargo_names, {"image", "epaint", "egui-wgpu"})


if __name__ == "__main__":
    unittest.main()
