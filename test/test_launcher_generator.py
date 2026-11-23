#!/usr/bin/env python3
"""Unit tests for lib/launcher_generator.py."""

import py_compile
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
GENERATOR_SCRIPT = ROOT / "lib" / "launcher_generator.py"


class TestLauncherGenerator(unittest.TestCase):
    def test_script_exists_and_executable(self):
        self.assertTrue(GENERATOR_SCRIPT.is_file(), f"{GENERATOR_SCRIPT} does not exist")
        import os

        self.assertTrue(os.access(GENERATOR_SCRIPT, os.X_OK), f"{GENERATOR_SCRIPT} is not executable")

    def test_py_compile_syntax(self):
        compiled = py_compile.compile(str(GENERATOR_SCRIPT), doraise=True)
        self.assertIsNotNone(compiled)

    def test_cli_usage_on_missing_args(self):
        result = subprocess.run(
            [sys.executable, str(GENERATOR_SCRIPT)],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("Usage: launcher_generator.py", result.stderr)

    def test_launcher_generation_on_wordcraft(self):
        wordcraft_src = ROOT / "source" / "wordcraft"
        if not wordcraft_src.is_dir():
            self.skipTest(f"source/wordcraft not present at {wordcraft_src}")

        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_root = Path(tmp_dir)
            tmp_src = tmp_root / "wordcraft"
            # Copy source/wordcraft to temp dir to avoid mutating main workspace
            shutil.copytree(
                wordcraft_src,
                tmp_src,
                ignore=shutil.ignore_patterns(".git", "target", "apps/*-android"),
            )

            cmd = [
                sys.executable,
                str(GENERATOR_SCRIPT),
                str(ROOT),
                str(tmp_src),
                "wordcraft",
                "WordCraft",
                "34",
                "WordCraft",
                "com.artcraft.wordcraft",
                "",
                "false",
            ]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(
                result.returncode,
                0,
                f"launcher_generator.py failed: stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
            )

            android_app_dir = tmp_src / "apps" / "wordcraft-android"
            self.assertTrue(android_app_dir.is_dir(), "apps/wordcraft-android directory not created")
            self.assertTrue(
                (android_app_dir / ".artcraft-android-builder-generated").is_file(),
                ".artcraft-android-builder-generated marker missing",
            )
            self.assertTrue((android_app_dir / "Cargo.toml").is_file(), "Cargo.toml missing")
            self.assertTrue((android_app_dir / "src" / "lib.rs").is_file(), "src/lib.rs missing")
            self.assertTrue((android_app_dir / "src" / "mcp_http.rs").is_file(), "src/mcp_http.rs missing")
            self.assertTrue(
                (android_app_dir / "src" / "mobile_text_bar.rs").is_file(),
                "src/mobile_text_bar.rs missing",
            )


if __name__ == "__main__":
    unittest.main()
