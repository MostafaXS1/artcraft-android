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

    def test_vendoring_egui_36_app_consistency(self):
        """Ensure the set of apps vendoring egui-winit-0.36 and egui-wgpu-0.36 in lib/vendoring.sh are identical."""
        vendoring_sh = ROOT / "lib" / "vendoring.sh"
        self.assertTrue(vendoring_sh.is_file())
        text = vendoring_sh.read_text()

        # Find egui-winit 0.36 match block
        winit_match = re.search(r'if \[\[(.*?)\]\]; then\s+rm -rf "\$src_dir/vendor/egui-winit"\s+cp -a "\$ROOT/vendor/egui-winit-0.36"', text)
        self.assertIsNotNone(winit_match, "Could not find egui-winit-0.36 branch in lib/vendoring.sh")
        winit_apps = set(re.findall(r'"\$APP" == "([^"]+)"', winit_match.group(1)))

        # Find egui-wgpu 0.36 match block
        wgpu_match = re.search(r'elif \[\[(.*?)\]\]; then\s+rm -rf "\$src_dir/vendor/egui-wgpu"\s+cp -a "\$ROOT/vendor/egui-wgpu-0.36"', text)
        self.assertIsNotNone(wgpu_match, "Could not find egui-wgpu-0.36 branch in lib/vendoring.sh")
        wgpu_apps = set(re.findall(r'"\$APP" == "([^"]+)"', wgpu_match.group(1)))

        self.assertEqual(
            winit_apps,
            wgpu_apps,
            f"Mismatch between egui-winit-0.36 and egui-wgpu-0.36 apps in lib/vendoring.sh: {winit_apps ^ wgpu_apps}",
        )
        self.assertIn("clothcraft", winit_apps)

    def test_touch_gestures_mousewheel_source_sanitization(self):
        """Verify that MouseWheelSource stripping regex in build-common.sh and launcher_generator.py strips Trackpad/Unknown."""
        template_tg = (ROOT / "templates" / "android" / "touch_gestures.rs").read_text()
        self.assertIn("source: egui::MouseWheelSource::", template_tg)

        # 1. Test build-common.sh sed replacement logic:
        # sed -i '/source: egui::MouseWheelSource::/d'
        sed_applied = re.sub(r'(?m)^.*source: egui::MouseWheelSource::.*\n?', '', template_tg)
        self.assertNotIn("source: egui::MouseWheelSource::", sed_applied)
        self.assertIn("phase: TouchPhase::Move,", sed_applied)

        # 2. Test launcher_generator.py regex replacement logic
        py_regex = re.compile(r'(?m)^\s*source:\s*egui::MouseWheelSource::.*?\n')
        regex_applied = py_regex.sub('', template_tg)
        self.assertNotIn("source: egui::MouseWheelSource::", regex_applied)
        self.assertIn("phase: TouchPhase::Move,", regex_applied)

    def test_photocraft_preset_store_and_persistence_wiring(self):
        """Verify PhotoCraft wires preset_store, persistence flushing, ui.ron, and SAF save naming."""
        pc_lib = (ADAPTERS_DIR / "photocraft" / "src" / "lib.rs").read_text()
        pc_java = (
            ADAPTERS_DIR
            / "photocraft"
            / "java"
            / "com"
            / "artcraft"
            / "photocraft"
            / "PhotoCraftActivity.java"
        ).read_text()

        self.assertIn("photocraft_engine::preset_store::open_dir_async", pc_lib)
        self.assertIn("preset_store:", pc_lib)
        self.assertIn("self.inner.flush_persistence()", pc_lib)
        self.assertIn("self.inner.on_exit()", pc_lib)
        self.assertIn("autosave_now(&mut self.inner)", pc_lib)
        self.assertIn("self.inner.session.sync_preset_store()", pc_lib)
        self.assertIn('dir.join("ui.ron")', pc_lib)

        for symbol in (
            "extensionLast",
            "renameDocument",
            'startsWith(stagedFile.getName() + ".")',
        ):
            self.assertIn(symbol, pc_java, f"Missing {symbol} in PhotoCraftActivity.java")

    def test_vectorcraft_saf_folder_picker_and_save_export(self):
        """Verify VectorCraft wires PickRequest::Folder and handles SAF save filename suffixes."""
        vc_lib = (ADAPTERS_DIR / "vectorcraft" / "src" / "lib.rs").read_text()
        vc_java = (
            ADAPTERS_DIR
            / "vectorcraft"
            / "java"
            / "com"
            / "artcraft"
            / "vectorcraft"
            / "VectorCraftActivity.java"
        ).read_text()

        # Folder picker must be wired to vcRequestFolder instead of returning None
        self.assertIn("fn request_folder(", vc_lib)
        self.assertIn('jni_str!("vcRequestFolder")', vc_lib)
        self.assertIn("bridge.request_folder(id)", vc_lib)
        self.assertNotIn("PickRequest::Folder => return None", vc_lib)

        # Save requests must never pass wildcard */* to ACTION_CREATE_DOCUMENT
        self.assertIn("fn save_mime_for_name(", vc_lib)
        self.assertIn('"*/*".equals(mime)', vc_java)

        # Java Activity must implement folder picking and duplicate-suffix normalization for Export As
        for symbol in (
            "REQ_FOLDER",
            "vcRequestFolder",
            "treeToPath",
            "extensionLast",
            "renameDocument",
            'startsWith(stagedFile.getName() + ".")',
        ):
            self.assertIn(symbol, vc_java, f"Missing {symbol} in VectorCraftActivity.java")

    def test_on_check_is_text_editor_does_not_require_focus_and_restores_soft_input(self):
        """Ensure virtual keyboard language/subtype switching works by decoupling onCheckIsTextEditor from isFocused."""
        java_files = list((ROOT / "adapters").glob("*/java/**/*.java"))
        java_files.append(ROOT / "templates" / "android" / "SafActivity.java")
        java_files.append(ROOT / "templates" / "android" / "__CLASS__.java")

        checked_text_editors = 0
        for f in java_files:
            text = f.read_text()
            if "onCheckIsTextEditor" in text:
                # Must never check isFocused() inside onCheckIsTextEditor(), which terminates InputConnection on dialogs
                self.assertNotIn(
                    "isFocused() && getVisibility()",
                    text,
                    f"{f.relative_to(ROOT)} onCheckIsTextEditor() must not require isFocused()",
                )
                if "class TextInputView" not in text:
                    self.assertIn(
                        "return getVisibility() == View.VISIBLE;",
                        text,
                        f"{f.relative_to(ROOT)} onCheckIsTextEditor() must return getVisibility() == View.VISIBLE;",
                    )
                    # Window focus recovery must re-assert soft keyboard visibility
                    self.assertIn(
                        "imm.showSoftInput(inputView,",
                        text,
                        f"{f.relative_to(ROOT)} onWindowFocusChanged must restore soft input via imm.showSoftInput",
                    )
                    self.assertIn(
                        "public void imeShowImplicit()",
                        text,
                        f"{f.relative_to(ROOT)} must expose imeShowImplicit() for hardware-keyboard IME binding",
                    )
                    self.assertIn(
                        "InputMethodManager.SHOW_IMPLICIT",
                        text,
                        f"{f.relative_to(ROOT)} must pass SHOW_IMPLICIT when showing IME implicitly",
                    )
                    checked_text_editors += 1

        self.assertEqual(checked_text_editors, 25, f"Expected exactly 25 activities checked, got {checked_text_editors}")

        # ArtCraft Launcher input view & activity checks
        launcher_input = (
            ROOT
            / "launcher"
            / "apps"
            / "artcraft-launcher-android"
            / "android"
            / "app"
            / "src"
            / "main"
            / "java"
            / "com"
            / "artcraft"
            / "launcher"
            / "ArtCraftInputView.java"
        ).read_text()
        self.assertNotIn("isFocused() && getVisibility()", launcher_input)
        self.assertIn("return getVisibility() == VISIBLE;", launcher_input)

        launcher_activity = (
            ROOT
            / "launcher"
            / "apps"
            / "artcraft-launcher-android"
            / "android"
            / "app"
            / "src"
            / "main"
            / "java"
            / "com"
            / "artcraft"
            / "launcher"
            / "ArtCraftLauncherActivity.java"
        ).read_text()
        self.assertIn("imm.showSoftInput(inputView, 0);", launcher_activity)

    def test_ime_not_hidden_when_android_window_unfocused(self):
        """Ensure egui-winit and poll_ime do not call imeHide/set_ime_allowed(false) while window focus is lost."""
        for rel in (
            "vendor/egui-winit/src/lib.rs",
            "vendor/egui-winit-0.36/src/lib.rs",
            "vendor/egui-winit-0.34/src/lib.rs",
            "launcher/vendor-egui-winit-0.37/src/lib.rs",
        ):
            text = (ROOT / rel).read_text()
            self.assertIn(
                "!self.egui_input.focused",
                text,
                f"{rel} must guard set_ime_allowed(false) against unfocused Android windows",
            )
        for adapter in ("photocraft", "vectorcraft", "subcraft"):
            text = (ROOT / "adapters" / adapter / "src" / "lib.rs").read_text()
            self.assertIn(
                "!(!editing && !ctx.input(|i| i.focused))",
                text,
                f"adapters/{adapter}/src/lib.rs poll_ime must guard hide_soft_input when unfocused",
            )

    def test_hardware_keyboard_binds_ime_implicitly(self):
        """Ensure show_soft_input binds IME via imeShowImplicit instead of returning early on hardware keyboards."""
        native_act = (
            ROOT / "vendor" / "android-activity" / "src" / "native_activity" / "mod.rs"
        ).read_text()
        self.assertIn("imeShowImplicit", native_act)
        self.assertIn("hardware_keyboard_pointing", native_act)
        # Must not early-return before attaching to JVM and binding InputConnection
        fn_start = native_act.index("pub fn show_soft_input(")
        fn_body = native_act[fn_start : fn_start + 700]
        self.assertNotIn("return;", fn_body)

    def test_saf_commit_truncates_output_stream_before_writing(self):
        """Ensure SAF commits open content:// URIs in truncating mode ('wt'/'rwt') before falling back to 'w'."""
        java_files = list((ROOT / "adapters").glob("*/java/**/*.java"))
        java_files.append(ROOT / "templates" / "android" / "SafActivity.java")
        java_files.append(ROOT / "templates" / "android" / "__CLASS__.java")

        checked = 0
        for f in java_files:
            text = f.read_text()
            if "openOutputStream(" not in text:
                continue
            rel = f.relative_to(ROOT)
            self.assertNotIn(
                'openOutputStream(uri, "w")',
                text,
                f"{rel} must not call openOutputStream(uri, \"w\") directly without trying \"wt\" first",
            )
            wt_idx = text.find('"wt"')
            self.assertNotEqual(wt_idx, -1, f"{rel} must try truncating mode \"wt\" when opening SAF output stream")
            w_idx = text.find('"w"', wt_idx)
            self.assertNotEqual(w_idx, -1, f"{rel} must keep \"w\" fallback after \"wt\"")
            checked += 1

        self.assertEqual(checked, 26, f"Expected 26 Java files with SAF openOutputStream checked, got {checked}")

if __name__ == "__main__":
    unittest.main()
