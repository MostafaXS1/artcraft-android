#!/usr/bin/env python3
"""Unit tests for lib/file_associations.py."""

from pathlib import Path
import re
import sys
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "lib"))
import file_associations


class TestFileAssociations(unittest.TestCase):
    def test_associations_dict_populated(self):
        self.assertGreater(len(file_associations.ASSOCIATIONS), 15)
        self.assertIn("wordcraft", file_associations.ASSOCIATIONS)
        self.assertIn("photocraft", file_associations.ASSOCIATIONS)
        self.assertIn("cadcraft", file_associations.ASSOCIATIONS)

    def test_mime_database_sanity(self):
        """Verify that all mapped extensions have valid MIME types."""
        self.assertGreater(len(file_associations.EXT_MIME), 30)
        for ext, mimes in file_associations.EXT_MIME.items():
            self.assertTrue(ext.islower(), f"Extension {ext!r} should be lowercase")
            self.assertFalse(ext.startswith("."), f"Extension {ext!r} should not start with a dot")
            self.assertTrue(len(mimes) > 0, f"Extension {ext!r} must have at least one MIME type")
            for mime in mimes:
                self.assertIn("/", mime, f"Invalid MIME format {mime!r} for extension {ext}")
                self.assertEqual(mime, mime.strip(), f"MIME {mime!r} has leading/trailing whitespace")

    def test_intent_filter_toml_validity(self):
        """Verify that intent_filter_toml generates valid TOML for every mapped app."""
        for app in file_associations.ASSOCIATIONS:
            toml_value = file_associations.intent_filter_toml(app)
            self.assertTrue(toml_value.startswith("["), f"intent_filter for {app} must start with [")
            self.assertTrue(toml_value.endswith("]"), f"intent_filter for {app} must end with ]")

            # Parse with tomllib
            wrapper = f"intent_filter = {toml_value}"
            try:
                parsed = tomllib.loads(wrapper)
            except Exception as e:
                self.fail(f"Failed to parse TOML intent_filter for {app}: {e}\nValue: {toml_value[:200]}")

            filters = parsed.get("intent_filter", [])
            self.assertTrue(len(filters) >= 1, f"Expected at least MAIN_FILTER for {app}")
            # First filter is always MAIN/LAUNCHER
            main_filter = filters[0]
            self.assertIn("android.intent.action.MAIN", main_filter.get("actions", []))
            self.assertIn("android.intent.category.LAUNCHER", main_filter.get("categories", []))

    def test_patch_cargo_toml_application(self):
        """Test patching a mock Cargo.toml with file associations."""
        original_toml = (
            '[package]\n'
            'name = "wordcraft-android"\n'
            'version = "0.2.1"\n\n'
            '[package.metadata.android.application.activity]\n'
            'name = "com.artcraft.wordcraft.WordCraftActivity"\n'
            'intent_filter = [{ actions = ["android.intent.action.MAIN"] }]\n'
        )
        with tempfile.TemporaryDirectory() as tmp_dir:
            cargo_path = Path(tmp_dir) / "Cargo.toml"
            cargo_path.write_text(original_toml)

            # First run should patch
            modified = file_associations.patch_cargo_toml(cargo_path, "wordcraft")
            self.assertTrue(modified, "patch_cargo_toml should return True when modifying")

            patched_content = cargo_path.read_text()
            self.assertIn("android.intent.category.LAUNCHER", patched_content)
            self.assertIn("android.intent.action.VIEW", patched_content)

            # Must be valid TOML
            parsed = tomllib.loads(patched_content)
            self.assertIn("package", parsed)

            # Running on a file without intent_filter returns False
            no_filter_path = Path(tmp_dir) / "NoFilter.toml"
            no_filter_path.write_text('[package]\nname = "test"\n')
            self.assertFalse(file_associations.patch_cargo_toml(no_filter_path, "wordcraft"))


if __name__ == "__main__":
    unittest.main()
