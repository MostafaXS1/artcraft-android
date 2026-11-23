#!/usr/bin/env python3
"""Unit tests for lib/android_project.py."""

import os
from pathlib import Path
import py_compile
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
PROJECT_SCRIPT = ROOT / "lib" / "android_project.py"
sys.path.insert(0, str(ROOT / "lib"))
import android_project


class TestAndroidProject(unittest.TestCase):
    def test_script_exists_and_executable(self):
        self.assertTrue(PROJECT_SCRIPT.is_file(), f"{PROJECT_SCRIPT} does not exist")
        self.assertTrue(os.access(PROJECT_SCRIPT, os.X_OK), f"{PROJECT_SCRIPT} is not executable")

    def test_py_compile_syntax(self):
        compiled = py_compile.compile(str(PROJECT_SCRIPT), doraise=True)
        self.assertIsNotNone(compiled)

    def test_version_code_standard_semver(self):
        # Formula: (2 << 24) | (major << 16) | (minor << 8) | patch
        # 0.2.1 -> (2 << 24) | (0 << 16) | (2 << 8) | 1 = 33554432 | 0 | 512 | 1 = 33554945
        code = android_project.version_code("0.2.1")
        self.assertEqual(code, (2 << 24) | (0 << 16) | (2 << 8) | 1)

        code_1 = android_project.version_code("1.0.0")
        self.assertEqual(code_1, (2 << 24) | (1 << 16) | (0 << 8) | 0)

        code_large = android_project.version_code("0.45.12")
        self.assertEqual(code_large, (2 << 24) | (0 << 16) | (45 << 8) | 12)

    def test_version_code_with_prerelease_and_build_metadata(self):
        # Splits on [.+-] and takes first 3 components
        code = android_project.version_code("0.45.0-rc.4")
        self.assertEqual(code, (2 << 24) | (0 << 16) | (45 << 8) | 0)

        code_meta = android_project.version_code("2.12.5+build.101")
        self.assertEqual(code_meta, (2 << 24) | (2 << 16) | (12 << 8) | 5)

    def test_version_code_invalid_format(self):
        with self.assertRaises(SystemExit):
            android_project.version_code("invalid")
        with self.assertRaises(SystemExit):
            android_project.version_code("1.0")

    def test_manifest_xml_generation(self):
        meta = {
            "package": "com.artcraft.testapp",
            "uses_permission": [{"name": "android.permission.INTERNET"}],
            "application": {
                "label": "Test App",
                "theme": "@style/Theme.Artcraft",
                "activity": [
                    {
                        "name": "com.artcraft.testapp.MainActivity",
                        "exported": True,
                        "intent_filter": [
                            {
                                "actions": ["android.intent.action.MAIN"],
                                "categories": ["android.intent.category.LAUNCHER"],
                            }
                        ],
                    }
                ],
            },
        }
        xml = android_project.manifest_xml(meta)
        self.assertIn('<?xml version="1.0" encoding="utf-8"?>', xml)
        self.assertIn('<manifest xmlns:android="http://schemas.android.com/apk/res/android">', xml)
        self.assertIn('<uses-permission android:name="android.permission.INTERNET" />', xml)
        self.assertIn('android:label="Test App"', xml)
        self.assertIn('android:name="com.artcraft.testapp.MainActivity"', xml)
        self.assertIn('android:exported="true"', xml)
        self.assertIn('<action android:name="android.intent.action.MAIN" />', xml)
        self.assertIn('<category android:name="android.intent.category.LAUNCHER" />', xml)

    def test_manifest_xml_escaping(self):
        meta = {
            "application": {
                "label": 'Test "App" & Co',
            }
        }
        xml = android_project.manifest_xml(meta)
        self.assertIn('android:label="Test &quot;App&quot; &amp; Co"', xml)

    def test_build_gradle_generation(self):
        gradle = android_project.build_gradle(
            namespace="com.artcraft.wordcraft",
            min_sdk=26,
            target_sdk=34,
            code=33554945,
            version="0.2.1",
            legacy_packaging=False,
        )
        self.assertIn("namespace 'com.artcraft.wordcraft'", gradle)
        self.assertIn("minSdk 26", gradle)
        self.assertIn("targetSdk 34", gradle)
        self.assertIn("versionCode 33554945", gradle)
        self.assertIn("versionName '0.2.1'", gradle)
        self.assertIn("com.android.application", gradle)
        self.assertNotIn("useLegacyPackaging = true", gradle)

        # Legacy packaging enabled
        gradle_legacy = android_project.build_gradle(
            namespace="com.artcraft.ffmpegapp",
            min_sdk=26,
            target_sdk=34,
            code=1,
            version="1.0.0",
            legacy_packaging=True,
        )
        self.assertIn("useLegacyPackaging = true", gradle_legacy)

    def test_settings_gradle_and_constants(self):
        settings = android_project.settings_gradle("wordcraft")
        self.assertIn("pluginManagement {", settings)
        self.assertIn("dependencyResolutionManagement {", settings)
        self.assertIn("Theme.Artcraft", android_project.THEMES_XML)
        self.assertIn("android.useAndroidX=true", android_project.GRADLE_PROPERTIES)


if __name__ == "__main__":
    unittest.main()
