#!/usr/bin/env python3
"""Unit tests for lib/toolchain.sh."""

import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent
TOOLCHAIN_SCRIPT = ROOT / "lib" / "toolchain.sh"


class TestToolchain(unittest.TestCase):
    def test_script_exists_and_executable(self):
        self.assertTrue(TOOLCHAIN_SCRIPT.is_file(), f"{TOOLCHAIN_SCRIPT} does not exist")
        self.assertTrue(os.access(TOOLCHAIN_SCRIPT, os.X_OK), f"{TOOLCHAIN_SCRIPT} is not executable")

    def test_bash_syntax(self):
        res = subprocess.run(["bash", "-n", str(TOOLCHAIN_SCRIPT)], capture_output=True, text=True)
        self.assertEqual(res.returncode, 0, f"Bash syntax check failed: {res.stderr}")

    def test_needs_saf(self):
        cmd = f'source "{TOOLCHAIN_SCRIPT}" && needs_saf wordcraft && ! needs_saf nonexistent_app_xyz'
        res = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True)
        self.assertEqual(res.returncode, 0, f"needs_saf logic check failed: {res.stderr}")

    def test_needs_apk2(self):
        cmd = f'source "{TOOLCHAIN_SCRIPT}" && needs_apk2 wordcraft && ! needs_apk2 nonexistent_app_xyz'
        res = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True)
        self.assertEqual(res.returncode, 0, f"needs_apk2 logic check failed: {res.stderr}")


if __name__ == "__main__":
    unittest.main()
