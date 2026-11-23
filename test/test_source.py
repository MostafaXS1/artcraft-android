#!/usr/bin/env python3
"""Unit tests for lib/source.sh."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
SOURCE_SCRIPT = ROOT / "lib" / "source.sh"


class TestSource(unittest.TestCase):
    def test_script_exists_and_executable(self):
        self.assertTrue(SOURCE_SCRIPT.is_file(), f"{SOURCE_SCRIPT} does not exist")
        self.assertTrue(os.access(SOURCE_SCRIPT, os.X_OK), f"{SOURCE_SCRIPT} is not executable")

    def test_bash_syntax(self):
        res = subprocess.run(["bash", "-n", str(SOURCE_SCRIPT)], capture_output=True, text=True)
        self.assertEqual(res.returncode, 0, f"Bash syntax check failed: {res.stderr}")

    def test_source_tree_is_complete_success(self):
        """Verify source_tree_is_complete returns 0 when all tracked files exist on disk."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            repo_dir = Path(tmp_dir) / "test_repo"
            repo_dir.mkdir()
            # Initialize git repo with crates/ and apps/ tracked files
            subprocess.run(["git", "init", "-q"], cwd=repo_dir, check=True)
            subprocess.run(["git", "config", "user.name", "Test"], cwd=repo_dir, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=repo_dir, check=True)

            crates_file = repo_dir / "crates" / "lib.rs"
            crates_file.parent.mkdir(parents=True, exist_ok=True)
            crates_file.write_text("// test crates")

            subprocess.run(["git", "add", "."], cwd=repo_dir, check=True)
            subprocess.run(["git", "commit", "-q", "-m", "initial commit"], cwd=repo_dir, check=True)

            cmd = f'source "{SOURCE_SCRIPT}" && source_tree_is_complete "{repo_dir}"'
            res = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True)
            self.assertEqual(res.returncode, 0, f"Expected 0 (complete tree), got {res.returncode}")

    def test_source_tree_is_complete_missing_file(self):
        """Verify source_tree_is_complete returns non-zero when a tracked file is missing on disk."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            repo_dir = Path(tmp_dir) / "test_repo"
            repo_dir.mkdir()
            subprocess.run(["git", "init", "-q"], cwd=repo_dir, check=True)
            subprocess.run(["git", "config", "user.name", "Test"], cwd=repo_dir, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=repo_dir, check=True)

            crates_file = repo_dir / "crates" / "lib.rs"
            crates_file.parent.mkdir(parents=True, exist_ok=True)
            crates_file.write_text("// test crates")

            subprocess.run(["git", "add", "."], cwd=repo_dir, check=True)
            subprocess.run(["git", "commit", "-q", "-m", "initial commit"], cwd=repo_dir, check=True)

            # Delete the tracked file from disk without git rm (simulating interrupted clone/checkout)
            crates_file.unlink()

            cmd = f'source "{SOURCE_SCRIPT}" && source_tree_is_complete "{repo_dir}"'
            res = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True)
            self.assertNotEqual(res.returncode, 0, "Expected non-zero when files are missing on disk")

    def test_source_tree_is_complete_not_a_git_repo(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            cmd = f'source "{SOURCE_SCRIPT}" && source_tree_is_complete "{tmp_dir}"'
            res = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True)
            self.assertNotEqual(res.returncode, 0, "Expected non-zero for directory without .git")


if __name__ == "__main__":
    unittest.main()
