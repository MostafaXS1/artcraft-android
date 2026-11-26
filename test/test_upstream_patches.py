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
SCROLLING_SCRIPT = ROOT / "lib" / "scrolling.sh"


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

    def test_wordcraft_autosave_save_in_place_patch(self):
        for rel_path, indent in (
            ("crates/ui-egui/src/chrome/title_bar.rs", "            "),
            ("crates/ui-egui/src/chrome.rs", "                            "),
        ):
            with tempfile.TemporaryDirectory() as tmp_dir:
                tmp_src = Path(tmp_dir)
                target_file = tmp_src / rel_path
                target_file.parent.mkdir(parents=True, exist_ok=True)
                target_file.write_text(
                    f"{indent}// AutoSave comes on once Save As has saved (maybe on a later frame, #94).\n"
                    f"{indent}Some(b) if b.save_as_helps() => {{\n"
                )
                cmd = [sys.executable, str(PATCHES_SCRIPT), str(tmp_src), "wordcraft", "WordCraft"]
                result = subprocess.run(cmd, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0)
                self.assertIn("AutoSave toggle saves opened file in place", result.stdout)
                patched = target_file.read_text()
                self.assertIn("Some(crate::AutoSaveBlock::NotSavedHere) => {", patched)
                self.assertIn('app.run("file.save", json!({}))', patched)

                # Idempotent on second run
                result2 = subprocess.run(cmd, capture_output=True, text=True)
                self.assertEqual(result2.returncode, 0)
                self.assertEqual(result2.stdout.strip(), "")
                self.assertEqual(target_file.read_text(), patched)

    def test_diagramcraft_spellcheck_patch(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_src = Path(tmp_dir)
            target_file = tmp_src / "crates" / "dc" / "src" / "spellcheck.rs"
            target_file.parent.mkdir(parents=True, exist_ok=True)
            original_content = (
                "fn system_dict_dirs() -> Vec<PathBuf> {\n"
                "    #[cfg(target_os = \"linux\")]\n"
                "    {\n"
                "        vec![PathBuf::from(\"/usr/share/hunspell\")]\n"
                "    }\n"
                "    #[cfg(target_os = \"macos\")]\n"
                "    {\n"
                "        vec![PathBuf::from(\"/Library/Spelling\")]\n"
                "    }\n"
                "    #[cfg(windows)]\n"
                "    {\n"
                "        Vec::new()\n"
                "    }\n"
                "}\n"
            )
            target_file.write_text(original_content)

            cmd = [
                sys.executable,
                str(PATCHES_SCRIPT),
                str(tmp_src),
                "diagramcraft",
                "DiagramCraft",
            ]
            # 1. First run applies patch
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0)
            self.assertIn("Repaired upstream compile errors still present in DiagramCraft", result.stdout)
            self.assertIn("system_dict_dirs android fallback", result.stdout)

            patched_content = target_file.read_text()
            self.assertIn('#[cfg(target_os = "android")]\n    {\n        Vec::new()\n    }', patched_content)

            # 2. Second run is a silent no-op
            result2 = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result2.returncode, 0)
            self.assertEqual(result2.stdout.strip(), "")
            self.assertEqual(target_file.read_text(), patched_content)

    def test_filmcraft_platform_rayon_ndk_patch(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_src = Path(tmp_dir)
            target_cargo = tmp_src / "crates" / "platform" / "Cargo.toml"
            target_cargo.parent.mkdir(parents=True, exist_ok=True)
            (tmp_src / "crates" / "platform" / "src").mkdir(parents=True, exist_ok=True)
            original_content = (
                '[package]\nname = "filmcraft-platform"\n\n'
                '[target.\'cfg(target_os = "linux")\'.dependencies]\nlibloading = "0.8"\n\n'
                '[dependencies]\n'
            )
            target_cargo.write_text(original_content)

            cmd = [
                sys.executable,
                str(PATCHES_SCRIPT),
                str(tmp_src),
                "filmcraft",
                "FilmCraft",
            ]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, f"STDOUT: {result.stdout}\nSTDERR: {result.stderr}")
            patched_content = target_cargo.read_text()
            self.assertIn('rayon = { workspace = true }', patched_content)
            self.assertIn('ndk = { version = "0.9"', patched_content)

    def test_photocraft_cursor_and_persistence_patches(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_src = Path(tmp_dir)
            mem_system = tmp_src / "crates" / "raster" / "src" / "memory" / "system.rs"
            tool_cursor = tmp_src / "crates" / "ui-egui" / "src" / "canvas" / "tool_cursor.rs"
            last_brush = tmp_src / "crates" / "ui-egui" / "src" / "last_brush.rs"
            tool_memory = tmp_src / "crates" / "ui-egui" / "src" / "tool_memory.rs"
            app_rs = tmp_src / "crates" / "ui-egui" / "src" / "app.rs"
            for f in (mem_system, tool_cursor, last_brush, tool_memory, app_rs):
                f.parent.mkdir(parents=True, exist_ok=True)

            mem_system.write_text(
                '#[cfg(target_os = "linux")]\nmod a {}\n'
                '#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))] fn b() {}\n'
                '#[cfg(target_os = "linux")] fn c() {}\n'
                '#[cfg(not(target_os = "linux"))] fn d() {}\n'
                '#[cfg(target_os = "linux")] mod linux {\n'
                '        let (cgroup_ram, cgroup_swap) = match cgroup_limits(proc_root, sys_root) {\n'
                '            CgroupLimits::NotConstrained => (None, None),\n'
                '            CgroupLimits::Limited { ram, swap } => (ram, swap),\n'
                '            CgroupLimits::Unreadable => return SystemMemory { available_ram: Some(0), swap_free: Some(0), commit_headroom: None, constraint_unreadable: true },\n'
                '        };\n'
                '}\n'
            )

            tool_cursor.write_text(
                'fn native(ctx: &egui::Context) -> bool {\n'
                '    cfg!(not(target_arch = "wasm32")) && ctx.viewport_id() == egui::ViewportId::ROOT && !scaled_wayland(ctx)\n'
                '}\n'
            )
            last_brush.write_text(
                'pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {\n'
                '    if !ctx.input(|i| i.pointer.any_down()) {\n'
                '        photocraft_engine::brush_cmds::remember_brush(&mut app.session);\n'
                '    }\n'
                '}\n'
            )
            tool_memory.write_text(
                '    persist_brushes(app);\n'
                '    if app.prefs_rt.tool_options.as_ref() == Some(&app.ui.tool_options) {\n'
                '        return;\n'
                '    }\n'
                '    app.prefs_rt.tool_options = Some(app.ui.tool_options.clone());\n'
                '    let now = changed(&app.ui.tool_options);\n'
                '    if app.session.prefs().tool_options != now {\n'
                '        app.session.prefs.edit(|p| p.tool_options = now);\n'
                '    }\n'
                '}\n'
                'fn persist_brushes(app: &mut PhotocraftApp) {\n'
                '    let (owner, brushes) = (saved_owner(app), &app.ui.tool_brushes);\n'
                '    if app.prefs_rt.tool_brushes.as_ref().is_some_and(|(o, b)| *o == owner && b == brushes) {\n'
                '        return;\n'
                '    }\n'
                '    let old = app.session.prefs().tool_brushes.get("brushes").and_then(Value::as_object).cloned().unwrap_or_default();\n'
                '    let mut saved = Map::new();\n'
                '    for (tool, brush) in brushes {\n'
                '        // The owner\'s brush is the session brush, remembered as `last_brush`.\n'
                '        if *tool == owner {\n'
                '            continue;\n'
                '        }\n'
                '        let Ok(Value::String(key)) = serde_json::to_value(tool) else { continue };\n'
                '        if let Some(v) = photocraft_engine::brush_cmds::remembered_settings(brush).or_else(|| old.get(&key).cloned()) {\n'
                '            saved.insert(key, v);\n'
                '        }\n'
                '    }\n'
                '    // Nothing to remember while the Brush owns the session brush and no other tool has its own.\n'
                '    let now = match serde_json::to_value(owner) {\n'
                '        Ok(o) if !saved.is_empty() || owner != Tool::Brush => json!({"owner": o, "brushes": saved}),\n'
                '        _ => Value::Null,\n'
                '    };\n'
                '    app.prefs_rt.tool_brushes = Some((owner, brushes.clone()));\n'
                '    if app.session.prefs().tool_brushes != now {\n'
                '        app.session.prefs.edit(|p| p.tool_brushes = now);\n'
                '    }\n'
                '}\n'
            )
            app_rs.write_text(
                'impl eframe::App for PhotocraftApp {\n'
                '    fn on_exit(&mut self) {\n'
                '        let _ = tool_memory::save_on_exit(self);\n'
                '    }\n'
                '}\n'
            )

            cmd = [sys.executable, str(PATCHES_SCRIPT), str(tmp_src), "photocraft", "PhotoCraft"]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, f"STDOUT: {result.stdout}\nSTDERR: {result.stderr}")
            mem_patched = mem_system.read_text()
            self.assertNotIn('#[cfg(target_os = "linux")]', mem_patched)
            self.assertNotIn('#[cfg(not(target_os = "linux"))]', mem_patched)
            self.assertEqual(mem_patched.count('#[cfg(any(target_os = "linux", target_os = "android"))]'), 3)
            self.assertIn('if cfg!(target_os = "android")', mem_patched)
            self.assertIn('target_os = "android"', tool_cursor.read_text())
            self.assertIn("ctx.request_repaint()", last_brush.read_text())
            self.assertIn("fn persist_brushes(app: &mut PhotocraftApp, ctx: &egui::Context)", tool_memory.read_text())
            self.assertEqual(tool_memory.read_text().count("ctx.request_repaint()"), 2)
            self.assertIn("pub fn flush_persistence(&mut self)", app_rs.read_text())

            # Idempotent on re-run
            result2 = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result2.returncode, 0)
            self.assertEqual(result2.stdout.strip(), "")

    def test_vectorcraft_modal_popup_sublayer_patch(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_src = Path(tmp_dir)
            modal_rs = tmp_src / "crates" / "ui-egui" / "src" / "dialogs" / "modal.rs"
            modal_rs.parent.mkdir(parents=True, exist_ok=True)
            modal_rs.write_text(
                "    let shown = egui::Window::new(title)\n"
                "        .id(id)\n"
                "        .order(egui::Order::Foreground)\n"
                "        .show(ctx, add)?;\n"
                "    keep_rest(ctx, id, shown.response.rect.height());\n"
                "    shown.inner\n"
            )

            cmd = [sys.executable, str(PATCHES_SCRIPT), str(tmp_src), "vectorcraft", "VectorCraft"]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, f"STDOUT: {result.stdout}\nSTDERR: {result.stderr}")
            self.assertIn("keep modal popups above modal dialog", result.stdout)
            patched = modal_rs.read_text()
            self.assertIn("layer.open_popups.iter()", patched)
            self.assertIn("ctx.set_sublayer(dialog_layer, popup_layer);", patched)

            # Idempotent on re-run
            result2 = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result2.returncode, 0)
            self.assertEqual(result2.stdout.strip(), "")

    def test_vectorcraft_two_finger_rotation_and_scrolling_patches(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_src = Path(tmp_dir)
            input_rs = tmp_src / "crates" / "ui-egui" / "src" / "canvas" / "input.rs"
            view_rs = tmp_src / "crates" / "ui-egui" / "src" / "canvas" / "view.rs"
            input_rs.parent.mkdir(parents=True, exist_ok=True)
            input_rs.write_text(
                "    if resp.hovered() {\n"
                "        if (factor - 1.0).abs() > 1e-6\n"
                "            && let (Some(p), Some(vm)) = (hover, app.view_mut())\n"
                "        {\n"
                "            zoom_about(vm, rect, p, vm.zoom * factor);\n"
                "        }\n"
                "        if scroll != egui::Vec2::ZERO {\n"
                "            scroll_view(app, ui, scroll);\n"
                "        }\n"
                "        if let Some(p) = hover\n"
                "            && let Some(deg) = trackpad_turn(ui, app.session.prefs.trackpad_rotate_view)\n"
                "            && let Some(vm) = app.view_mut()\n"
                "        {\n"
                "            rotate_about(vm, rect, p, deg);\n"
                "        }\n"
                "    }\n"
            )
            view_rs.write_text(
                "    let (deg, now) = ui.input(|i| {\n"
                "        let rad: f32 =\n"
                "            if i.multi_touch().is_some() { 0.0 } else { i.events.iter().map(|e| if let egui::Event::Rotate(r) = e { *r } else { 0.0 }).sum() };\n"
                "        (f64::from(rad).to_degrees(), i.time)\n"
                "    });\n"
                "    let turn = if t.turning {\n"
                "        Some(deg)\n"
                "    } else if t.total.abs() >= ROTATE_DEAD_ZONE {\n"
                "        t.turning = true;\n"
                "        Some(t.total)\n"
                "    } else {\n"
                "        None\n"
                "    };\n"
                "                } else if *unit == egui::MouseWheelUnit::Point || !animate {\n"
                "                    zoom *= (f64::from(d.y) * WHEEL_ZOOM).exp();\n"
                "                    scroll.x += d.x;\n"
                "                } else {\n"
            )

            cmd = ["bash", str(SCROLLING_SCRIPT), str(tmp_src), "vectorcraft", "VectorCraft"]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, f"STDOUT: {result.stdout}\nSTDERR: {result.stderr}")
            self.assertIn("touch rotation and zoom pivot", result.stdout)
            self.assertIn("touch rotation recognition", result.stdout)
            self.assertIn("touch rotation responsiveness", result.stdout)
            self.assertIn("trackpad/touch 2-finger panning", result.stdout)

            patched_input = input_rs.read_text()
            self.assertIn("let pivot = hover.or_else(|| pointer.interact_pos())", patched_input)
            self.assertIn("rotate_about(vm, rect, pivot, deg);", patched_input)

            patched_view = view_rs.read_text()
            self.assertIn("t.turning || cfg!(target_os = \"android\")", patched_view)
            self.assertIn("*source == egui::MouseWheelSource::Trackpad || cfg!(target_os = \"android\")", patched_view)

            # Idempotent on re-run
            result2 = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result2.returncode, 0)
            self.assertEqual(result2.stdout.strip(), "")

    def test_materialcraft_two_finger_pan_scrolling_patch(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_src = Path(tmp_dir)
            cam_rs = tmp_src / "crates" / "ui-egui" / "src" / "graph" / "graph_camera.rs"
            cam_rs.parent.mkdir(parents=True, exist_ok=True)
            cam_rs.write_text(
                "    /// Zoom around the pointer by the wheel or a pinch. `zoom_direction` is -1 for inverted zoom.\n"
                "    pub(crate) fn zoom_from_input(&mut self, ui: &Ui, view: Rect, zoom_direction: f32) {\n"
                "        const PINCH_TO_SCROLL_POINTS: f32 = 200.0;\n"
                "        const ZOOM_PER_SCROLL_POINT: f32 = 0.0025;\n"
                "\n"
                "        self.zoom = self.zoom.clamp(crate::state::GraphCamera::ZOOM.0, crate::state::GraphCamera::ZOOM.1);\n"
                "        if !ui.rect_contains_pointer(view) {\n"
                "            return;\n"
                "        }\n"
                "\n"
                "        let Some(pointer) = ui.input(|input| input.pointer.hover_pos()).filter(|pointer| view.contains(*pointer)) else { return };\n"
                "\n"
                "        let scroll = zoom_direction * ui.input(|input| input.smooth_scroll_delta.y + input.zoom_delta().ln() * PINCH_TO_SCROLL_POINTS);\n"
                "        if scroll.is_finite() && scroll != 0.0 {\n"
                "            let before = self.to_graph(pointer);\n"
                "            self.zoom = (self.zoom * (scroll * ZOOM_PER_SCROLL_POINT).exp()).clamp(crate::state::GraphCamera::ZOOM.0, crate::state::GraphCamera::ZOOM.1);\n"
                "            self.pan = before.to_vec2() - (pointer - self.origin) / self.zoom;\n"
                "        }\n"
                "    }\n"
            )

            cmd = ["bash", str(SCROLLING_SCRIPT), str(tmp_src), "materialcraft", "MaterialCraft"]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, f"STDOUT: {result.stdout}\nSTDERR: {result.stderr}")
            self.assertIn("two-finger pan and touch zoom pivot", result.stdout)
            patched = cam_rs.read_text()
            self.assertIn("egui::MouseWheelSource::Trackpad", patched)
            self.assertIn("self.pan -= pan_delta / self.zoom;", patched)

            # Idempotent on re-run
            result2 = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result2.returncode, 0)
            self.assertEqual(result2.stdout.strip(), "")

        # Also verify the release-commit (65f7bfd3) 2-argument zoom_from_input + gesture.rs
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_src = Path(tmp_dir)
            cam_rs = tmp_src / "crates" / "ui-egui" / "src" / "graph" / "graph_camera.rs"
            gesture_rs = tmp_src / "crates" / "ui-egui" / "src" / "graph" / "gesture.rs"
            cam_rs.parent.mkdir(parents=True, exist_ok=True)
            cam_rs.write_text(
                "    pub(crate) fn zoom_from_input(&mut self, ui: &Ui, view: Rect) {\n"
                "        self.zoom = self.zoom.clamp(crate::state::GraphCamera::ZOOM.0, crate::state::GraphCamera::ZOOM.1);\n"
                "        if !ui.rect_contains_pointer(view) {\n"
                "            return;\n"
                "        }\n"
                "\n"
                "        let Some(pointer) = ui.input(|input| input.pointer.hover_pos()).filter(|pointer| view.contains(*pointer)) else { return };\n"
                "\n"
                "        let scroll = ui.input(|input| input.smooth_scroll_delta.y + input.zoom_delta().ln() * 200.0);\n"
                "        if scroll.is_finite() && scroll != 0.0 {\n"
                "            let before = self.to_graph(pointer);\n"
                "            self.zoom = (self.zoom * (scroll * 0.0025).exp()).clamp(crate::state::GraphCamera::ZOOM.0, crate::state::GraphCamera::ZOOM.1);\n"
                "            self.pan = before.to_vec2() - (pointer - self.origin) / self.zoom;\n"
                "        }\n"
                "    }\n"
            )
            gesture_rs.write_text(
                "pub(crate) fn cancelled(ui: &Ui) -> bool {\n"
                "    ui.input(|input| !input.focused || input.key_pressed(egui::Key::Escape))\n"
                "}\n"
            )

            cmd = ["bash", str(SCROLLING_SCRIPT), str(tmp_src), "materialcraft", "MaterialCraft"]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, f"STDOUT: {result.stdout}\nSTDERR: {result.stderr}")
            self.assertIn("two-finger pan and touch zoom pivot, v1", result.stdout)
            self.assertIn("cancel drag on multi-touch/PointerGone", result.stdout)
            self.assertIn("self.pan -= pan_delta / self.zoom;", cam_rs.read_text())
            self.assertIn("egui::Event::PointerGone", gesture_rs.read_text())

            result2 = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result2.returncode, 0)
            self.assertEqual(result2.stdout.strip(), "")

    def test_wordcraft_font_dirs_include_craft_font_store(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_src = Path(tmp_dir)
            names_rs = tmp_src / "crates" / "fonts" / "src" / "fontdb" / "names.rs"
            names_rs.parent.mkdir(parents=True, exist_ok=True)
            names_rs.write_text(
                '        dirs.extend(["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts", "/run/host/user-fonts-legacy"].map(Into::into));\n'
            )

            cmd = [sys.executable, str(PATCHES_SCRIPT), str(tmp_src), "wordcraft", "WordCraft"]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0)
            self.assertIn("system and user font directories", result.stdout)
            patched = names_rs.read_text()
            self.assertIn("/system/fonts", patched)
            self.assertIn('std::env::var_os("CRAFT_FONT_STORE")', patched)

            result2 = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result2.returncode, 0)
            self.assertEqual(result2.stdout.strip(), "")


if __name__ == "__main__":
    unittest.main()
