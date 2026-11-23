#!/usr/bin/env python3
"""Unit tests for lib/cargo_patches.py."""

import os
from pathlib import Path
import py_compile
import subprocess
import sys
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent
PATCHES_SCRIPT = ROOT / "lib" / "cargo_patches.py"


class TestCargoPatches(unittest.TestCase):
    def test_script_exists_and_executable(self):
        self.assertTrue(PATCHES_SCRIPT.is_file(), f"{PATCHES_SCRIPT} does not exist")
        self.assertTrue(os.access(PATCHES_SCRIPT, os.X_OK), f"{PATCHES_SCRIPT} is not executable")

    def test_py_compile_syntax(self):
        compiled = py_compile.compile(str(PATCHES_SCRIPT), doraise=True)
        self.assertIsNotNone(compiled)

    def test_cli_usage_on_missing_args(self):
        result = subprocess.run(
            [sys.executable, str(PATCHES_SCRIPT)],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("Usage: cargo_patches.py", result.stderr)

    def test_patch_crates_io(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            cargo_toml = Path(tmp_dir) / "Cargo.toml"
            cargo_toml.write_text('[package]\nname = "test-pkg"\nversion = "0.1.0"\n')

            # 1. Add rfd patch
            cmd = [
                sys.executable,
                str(PATCHES_SCRIPT),
                "patch-crates-io",
                str(cargo_toml),
                "rfd",
                "vendor/rfd",
            ]
            res = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(res.returncode, 0, res.stderr)

            doc = tomllib.loads(cargo_toml.read_text())
            self.assertIn("patch", doc)
            self.assertIn("crates-io", doc["patch"])
            self.assertEqual(doc["patch"]["crates-io"]["rfd"]["path"], "vendor/rfd")

            # 2. Add egui-winit with git_url patch
            cmd2 = [
                sys.executable,
                str(PATCHES_SCRIPT),
                "patch-crates-io",
                str(cargo_toml),
                "egui-winit",
                "vendor/egui-winit",
                "https://github.com/storytold/egui.git",
            ]
            res2 = subprocess.run(cmd2, capture_output=True, text=True)
            self.assertEqual(res2.returncode, 0, res2.stderr)

            # 3. Idempotency check: run again, verify valid TOML and no duplicate entries
            res3 = subprocess.run(cmd2, capture_output=True, text=True)
            self.assertEqual(res3.returncode, 0, res3.stderr)
            doc2 = tomllib.loads(cargo_toml.read_text())
            self.assertEqual(doc2["patch"]["crates-io"]["egui-winit"]["path"], "vendor/egui-winit")

    def test_patch_egui_dock(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            src = Path(tmp_dir)
            cargo_toml = src / "Cargo.toml"
            cargo_toml.write_text('[workspace]\nmembers = ["apps/*"]\n\n[workspace.dependencies]\nfoo = "1.0"\n')

            cmd = [
                sys.executable,
                str(PATCHES_SCRIPT),
                "patch-egui-dock",
                str(src),
            ]
            res = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(res.returncode, 0, res.stderr)

            doc = tomllib.loads(cargo_toml.read_text())
            self.assertIn("egui_dock", doc["workspace"]["dependencies"])
            self.assertEqual(
                doc["workspace"]["dependencies"]["egui_dock"]["path"],
                "vendor/egui_dock",
            )

    def test_strip_winit_wayland(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            src = Path(tmp_dir)
            winit_dir = src / "vendor" / "winit"
            winit_dir.mkdir(parents=True, exist_ok=True)
            winit_toml = winit_dir / "Cargo.toml"
            winit_toml.write_text(
                """[package]
name = "winit"

[features]
default = [
    "x11",
    "wayland",
]
wayland = [
    "sctk-adwaita",
    "crossfont",
]
wayland-dlopen = []
"""
            )

            cmd = [
                sys.executable,
                str(PATCHES_SCRIPT),
                "strip-winit-wayland",
                str(src),
            ]
            res = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(res.returncode, 0, res.stderr)

            doc = tomllib.loads(winit_toml.read_text())
            self.assertNotIn("wayland", doc["features"].get("default", []))
            self.assertEqual(doc["features"]["wayland"], [])


if __name__ == "__main__":
    unittest.main()
