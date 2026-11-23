#!/usr/bin/env python3
"""Unit tests for lib/upstream_patches.py."""

import os
from pathlib import Path
import py_compile
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
PATCHES_SCRIPT = ROOT / "lib" / "upstream_patches.py"


class TestUpstreamPatches(unittest.TestCase):
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
        self.assertIn("Usage: upstream_patches.py", result.stderr)

    def test_noop_on_clean_repo(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_src = Path(tmp_dir)
            cmd = [
                sys.executable,
                str(PATCHES_SCRIPT),
                str(tmp_src),
                "wordcraft",
                "WordCraft",
            ]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.stdout.strip(), "")

    def test_patch_application_and_idempotence(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_src = Path(tmp_dir)
            target_file = tmp_src / "crates" / "ui-egui" / "src" / "window_ui.rs"
            target_file.parent.mkdir(parents=True, exist_ok=True)
            original_content = (
                "// Header\n"
                "        let WindowUi { $($(#[$attr])* $field: _),* } = &*$w;\n"
                "// Footer\n"
            )
            target_file.write_text(original_content)

            cmd = [
                sys.executable,
                str(PATCHES_SCRIPT),
                str(tmp_src),
                "wordcraft",
                "WordCraft",
            ]
            # 1. First run should apply the patch
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0)
            self.assertIn("Repaired upstream compile errors still present in WordCraft", result.stdout)
            self.assertIn("swap_fields pattern", result.stdout)

            patched_content = target_file.read_text()
            self.assertIn("let WindowUi { $($(#[$attr])* $field: _),*, .. } = &*$w;", patched_content)

            # 2. Second run should be a silent no-op (idempotent)
            result2 = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result2.returncode, 0)
            self.assertEqual(result2.stdout.strip(), "")
            self.assertEqual(target_file.read_text(), patched_content)


if __name__ == "__main__":
    unittest.main()
