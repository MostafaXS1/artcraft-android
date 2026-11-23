#!/usr/bin/env python3
"""Unit tests for adapters in adapters/."""

import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent
ADAPTERS_DIR = ROOT / "adapters"
CATALOG_SH = ROOT / "lib" / "catalog.sh"


class TestAdapters(unittest.TestCase):
    def setUp(self):
        self.adapters = [d for d in sorted(ADAPTERS_DIR.iterdir()) if d.is_dir()]
        self.assertTrue(len(self.adapters) > 0, "No adapters found in adapters/")

    def test_all_adapters_have_valid_bash_syntax(self):
        """Verify bash -n on every prepare.sh found across all adapters."""
        checked = 0
        for adapter in self.adapters:
            prep = adapter / "prepare.sh"
            if prep.is_file():
                res = subprocess.run(["bash", "-n", str(prep)], capture_output=True, text=True)
                self.assertEqual(
                    res.returncode,
                    0,
                    f"Syntax error in {prep.relative_to(ROOT)}:\n{res.stderr}",
                )
                self.assertTrue(os.access(prep, os.X_OK), f"{prep.relative_to(ROOT)} is not executable")
                checked += 1
        self.assertGreater(checked, 15, "Expected at least 15 adapters with prepare.sh")

    def test_all_adapters_cargo_toml_valid(self):
        """Verify all Cargo.toml files in adapters/ are valid TOML."""
        checked = 0
        for adapter in self.adapters:
            cargo = adapter / "Cargo.toml"
            if cargo.is_file():
                try:
                    with open(cargo, "rb") as f:
                        data = tomllib.load(f)
                except Exception as e:
                    self.fail(f"Invalid TOML in {cargo.relative_to(ROOT)}: {e}")
                self.assertIn("package", data, f"{cargo.relative_to(ROOT)} missing [package] table")
                package_name = data["package"].get("name", "")
                self.assertTrue(
                    package_name.endswith("-android"),
                    f"Package name in {cargo.relative_to(ROOT)} should end with -android, got: {package_name}",
                )
                checked += 1
        self.assertGreater(checked, 15, "Expected at least 15 adapters with Cargo.toml")

    def test_all_adapter_cargo_package_ids(self):
        """Verify [package.metadata.android] package ID follows com.artcraft.<app> or dev.tracedraw.app."""
        for adapter in self.adapters:
            cargo = adapter / "Cargo.toml"
            if cargo.is_file():
                with open(cargo, "rb") as f:
                    data = tomllib.load(f)
                meta = data.get("package", {}).get("metadata", {}).get("android", {})
                package_id = meta.get("package", "")
                if package_id:
                    self.assertTrue(
                        package_id.startswith("com.artcraft.") or package_id == "dev.tracedraw.app",
                        f"Unexpected package ID in {cargo.relative_to(ROOT)}: {package_id}",
                    )

    def test_embedded_python_heredocs_syntax(self):
        """Verify that all Python heredocs inside prepare.sh compile without syntax or indentation errors."""
        checked_heredocs = 0
        for adapter in self.adapters:
            prep = adapter / "prepare.sh"
            if not prep.is_file():
                continue
            text = prep.read_text()
            # Match python3 - ... <<'PY' ... PY blocks
            matches = re.findall(r"<<\s*['\"]?PY['\"]?\n(.*?\n)PY", text, re.DOTALL)
            for idx, code in enumerate(matches):
                try:
                    compile(code, f"{adapter.name}/prepare.sh:heredoc_{idx}", "exec")
                except SyntaxError as e:
                    self.fail(
                        f"Python syntax/indentation error in {adapter.name}/prepare.sh block {idx}:\n"
                        f"Line {e.lineno}: {e.msg}\n{e.text}"
                    )
                checked_heredocs += 1
        self.assertGreater(checked_heredocs, 10, "Expected to find embedded Python heredocs in prepare.sh")

    def test_adapter_catalog_sync(self):
        """Verify that any custom adapter directory names match supported catalog apps."""
        catalog_text = CATALOG_SH.read_text()
        for adapter in self.adapters:
            app_name = adapter.name
            # The app name must appear in lib/catalog.sh
            self.assertIn(
                app_name,
                catalog_text,
                f"Adapter directory {app_name} does not appear anywhere in lib/catalog.sh",
            )


if __name__ == "__main__":
    unittest.main()
