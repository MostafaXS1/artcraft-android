#!/usr/bin/env python3
"""Unit tests for lib/catalog.sh."""

import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent
CATALOG_SCRIPT = ROOT / "lib" / "catalog.sh"


class TestCatalog(unittest.TestCase):
    def test_script_exists_and_executable(self):
        self.assertTrue(CATALOG_SCRIPT.is_file(), f"{CATALOG_SCRIPT} does not exist")
        self.assertTrue(os.access(CATALOG_SCRIPT, os.X_OK), f"{CATALOG_SCRIPT} is not executable")

    def test_bash_syntax(self):
        res = subprocess.run(["bash", "-n", str(CATALOG_SCRIPT)], capture_output=True, text=True)
        self.assertEqual(res.returncode, 0, f"Bash syntax check failed: {res.stderr}")

    def test_resolve_catalog_wordcraft(self):
        cmd = f'source "{CATALOG_SCRIPT}" && resolve_app_catalog wordcraft && echo "$TITLE|$REPO|$SRC"'
        res = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True)
        self.assertEqual(res.returncode, 0, res.stderr)
        out = res.stdout.strip()
        self.assertEqual(out, "WordCraft|https://github.com/storytold/wordcraft.git|source/wordcraft")

    def test_resolve_catalog_subcraft(self):
        cmd = f'source "{CATALOG_SCRIPT}" && resolve_app_catalog subcraft && echo "$TITLE|$REPO|$SRC"'
        res = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True)
        self.assertEqual(res.returncode, 0, res.stderr)
        out = res.stdout.strip()
        self.assertEqual(out, "SubCraft|https://github.com/Aalzard/subcraft.git|source/subcraft")


if __name__ == "__main__":
    unittest.main()
