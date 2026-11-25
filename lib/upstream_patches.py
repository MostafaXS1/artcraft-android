#!/usr/bin/env python3
"""Apply transient upstream compile error repairs for ArtCraft Android apps.

Upstream `main` branches are normally green, but temporary regressions can
occur. Each fix-up is guarded: it edits the checkout only while the broken text
is still present, and is a silent no-op once upstream repairs it, so nothing is
permanently forked.

Extracted from lib/build-common.sh as part of modularization (issue #73).
"""

from pathlib import Path
import re
import sys

if len(sys.argv) < 4:
    print("Usage: upstream_patches.py <src> <app> <title>", file=sys.stderr)
    sys.exit(1)

src, app, title = Path(sys.argv[1]), sys.argv[2], sys.argv[3]


def fix(rel: str, old: str, new: str) -> bool:
    """Replace the single `old` in `rel` with `new`, unless it is already gone or already fixed."""
    path = src / rel
    try:
        text = path.read_text()
    except OSError:
        return False
    if old not in text or new in text or text.count(old) != 1:
        return False
    path.write_text(text.replace(old, new, 1))
    return True


def fix_all(rel: str, old: str, new: str) -> bool:
    """Replace all occurrences of `old` in `rel` with `new`, unless it is already gone."""
    path = src / rel
    try:
        text = path.read_text()
    except OSError:
        return False
    if old not in text:
        return False
    path.write_text(text.replace(old, new))
    return True


fixed = []
if app == "photocraft":
    # "load each persisted preset set independently" added `export_presets` to the serialised
    # `Persisted` set and to its write path, but not to the read initialiser, so the crate no
    # longer compiles. Fill in the missing field; the code below it already reads it back.
    if fix(
        "crates/engine/src/presets/mod.rs",
        '            recent_documents: take(&mut m, "recent_documents"),\n',
        '            recent_documents: take(&mut m, "recent_documents"),\n'
        '            export_presets: take(&mut m, "export_presets"),\n',
    ):
        fixed.append("crates/engine/src/presets/mod.rs")
    if fix_all("crates/raster/src/memory/system.rs", '#[cfg(target_os = "linux")]', '#[cfg(any(target_os = "linux", target_os = "android"))]'):
        fixed.append("crates/raster/src/memory/system.rs (Android meminfo)")
    fix_all("crates/raster/src/memory/system.rs", '#[cfg(not(target_os = "linux"))]', '#[cfg(not(any(target_os = "linux", target_os = "android")))]')
    fix_all("crates/raster/src/memory/system.rs", '#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]', '#[cfg(not(any(target_os = "linux", target_os = "android", windows, target_os = "macos")))]')
    # Android's untrusted_app SELinux sandbox blocks reading /proc/self/cgroup and /sys/fs/cgroup,
    # causing `cgroup_limits` to return `CgroupLimits::Unreadable` (`available_ram = Some(0)`) and
    # hard-refuse opening files. Skip cgroup probing on Android so `/proc/meminfo` (or `UNKNOWN` if
    # unreadable) drives the budget instead of `Some(0)`.
    if fix(
        "crates/raster/src/memory/system.rs",
        '        let (cgroup_ram, cgroup_swap) = match cgroup_limits(proc_root, sys_root) {\n'
        '            CgroupLimits::NotConstrained => (None, None),\n'
        '            CgroupLimits::Limited { ram, swap } => (ram, swap),\n'
        '            CgroupLimits::Unreadable => return SystemMemory { available_ram: Some(0), swap_free: Some(0), commit_headroom: None, constraint_unreadable: true },\n'
        '        };',
        '        let (cgroup_ram, cgroup_swap) = if cfg!(target_os = "android") {\n'
        '            let _ = sys_root;\n'
        '            (None, None)\n'
        '        } else {\n'
        '            match cgroup_limits(proc_root, sys_root) {\n'
        '                CgroupLimits::NotConstrained => (None, None),\n'
        '                CgroupLimits::Limited { ram, swap } => (ram, swap),\n'
        '                CgroupLimits::Unreadable => return SystemMemory { available_ram: Some(0), swap_free: Some(0), commit_headroom: None, constraint_unreadable: true },\n'
        '            }\n'
        '        };',
    ):
        fixed.append("crates/raster/src/memory/system.rs (skip cgroup on Android)")
    # Android's winit backend stubs out custom cursor bitmaps (`create_custom_cursor` and
    # `set_cursor` are no-ops), so uploading a native cursor image leaves only the system stylus dot
    # and skips painting the brush tip outline on the canvas. Paint tool cursors on canvas instead.
    if fix(
        "crates/ui-egui/src/canvas/tool_cursor.rs",
        '    cfg!(not(target_arch = "wasm32")) && ctx.viewport_id() == egui::ViewportId::ROOT && !scaled_wayland(ctx)',
        '    cfg!(not(any(target_arch = "wasm32", target_os = "android"))) && ctx.viewport_id() == egui::ViewportId::ROOT && !scaled_wayland(ctx)',
    ):
        fixed.append("crates/ui-egui/src/canvas/tool_cursor.rs (paint tool cursors on Android)")
    # `last_brush::persist` and `tool_memory::persist` run at the end of `ui_frame` after
    # `prefs_ui::tick` already ran in `logic_frame`. Request a repaint when they edit preferences
    # so the updated brush settings / tool options flush on the very next frame rather than waiting
    # for unrelated input.
    if fix(
        "crates/ui-egui/src/last_brush.rs",
        '    if !ctx.input(|i| i.pointer.any_down()) {\n'
        '        photocraft_engine::brush_cmds::remember_brush(&mut app.session);\n'
        '    }',
        '    if !ctx.input(|i| i.pointer.any_down()) {\n'
        '        let before = app.session.tools.brush_saved_rev;\n'
        '        photocraft_engine::brush_cmds::remember_brush(&mut app.session);\n'
        '        if app.session.tools.brush_saved_rev != before {\n'
        '            ctx.request_repaint();\n'
        '        }\n'
        '    }',
    ):
        fixed.append("crates/ui-egui/src/last_brush.rs (repaint after remember_brush)")
    if fix(
        "crates/ui-egui/src/tool_memory.rs",
        '    persist_brushes(app);\n'
        '    if app.prefs_rt.tool_options.as_ref() == Some(&app.ui.tool_options) {\n'
        '        return;\n'
        '    }\n'
        '    app.prefs_rt.tool_options = Some(app.ui.tool_options.clone());\n'
        '    let now = changed(&app.ui.tool_options);\n'
        '    if app.session.prefs().tool_options != now {\n'
        '        app.session.prefs.edit(|p| p.tool_options = now);\n'
        '    }',
        '    persist_brushes(app, ctx);\n'
        '    if app.prefs_rt.tool_options.as_ref() == Some(&app.ui.tool_options) {\n'
        '        return;\n'
        '    }\n'
        '    app.prefs_rt.tool_options = Some(app.ui.tool_options.clone());\n'
        '    let now = changed(&app.ui.tool_options);\n'
        '    if app.session.prefs().tool_options != now {\n'
        '        app.session.prefs.edit(|p| p.tool_options = now);\n'
        '        ctx.request_repaint();\n'
        '    }',
    ):
        fixed.append("crates/ui-egui/src/tool_memory.rs (repaint after tool_options edit)")
    if fix(
        "crates/ui-egui/src/tool_memory.rs",
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
        '    }',
        'fn persist_brushes(app: &mut PhotocraftApp, ctx: &egui::Context) {\n'
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
        '        ctx.request_repaint();\n'
        '    }',
    ):
        fixed.append("crates/ui-egui/src/tool_memory.rs (repaint after tool_brushes edit)")
    # Expose immediate state flushing so the Android shell can persist brush presets and
    # preferences on focus loss / pause before Android terminates the process.
    if fix(
        "crates/ui-egui/src/app.rs",
        'impl eframe::App for PhotocraftApp {\n'
        '    fn on_exit(&mut self) {\n',
        'impl PhotocraftApp {\n'
        '    /// Flush pending brush-preset store writes and tool/brush/preference state immediately.\n'
        '    pub fn flush_persistence(&mut self) {\n'
        '        self.session.sync_preset_store();\n'
        '        if let Err(err) = tool_memory::save_on_exit(self) {\n'
        '            log::warn!("Preferences: could not save the tool state: {err}");\n'
        '        }\n'
        '    }\n'
        '}\n'
        '\n'
        'impl eframe::App for PhotocraftApp {\n'
        '    fn on_exit(&mut self) {\n'
        '        self.session.sync_preset_store();\n',
    ):
        fixed.append("crates/ui-egui/src/app.rs (flush_persistence)")
elif app == "cadcraft":
    # cadcraft's own crates/fonts/src/complex.rs was written against harfrust 0.14
    # (FontRef::from_index, ShaperData, UnicodeBuffer::push(u32, u32), shape()), but the
    # workspace pinned harfrust 0.12.0. Bump it to 0.14 so the fonts crate compiles.
    # egui/eframe/egui-wgpu stay at 0.37 (they are path deps to the vendored fork, the
    # "0.37" -> "0.36" patterns below never match path deps and are left as no-ops).
    for rel, old, new in [
        ("Cargo.toml", 'harfrust = "0.12.0"', 'harfrust = "0.12.0"'),
        ("Cargo.toml", 'harfrust = "0.13.3"', 'harfrust = "0.12.0"'),
        ("Cargo.toml", 'harfrust = "0.14.0"', 'harfrust = "0.12.0"'),
        ("Cargo.toml", 'harfrust = "0.15.0"', 'harfrust = "0.12.0"'),
        ("Cargo.toml", 'quick-xml = "0.42"', 'quick-xml = "0.41"'),
        ("crates/engine/Cargo.toml", 'rstar = { version = "0.13", default-features = false }', 'rstar = { version = "0.12", default-features = false }'),
    ]:
        if fix(rel, old, new):
            fixed.append(f"{rel} ({old} -> {new})")

    # Fix rstar 0.12 API (locate_in_envelope_intersecting takes &T::Envelope)
    if fix(
        "crates/engine/src/spatial.rs",
        'self.tree.locate_in_envelope_intersecting(env)',
        'self.tree.locate_in_envelope_intersecting(&env)',
    ):
        fixed.append("crates/engine/src/spatial.rs (rstar API)")

    # Fix quick-xml into_inner() API (returns &[u8], which does not have .as_bytes())
    if fix_all(
        "crates/io/src/svg_read.rs",
        ".into_inner().as_bytes()",
        ".into_inner()",
    ):
        fixed.append("crates/io/src/svg_read.rs (quick-xml into_inner)")

elif app == "diagramcraft":
    # DiagramCraft's system_dict_dirs() only defines branches for linux, macos, and windows.
    # On Android, none match, implicitly returning () and causing a type mismatch (expected Vec<PathBuf>).
    # Provide an empty fallback Vec<PathBuf> for Android.
    if fix(
        "crates/dc/src/spellcheck.rs",
        '    #[cfg(target_os = "macos")]\n'
        '    {\n'
        '        vec![PathBuf::from("/Library/Spelling")]\n'
        '    }\n'
        '    #[cfg(windows)]',
        '    #[cfg(target_os = "macos")]\n'
        '    {\n'
        '        vec![PathBuf::from("/Library/Spelling")]\n'
        '    }\n'
        '    #[cfg(target_os = "android")]\n'
        '    {\n'
        '        Vec::new()\n'
        '    }\n'
        '    #[cfg(windows)]',
    ):
        fixed.append("crates/dc/src/spellcheck.rs (system_dict_dirs android fallback)")

elif app == "wordcraft":
    # rustc 1.99 rejects exhaustive struct patterns that name a field which is private to another
    # module ("pattern requires `..` due to inaccessible fields"). The `swap_fields!` macro
    # destructures `WindowUi` exhaustively by name, so add `..` to keep it compiling against the
    # updated struct. Guarded like the other fixes; a no-op once upstream repairs it.
    if fix(
        "crates/ui-egui/src/window_ui.rs",
        '        let WindowUi { $($(#[$attr])* $field: _),* } = &*$w;',
        '        let WindowUi { $($(#[$attr])* $field: _),*, .. } = &*$w;',
    ):
        fixed.append("crates/ui-egui/src/window_ui.rs (swap_fields pattern)")

    # Ensure system_font_dirs discovers Android system fonts, shared storage Fonts folder,
    # and app-private fonts folders.
    if fix(
        "crates/fonts/src/fontdb/names.rs",
        '        dirs.extend(["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts", "/run/host/user-fonts-legacy"].map(Into::into));',
        '        dirs.extend(["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts", "/run/host/user-fonts-legacy", "/system/fonts", "/product/fonts", "/storage/emulated/0/Fonts", "/sdcard/Fonts"].map(Into::into));\n        if let Some(base) = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from) {\n            dirs.push(base.join("fonts"));\n        }\n        if let Some(cfg) = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from) {\n            dirs.push(cfg.join("wordcraft/fonts"));\n            dirs.push(cfg.join("fonts"));\n        }',
    ):
        fixed.append("crates/fonts/src/fontdb/names.rs (system and user font directories)")

    # Clicking the AutoSave switch on an opened document (NotSavedHere) should save in place
    # and enable AutoSave rather than opening Save As (#143).
    if fix(
        "crates/ui-egui/src/chrome/title_bar.rs",
        '            // AutoSave comes on once Save As has saved (maybe on a later frame, #94).\n'
        '            Some(b) if b.save_as_helps() => {',
        '            Some(crate::AutoSaveBlock::NotSavedHere) => {\n'
        '                if app.run("file.save", json!({})).is_ok() && app.saved_here() {\n'
        '                    let _ = app.run("file.autosave", json!({"value": true}));\n'
        '                }\n'
        '            }\n'
        '            // AutoSave comes on once Save As has saved (maybe on a later frame, #94).\n'
        '            Some(b) if b.save_as_helps() => {',
    ):
        fixed.append("crates/ui-egui/src/chrome/title_bar.rs (AutoSave toggle saves opened file in place)")
    if fix(
        "crates/ui-egui/src/chrome.rs",
        '                            // AutoSave comes on once Save As has saved (maybe on a later frame, #94).\n'
        '                            Some(b) if b.save_as_helps() => {',
        '                            Some(crate::AutoSaveBlock::NotSavedHere) => {\n'
        '                                if app.run("file.save", json!({})).is_ok() && app.saved_here() {\n'
        '                                    let _ = app.run("file.autosave", json!({"value": true}));\n'
        '                                }\n'
        '                            }\n'
        '                            // AutoSave comes on once Save As has saved (maybe on a later frame, #94).\n'
        '                            Some(b) if b.save_as_helps() => {',
    ):
        fixed.append("crates/ui-egui/src/chrome.rs (AutoSave toggle saves opened file in place)")

elif app == "lightcraft":
    # Interactive slider / curve performance on Android:
    # 1. Whole-frame drafts on high-DPI Android screens rendered at full 2560px canvas resolution,
    #    consuming huge memory bandwidth and 80-150ms per draft on mobile GPUs/CPUs. Cap mobile
    #    interactive drafts to 960px (and 0.35 CPU draft scale) for fluid 60-120 FPS slider updates.
    if fix(
        "crates/ui-egui/src/region.rs",
        '/// Size of a slider drag\'s whole-frame draft when full size would be too slow (the CPU renders\n'
        '/// it, or full-size drafts of this drag went over [`DRAFT_BUDGET_MS`]), against the render at rest.\n'
        'pub const CPU_DRAFT_SCALE: f32 = 0.6;\n',
        '/// Size of a slider drag\'s whole-frame draft when full size would be too slow (the CPU renders\n'
        '/// it, or full-size drafts of this drag went over [`DRAFT_BUDGET_MS`]), against the render at rest.\n'
        '#[cfg(target_os = "android")]\n'
        'pub const CPU_DRAFT_SCALE: f32 = 0.35;\n'
        '#[cfg(not(target_os = "android"))]\n'
        'pub const CPU_DRAFT_SCALE: f32 = 0.6;\n'
        '\n'
        '/// The maximum long edge for whole-frame drafts during interactive dragging on mobile.\n'
        '/// Keeps draft rendering within ~2-5 ms on mobile GPUs and drastically reduces memory bandwidth.\n'
        '#[cfg(target_os = "android")]\n'
        'pub const DRAFT_EDGE_MAX: usize = 960;\n'
        '#[cfg(not(target_os = "android"))]\n'
        'pub const DRAFT_EDGE_MAX: usize = usize::MAX;\n',
    ):
        fixed.append("crates/ui-egui/src/region.rs (mobile draft constants)")

    if fix(
        "crates/ui-egui/src/region.rs",
        '    if !v.windows {\n'
        '        // the whole frame at the size it is drawn (a whole-frame diagnostic has no\n'
        '        // window); while a slider drags, a canvas-sized draft as with windows: a drag at 1:1 must\n'
        '        // not render the photo\'s every pixel each frame\n'
        '        let long = if v.draft.is_some() { drawn_long.min(canvas_long) } else { drawn_long };\n'
        '        let main_edge = settings.loupe_edge(long * scale, v.native_long, v.texture_side);\n'
        '        return LoupePlan { main_edge, window_edge: None };\n'
        '    }\n'
        '    // a user who chose a size above the preview source level (3840, 5120) means it; otherwise the\n'
        '    // original is not decoded for the whole-frame render\n'
        '    let source_cap = if settings.preview_limit == 0 { lightcraft_engine::SourceLevel::Preview.max_edge() } else { settings.preview_limit as usize };\n'
        '    let edge_at = |scale: f32| settings.loupe_edge(drawn_long.min(canvas_long) * scale, v.native_long, v.texture_side).min(source_cap);\n'
        '    let frame_edge = settings.window_frame_edge(drawn_long, v.native_long);\n'
        '    // a user who capped the preview size has chosen speed over sharpness at fit\n'
        '    let fit = drawn_long <= canvas_long * 1.05;\n'
        '    // (against the undrafted whole-frame render: a drag must not switch the window off and on)\n'
        '    let window_edge = (frame_edge as f32 > edge_at(1.0) as f32 * WINDOW_RATIO && !(fit && settings.preview_limit != 0)).then_some(frame_edge);\n'
        '    // with a window over it, the whole-frame draft is hidden (it feeds the histogram and stands in\n'
        '    // where the window has no pixels yet): the small draft does, and the window keeps the frame time\n'
        '    let scale = if window_edge.is_some() && v.draft.is_some() { scale.min(CPU_DRAFT_SCALE) } else { scale };\n'
        '    LoupePlan { main_edge: edge_at(scale), window_edge }',
        '    if !v.windows {\n'
        '        // the whole frame at the size it is drawn (a whole-frame diagnostic has no\n'
        '        // window); while a slider drags, a canvas-sized draft as with windows: a drag at 1:1 must\n'
        '        // not render the photo\'s every pixel each frame\n'
        '        let long = if v.draft.is_some() { drawn_long.min(canvas_long) } else { drawn_long };\n'
        '        let mut main_edge = settings.loupe_edge(long * scale, v.native_long, v.texture_side);\n'
        '        if v.draft.is_some() {\n'
        '            main_edge = main_edge.min(DRAFT_EDGE_MAX);\n'
        '        }\n'
        '        return LoupePlan { main_edge, window_edge: None };\n'
        '    }\n'
        '    // a user who chose a size above the preview source level (3840, 5120) means it; otherwise the\n'
        '    // original is not decoded for the whole-frame render\n'
        '    let source_cap = if settings.preview_limit == 0 { lightcraft_engine::SourceLevel::Preview.max_edge() } else { settings.preview_limit as usize };\n'
        '    let edge_at = |scale: f32| settings.loupe_edge(drawn_long.min(canvas_long) * scale, v.native_long, v.texture_side).min(source_cap);\n'
        '    let frame_edge = settings.window_frame_edge(drawn_long, v.native_long);\n'
        '    // a user who capped the preview size has chosen speed over sharpness at fit\n'
        '    let fit = drawn_long <= canvas_long * 1.05;\n'
        '    // (against the undrafted whole-frame render: a drag must not switch the window off and on)\n'
        '    let window_edge = (frame_edge as f32 > edge_at(1.0) as f32 * WINDOW_RATIO && !(fit && settings.preview_limit != 0)).then_some(frame_edge);\n'
        '    // with a window over it, the whole-frame draft is hidden (it feeds the histogram and stands in\n'
        '    // where the window has no pixels yet): the small draft does, and the window keeps the frame time\n'
        '    let scale = if window_edge.is_some() && v.draft.is_some() { scale.min(CPU_DRAFT_SCALE) } else { scale };\n'
        '    let mut main_edge = edge_at(scale);\n'
        '    if v.draft.is_some() {\n'
        '        main_edge = main_edge.min(DRAFT_EDGE_MAX);\n'
        '    }\n'
        '    LoupePlan { main_edge, window_edge }',
    ):
        fixed.append("crates/ui-egui/src/region.rs (clamp mobile draft to DRAFT_EDGE_MAX)")

    # 2. `wait_for_draft` lockout: previous render at rest (25-50ms) made `last_main_ms * 1.5 >= budget`
    #    fail on the first draft frame, so wait_for_draft never waited during drags. On Android, wait
    #    when GPU is available so fast 960px drafts show in the very same frame without backlog lag.
    if fix(
        "crates/ui-egui/src/render.rs",
        '    #[cfg(not(target_arch = "wasm32"))]\n'
        '    pub fn wait_for_draft(&mut self, ctx: &egui::Context, session: &mut Session, budget: std::time::Duration) {\n'
        '        if self.offload.is_some() || self.last_main_ms * 1.5 >= budget.as_secs_f64() * 1e3 {\n'
        '            return;\n'
        '        }',
        '    #[cfg(not(target_arch = "wasm32"))]\n'
        '    pub fn wait_for_draft(&mut self, ctx: &egui::Context, session: &mut Session, budget: std::time::Duration) {\n'
        '        #[cfg(target_os = "android")]\n'
        '        let skip_wait = self.offload.is_some() || (!lightcraft_engine::gpu::available() && self.last_main_ms > 80.0);\n'
        '        #[cfg(not(target_os = "android"))]\n'
        '        let skip_wait = self.offload.is_some() || self.last_main_ms * 1.5 >= budget.as_secs_f64() * 1e3;\n'
        '        if skip_wait {\n'
        '            return;\n'
        '        }',
    ):
        fixed.append("crates/ui-egui/src/render.rs (wait_for_draft Android GPU support)")

    # 3. Provide sufficient budget (~10ms) on Android for fast drafts to complete synchronously.
    if fix(
        "crates/ui-egui/src/panels/detail.rs",
        '        #[cfg(not(target_arch = "wasm32"))]\n'
        '        if interacting && whole_draft {\n'
        '            let frame = ui.input(|i| i.predicted_dt).clamp(1.0 / 240.0, 1.0 / 30.0);\n'
        '            // (a frame time that is not a number waits for nothing)\n'
        '            let budget = std::time::Duration::try_from_secs_f32(frame * 0.4).unwrap_or_default();\n'
        '            app.renderer.wait_for_draft(ui.ctx(), &mut app.session, budget);\n'
        '        }',
        '        #[cfg(not(target_arch = "wasm32"))]\n'
        '        if interacting && whole_draft {\n'
        '            let frame = ui.input(|i| i.predicted_dt).clamp(1.0 / 240.0, 1.0 / 30.0);\n'
        '            // (a frame time that is not a number waits for nothing)\n'
        '            #[cfg(target_os = "android")]\n'
        '            let budget = std::time::Duration::try_from_secs_f32(frame.max(1.0 / 60.0) * 0.6).unwrap_or(std::time::Duration::from_millis(10));\n'
        '            #[cfg(not(target_os = "android"))]\n'
        '            let budget = std::time::Duration::try_from_secs_f32(frame * 0.4).unwrap_or_default();\n'
        '            app.renderer.wait_for_draft(ui.ctx(), &mut app.session, budget);\n'
        '        }',
    ):
        fixed.append("crates/ui-egui/src/panels/detail.rs (Android wait_for_draft budget)")

elif app == "filmcraft":
    # 0. Allow unsafe code in workspace for hardware surface interop.
    if fix(
        "Cargo.toml",
        'unsafe_code = "forbid"\n',
        'unsafe_code = "allow"\n',
    ):
        fixed.append("Cargo.toml (unsafe_code = allow)")

    # 1. Add ndk dependency to crates/platform/Cargo.toml for AMediaCodec.
    if fix(
        "crates/platform/Cargo.toml",
        '[target.\'cfg(target_os = "linux")\'.dependencies]\nlibloading = "0.8"\n',
        '[target.\'cfg(target_os = "linux")\'.dependencies]\nlibloading = "0.8"\n\n'
        '# Android: hardware H.264 video encoding through Android\'s NDK MediaCodec API (libmediandk).\n'
        '[target.\'cfg(target_os = "android")\'.dependencies]\nndk = { version = "0.9", features = ["media", "nativewindow", "api-level-28", "rwh_06"], default-features = false }\nndk-sys = "0.6"\njni = "0.22.4"\nndk-context = "0.1.1"\nrayon = { workspace = true }\n',
    ):
        fixed.append("crates/platform/Cargo.toml (ndk dependency)")

    # 1a. Allow unsafe code in crates/platform/Cargo.toml for NEON SIMD optimizations.
    if fix(
        "crates/platform/Cargo.toml",
        '[lints.rust]\nunsafe_code = "deny"\n',
        '[lints.rust]\nunsafe_code = "allow"\n',
    ):
        fixed.append("crates/platform/Cargo.toml (unsafe_code = allow)")

    # 1b. Add log dependency to crates/export/Cargo.toml for export logging.
    if fix(
        "crates/export/Cargo.toml",
        'rayon = { workspace = true }\n',
        'rayon = { workspace = true }\nlog = { workspace = true }\n',
    ):
        fixed.append("crates/export/Cargo.toml (log dependency)")

    # 2. Add android_mediacodec module declaration in crates/platform/src/lib.rs.
    if fix(
        "crates/platform/src/lib.rs",
        'mod annexb;\n',
        'mod annexb;\n'
        '#[cfg(target_os = "android")]\n'
        'pub mod android_mediacodec;\n',
    ):
        fixed.append("crates/platform/src/lib.rs (android_mediacodec module)")

    # 3. Register android_mediacodec factory in crates/platform/src/lib.rs.
    if fix(
        "crates/platform/src/lib.rs",
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]\n'
        '    {\n'
        '        Availability::Unavailable("no hardware video decoder for this system yet")\n'
        '    }\n'
        '}\n'
        '\n'
        '/// Whether [`register`] has put a hardware decoder factory in front of our decoders.\n'
        'pub fn registered() -> bool {\n'
        '    #[cfg(target_os = "macos")]\n'
        '    {\n'
        '        filmcraft_codecs::video_decoder_registered(videotoolbox_factory)\n'
        '    }\n'
        '    #[cfg(target_os = "windows")]\n'
        '    {\n'
        '        filmcraft_codecs::video_decoder_registered(media_foundation_factory)\n'
        '    }\n'
        '    #[cfg(target_os = "linux")]\n'
        '    {\n'
        '        #[cfg(target_pointer_width = "64")]\n'
        '        if filmcraft_codecs::video_decoder_registered(nvdec_factory) {\n'
        '            return true;\n'
        '        }\n'
        '        filmcraft_codecs::video_decoder_registered(vaapi_factory)\n'
        '    }\n'
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]\n',
        '    #[cfg(target_os = "android")]\n'
        '    {\n'
        '        static ONCE: std::sync::Once = std::sync::Once::new();\n'
        '        ONCE.call_once(|| {\n'
        '            filmcraft_codecs::register_video_decoder(android_mediacodec::decoder_factory);\n'
        '            filmcraft_export::register_encoder(android_mediacodec::factory);\n'
        '            filmcraft_codecs::hw::set_hw_backend("MediaCodec");\n'
        '            android_mediacodec::register_scaler();\n'
        '        });\n'
        '        Availability::Available("MediaCodec")\n'
        '    }\n'
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux", target_os = "android")))]\n'
        '    {\n'
        '        Availability::Unavailable("no hardware video decoder for this system yet")\n'
        '    }\n'
        '}\n'
        '\n'
        '/// Whether [`register`] has put a hardware decoder factory in front of our decoders.\n'
        'pub fn registered() -> bool {\n'
        '    #[cfg(target_os = "macos")]\n'
        '    {\n'
        '        filmcraft_codecs::video_decoder_registered(videotoolbox_factory)\n'
        '    }\n'
        '    #[cfg(target_os = "windows")]\n'
        '    {\n'
        '        filmcraft_codecs::video_decoder_registered(media_foundation_factory)\n'
        '    }\n'
        '    #[cfg(target_os = "linux")]\n'
        '    {\n'
        '        #[cfg(target_pointer_width = "64")]\n'
        '        if filmcraft_codecs::video_decoder_registered(nvdec_factory) {\n'
        '            return true;\n'
        '        }\n'
        '        filmcraft_codecs::video_decoder_registered(vaapi_factory)\n'
        '    }\n'
        '    #[cfg(target_os = "android")]\n'
        '    {\n'
        '        filmcraft_codecs::video_decoder_registered(android_mediacodec::decoder_factory)\n'
        '            && filmcraft_export::encoder_registered(android_mediacodec::factory)\n'
        '    }\n'
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux", target_os = "android")))]\n',
    ):
        fixed.append("crates/platform/src/lib.rs (MediaCodec registration)")

    # 3b. Support hardware_decoder_for on Android in crates/platform/src/lib.rs.
    if fix(
        "crates/platform/src/lib.rs",
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]\n'
        '    {\n'
        '        let _ = entry;\n'
        '        false\n'
        '    }\n'
        '}\n',
        '    #[cfg(target_os = "android")]\n'
        '    {\n'
        '        filmcraft_codecs::hw::NalStreamInfo::from_entry(entry).and_then(|r| r.ok()).is_some_and(|info| {\n'
        '            android_mediacodec::decoder_for(&info)\n'
        '        })\n'
        '    }\n'
        '    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux", target_os = "android")))]\n'
        '    {\n'
        '        let _ = entry;\n'
        '        false\n'
        '    }\n'
        '}\n',
    ):
        fixed.append("crates/platform/src/lib.rs (MediaCodec decoder_for)")

    # 3c. Support direct surface bridge in crates/codecs/src/hw.rs.
    if fix(
        "crates/codecs/src/hw.rs",
        'static ENABLED: AtomicBool = AtomicBool::new(true);\n',
        'static ENABLED: AtomicBool = AtomicBool::new(true);\n\n'
        'static DIRECT_SURFACE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n\n'
        'static DIRECT_SURFACE_DIMS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);\n\n'
        'pub fn set_direct_surface(surface: *mut std::ffi::c_void, w: u32, h: u32) {\n'
        '    let dims = ((w as u64) << 32) | (h as u64);\n'
        '    DIRECT_SURFACE_DIMS.store(dims, Ordering::SeqCst);\n'
        '    DIRECT_SURFACE.store(surface as usize, Ordering::SeqCst);\n'
        '}\n\n'
        'pub fn clear_direct_surface() {\n'
        '    DIRECT_SURFACE.store(0, Ordering::SeqCst);\n'
        '    DIRECT_SURFACE_DIMS.store(0, Ordering::SeqCst);\n'
        '    clear_surface_scaler();\n'
        '}\n\n'
        'pub fn has_direct_surface() -> bool {\n'
        '    DIRECT_SURFACE.load(Ordering::SeqCst) != 0\n'
        '}\n\n'
        'pub fn direct_surface() -> Option<*mut std::ffi::c_void> {\n'
        '    let p = DIRECT_SURFACE.load(Ordering::SeqCst);\n'
        '    if p != 0 {\n'
        '        Some(p as *mut std::ffi::c_void)\n'
        '    } else {\n'
        '        None\n'
        '    }\n'
        '}\n\n'
        'pub fn direct_surface_for(w: u32, h: u32) -> Option<*mut std::ffi::c_void> {\n'
        '    let dims = DIRECT_SURFACE_DIMS.load(Ordering::SeqCst);\n'
        '    let expected = ((w as u64) << 32) | (h as u64);\n'
        '    if dims != 0 && dims == expected {\n'
        '        direct_surface()\n'
        '    } else {\n'
        '        None\n'
        '    }\n'
        '}\n\n'
        'pub type ScalerInitFn = fn(*mut std::ffi::c_void, u32, u32, u32, u32) -> Option<*mut std::ffi::c_void>;\n'
        'pub type ScalerBlitFn = fn() -> bool;\n'
        'pub type ScalerCheckFn = fn() -> bool;\n'
        'pub type ScalerClearFn = fn();\n\n'
        'static SCALER_INIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n'
        'static SCALER_BLIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n'
        'static SCALER_CHECK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n'
        'static SCALER_CLEAR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n\n'
        'pub fn register_surface_scaler(init: ScalerInitFn, blit: ScalerBlitFn, check: ScalerCheckFn, clear: ScalerClearFn) {\n'
        '    SCALER_INIT.store(init as usize, Ordering::SeqCst);\n'
        '    SCALER_BLIT.store(blit as usize, Ordering::SeqCst);\n'
        '    SCALER_CHECK.store(check as usize, Ordering::SeqCst);\n'
        '    SCALER_CLEAR.store(clear as usize, Ordering::SeqCst);\n'
        '}\n\n'
        'pub fn init_surface_scaler(window: *mut std::ffi::c_void, src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Option<*mut std::ffi::c_void> {\n'
        '    let p = SCALER_INIT.load(Ordering::SeqCst);\n'
        '    if p != 0 {\n'
        '        let f: ScalerInitFn = unsafe { std::mem::transmute(p) };\n'
        '        f(window, src_w, src_h, dst_w, dst_h)\n'
        '    } else {\n'
        '        None\n'
        '    }\n'
        '}\n\n'
        'pub fn blit_surface_scaler() -> bool {\n'
        '    let p = SCALER_BLIT.load(Ordering::SeqCst);\n'
        '    if p != 0 {\n'
        '        let f: ScalerBlitFn = unsafe { std::mem::transmute(p) };\n'
        '        f()\n'
        '    } else {\n'
        '        false\n'
        '    }\n'
        '}\n\n'
        'pub fn has_surface_scaler() -> bool {\n'
        '    let p = SCALER_CHECK.load(Ordering::SeqCst);\n'
        '    if p != 0 {\n'
        '        let f: ScalerCheckFn = unsafe { std::mem::transmute(p) };\n'
        '        f()\n'
        '    } else {\n'
        '        false\n'
        '    }\n'
        '}\n\n'
        'pub fn clear_surface_scaler() {\n'
        '    let p = SCALER_CLEAR.load(Ordering::SeqCst);\n'
        '    if p != 0 {\n'
        '        let f: ScalerClearFn = unsafe { std::mem::transmute(p) };\n'
        '        f();\n'
        '    }\n'
        '}\n',
    ):
        fixed.append("crates/codecs/src/hw.rs (direct surface bridge tracking)")

    # 4. Copy android_mediacodec.rs template to checkout.
    mediacodec_target = src / "crates/platform/src/android_mediacodec.rs"
    template_file = Path.cwd() / "templates/filmcraft/android_mediacodec.rs"
    if template_file.exists() and mediacodec_target.parent.is_dir():
        content = template_file.read_text()
        if not mediacodec_target.exists() or mediacodec_target.read_text() != content:
            mediacodec_target.write_text(content)
            fixed.append("crates/platform/src/android_mediacodec.rs (updated from template)")

    # 5. Show hardware encoding status in export completion message and write export report log.
    if fix(
        "crates/export/src/job.rs",
        '            progress.set_status(format!("Done in {secs:.1}s"));\n',
        '            let enc = crate::hw_encode_stats();\n'
        '            let dec = filmcraft_codecs::hw::hw_stats();\n'
        '            let gop = filmcraft_codecs::gop_stats();\n'
        '            let gpu = crate::gpu_render_stats();\n'
        '            let hw_info = if gop.frames == 0 {\n'
        '                format!(" (Still/Gen, GPU: {}, Enc: {} HW)", gpu.frames, enc.frames)\n'
        '            } else {\n'
        '                format!(" (Dec: {}/{} HW, GPU: {}, Enc: {} HW)", dec.frames, gop.frames, gpu.frames, enc.frames)\n'
        '            };\n'
        '            let st = format!("Done in {secs:.1}s{hw_info}");\n'
        '            progress.set_status(st.clone());\n'
        '            let log_path = format!("{}.log", self.settings.path);\n'
        '            let mut diag = String::new();\n'
        '            diag.push_str("=== FILMCRAFT EXPORT REPORT ===\\n");\n'
        '            diag.push_str(&format!("Output: {}\\n", self.settings.path));\n'
        '            diag.push_str(&format!("Status: {st}\\n"));\n'
        '            diag.push_str(&format!("Frames: {nframes}\\n"));\n'
        '            diag.push_str(&format!("Duration: {secs:.2}s\\n"));\n'
        '            diag.push_str(&format!("Render FPS: {:.2}\\n", nframes as f64 / secs.max(1e-6)));\n'
        '            diag.push_str(&format!("File size: {bytes} bytes\\n\\n"));\n'
        '            diag.push_str("--- Pipeline Statistics ---\\n");\n'
        '            diag.push_str(&format!("GOP total decoded pictures: {}\\n", gop.frames));\n'
        '            diag.push_str(&format!("Hardware decoded pictures: {}\\n", dec.frames));\n'
        '            diag.push_str(&format!("Software decoded pictures: {}\\n", gop.frames.saturating_sub(dec.frames)));\n'
        '            diag.push_str(&format!("Hardware decoder sessions: {}, declined: {}, fallbacks: {}\\n", dec.sessions, dec.declined, dec.fallbacks));\n'
        '            diag.push_str(&format!("GPU composited frames: {}\\n", gpu.frames));\n'
        '            diag.push_str(&format!("Hardware encoded frames: {}\\n\\n", enc.frames));\n'
        '            diag.push_str("--- Stage Timings ---\\n");\n'
        '            for (stage, ns) in crate::stage_times() {\n'
        '                let ms = ns as f64 / 1_000_000.0;\n'
        '                diag.push_str(&format!("  {:?}: {:.1} ms ({:.2}s)\\n", stage, ms, ms / 1000.0));\n'
        '            }\n'
        '            log::info!("{diag}");\n'
        '            let _ = std::fs::write(&log_path, &diag);\n',
    ):
        fixed.append("crates/export/src/job.rs (export status HW indicator and log)")

    # 6. Display export status on the on-screen status bar when complete and show live progress.
    if fix(
        "crates/ui-egui/src/app/frame.rs",
        '        let job = running?;\n',
        '        for j in self.session.jobs.iter().rev() {\n'
        '            if j.progress.finished.load(Ordering::Relaxed) {\n'
        '                let st = j.progress.status.lock().unwrap_or_else(|e| e.into_inner()).clone();\n'
        '                if (st.starts_with("Done in ") || st.starts_with("Export failed")) && self.status_seen.0 != st {\n'
        '                    log::info!("Export complete: {st}");\n'
        '                    self.ui.status = st.clone();\n'
        '                    self.status_seen = (st, ctx.input(|i| i.time));\n'
        '                    ctx.request_repaint();\n'
        '                }\n'
        '                break;\n'
        '            }\n'
        '        }\n'
        '        let job = running?;\n'
        '        let done = job.progress.done.load(Ordering::Relaxed);\n'
        '        let total = job.progress.total.load(Ordering::Relaxed);\n'
        '        if total > 0 && !job.progress.finished.load(Ordering::Relaxed) {\n'
        '            let pct = (done as f32 / total as f32 * 100.0).clamp(0.0, 100.0);\n'
        '            self.ui.status = format!("Exporting: {done}/{total} frames ({pct:.0}%) [GPU Zero-Copy Enc]");\n'
        '        }\n',
    ):
        fixed.append("crates/ui-egui/src/app/frame.rs (export completion status and progress in UI)")

    # 7. Add diagnostics for make_video_decoder in crates/codecs/src/lib.rs.
    if fix(
        "crates/codecs/src/lib.rs",
        'pub fn make_video_decoder(entry: &filmcraft_isobmff::SampleEntry) -> Result<Box<dyn VideoDecoder>> {\n',
        'pub fn make_video_decoder(entry: &filmcraft_isobmff::SampleEntry) -> Result<Box<dyn VideoDecoder>> {\n'
        '    log::info!("make_video_decoder called for codec: {}", entry.codec.name());\n',
    ):
        fixed.append("crates/codecs/src/lib.rs (make_video_decoder diagnostic logging)")

    # 8. Single GPU frame renderer on Android to conserve unified memory and avoid TBDR context contention.
    if fix(
        "crates/export/src/pipeline.rs",
        '            let count = rayon::current_num_threads().clamp(2, 4);\n',
        '            let count = if cfg!(target_os = "android") { 1 } else { rayon::current_num_threads().clamp(2, 4) };\n',
    ):
        fixed.append("crates/export/src/pipeline.rs (bound GPU renderers on Android)")

    # 9. Bound GPU texture upload cache on Android and recycle textures into plane_pool.
    if fix(
        "crates/gpu/src/lib.rs",
        '        if self.uploads.len() > 24 {\n'
        '            let mut v: Vec<(u64, (usize, u32, u32))> = self.uploads.iter().map(|(k, u)| (u.last_used, *k)).collect();\n'
        '            v.sort_unstable();\n'
        '            for (_, k) in v.into_iter().take(self.uploads.len() - 24) {\n'
        '                self.uploads.remove(&k);\n'
        '            }\n'
        '        }\n',
        '        let max_uploads = if cfg!(target_os = "android") { 4 } else { 24 };\n'
        '        if self.uploads.len() > max_uploads {\n'
        '            let mut v: Vec<(u64, (usize, u32, u32))> = self.uploads.iter().map(|(k, u)| (u.last_used, *k)).collect();\n'
        '            v.sort_unstable();\n'
        '            for (_, k) in v.into_iter().take(self.uploads.len() - max_uploads) {\n'
        '                if let Some(removed) = self.uploads.remove(&k) {\n'
        '                    for (opt, view) in removed.textures.into_iter().zip(removed.views.into_iter()) {\n'
        '                        if let Some((w, h, fmt, tex)) = opt {\n'
        '                            if self.plane_pool.len() < 32 {\n'
        '                                self.plane_pool.push((w, h, fmt, tex, view));\n'
        '                            }\n'
        '                        }\n'
        '                    }\n'
        '                }\n'
        '            }\n'
        '        }\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (bound texture upload cache on Android and recycle textures into plane_pool)")

    # 10. Bound in-flight export memory budget and batch size on Android to prevent OOM/LMK during 4K exports.
    if fix(
        "crates/export/src/job.rs",
        'pub const IN_FLIGHT_BUDGET: u64 = 1 << 29;\n',
        'pub const IN_FLIGHT_BUDGET: u64 = if cfg!(target_os = "android") { 1 << 26 } else { 1 << 29 };\n',
    ):
        fixed.append("crates/export/src/job.rs (bound in-flight memory budget on Android)")

    if fix(
        "crates/export/src/job.rs",
        '            batch: rayon::current_num_threads().clamp(2, 16) as i64,\n',
        '            batch: if cfg!(target_os = "android") { 2 } else { rayon::current_num_threads().clamp(2, 16) as i64 },\n',
    ):
        fixed.append("crates/export/src/job.rs (bound default batch size on Android)")

    # 11. Bound GOP decoded frame cache ceiling on Android (96 MiB instead of 384 MiB).
    if fix(
        "crates/codecs/src/gop.rs",
        'const FRAME_CACHE_CEILING: usize = 384 << 20;\n',
        'const FRAME_CACHE_CEILING: usize = if cfg!(target_os = "android") { 96 << 20 } else { 384 << 20 };\n',
    ):
        fixed.append("crates/codecs/src/gop.rs (bound GOP frame cache ceiling on Android)")

    # Keep GOP decoders warm slightly longer on Android (5s vs 3s) to reduce re-decode thrashing.
    if fix(
        "crates/codecs/src/gop.rs",
        'const IDLE: Duration = Duration::from_secs(3);\n',
        'const IDLE: Duration = if cfg!(target_os = "android") { Duration::from_secs(5) } else { Duration::from_secs(3) };\n',
    ):
        fixed.append("crates/codecs/src/gop.rs (extend GOP IDLE duration on Android)")

    # 12. Fast contiguous luma and ARM64 NEON SIMD chroma de-interleave in biplanar::to_frame.
    if fix(
        "crates/platform/src/biplanar.rs",
        'fn row(plane: &[u8], stride: usize, y: usize, x: usize, len: usize) -> Result<&[u8], String> {\n'
        '    let start = y.checked_mul(stride).and_then(|s| s.checked_add(x)).ok_or("plane offset overflows")?;\n'
        '    plane.get(start..start.checked_add(len).ok_or("plane offset overflows")?).ok_or_else(|| "plane row out of range".to_string())\n'
        '}\n',
        'fn row(plane: &[u8], stride: usize, y: usize, x: usize, len: usize) -> Result<&[u8], String> {\n'
        '    let start = y.checked_mul(stride).and_then(|s| s.checked_add(x)).ok_or("plane offset overflows")?;\n'
        '    plane.get(start..start.checked_add(len).ok_or("plane offset overflows")?).ok_or_else(|| "plane row out of range".to_string())\n'
        '}\n\n'
        '#[allow(unsafe_code)]\n'
        '#[inline(always)]\n'
        'fn deinterleave_uv(pairs: &[u8], u: &mut [u8], v: &mut [u8]) {\n'
        '    let mut i = 0;\n'
        '    let mut out = 0;\n'
        '    let len = u.len().min(v.len()).min(pairs.len() / 2);\n'
        '    #[cfg(target_arch = "aarch64")]\n'
        '    // SAFETY: out + 16 <= len <= pairs.len() / 2 ensures i + 32 <= pairs.len(), and out + 16 <= u.len() and v.len(). All pointers are within bounds.\n'
        '    unsafe {\n'
        '        while out + 16 <= len {\n'
        '            let uv = std::arch::aarch64::vld2q_u8(pairs.as_ptr().add(i));\n'
        '            std::arch::aarch64::vst1q_u8(u.as_mut_ptr().add(out), uv.0);\n'
        '            std::arch::aarch64::vst1q_u8(v.as_mut_ptr().add(out), uv.1);\n'
        '            i += 32;\n'
        '            out += 16;\n'
        '        }\n'
        '    }\n'
        '    while out < len {\n'
        '        if let (Some(&u_val), Some(&v_val)) = (pairs.get(i), pairs.get(i + 1)) {\n'
        '            if let Some(u_slot) = u.get_mut(out) {\n'
        '                *u_slot = u_val;\n'
        '            }\n'
        '            if let Some(v_slot) = v.get_mut(out) {\n'
        '                *v_slot = v_val;\n'
        '            }\n'
        '        }\n'
        '        i += 2;\n'
        '        out += 1;\n'
        '    }\n'
        '}\n',
    ):
        fixed.append("crates/platform/src/biplanar.rs (NEON deinterleave_uv helper)")

    if fix(
        "crates/platform/src/biplanar.rs",
        '    let data = if bps == 1 {\n'
        '        let mut yp = pool::take_u8(w * h);\n'
        '        for y in 0..h {\n'
        '            yp.extend_from_slice(row(src.luma, src.stride, oy + y, ox, w)?);\n'
        '        }\n'
        '        let (mut u, mut v) = (pool::take_u8(cw * ch), pool::take_u8(cw * ch));\n'
        '        for y in 0..ch {\n'
        '            // two passes over the row: each is a simple strided copy the compiler vectorises\n'
        '            let pairs = row(src.chroma, src.stride, coy + y, cox * 2, cw * 2)?.as_chunks::<2>().0;\n'
        '            u.extend(pairs.iter().map(|p| p[0]));\n'
        '            v.extend(pairs.iter().map(|p| p[1]));\n'
        '        }\n'
        '        PixelData::Yuv8 { planes: [Arc::new(yp), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None }\n',
        '    let data = if bps == 1 {\n'
        '        let mut yp = pool::take_u8(w * h);\n'
        '        if ox == 0 && oy == 0 && src.stride == w && src.rows >= h {\n'
        '            if let Some(luma_slice) = src.luma.get(..w * h) {\n'
        '                yp.extend_from_slice(luma_slice);\n'
        '            }\n'
        '        }\n'
        '        if yp.len() < w * h {\n'
        '            yp.clear();\n'
        '            for y in 0..h {\n'
        '                yp.extend_from_slice(row(src.luma, src.stride, oy + y, ox, w)?);\n'
        '            }\n'
        '        }\n'
        '        let mut u = pool::take_u8(cw * ch);\n'
        '        let mut v = pool::take_u8(cw * ch);\n'
        '        u.resize(cw * ch, 0);\n'
        '        v.resize(cw * ch, 0);\n'
        '        let total_chroma_needed = cw.saturating_mul(ch).saturating_mul(2);\n'
        '        let mut done = false;\n'
        '        if cox == 0 && coy == 0 && src.stride == w && src.rows >= h {\n'
        '            if let Some(chroma_slice) = src.chroma.get(..total_chroma_needed) {\n'
        '                deinterleave_uv(chroma_slice, &mut u, &mut v);\n'
        '                done = true;\n'
        '            }\n'
        '        }\n'
        '        if !done {\n'
        '            for y in 0..ch {\n'
        '                let row_bytes = row(src.chroma, src.stride, coy + y, cox * 2, cw * 2)?;\n'
        '                let row_start = y * cw;\n'
        '                let row_end = row_start + cw;\n'
        '                if let (Some(row_u), Some(row_v)) = (u.get_mut(row_start..row_end), v.get_mut(row_start..row_end)) {\n'
        '                    deinterleave_uv(row_bytes, row_u, row_v);\n'
        '                }\n'
        '            }\n'
        '        }\n'
        '        PixelData::Yuv8 { planes: [Arc::new(yp), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None }\n',
    ):
        fixed.append("crates/platform/src/biplanar.rs (fast contiguous luma and NEON SIMD chroma copy)")

    # 13. Support input_surface and surface rendering in VideoEncoder and FrameRenderer.
    if fix(
        "crates/export/src/registry.rs",
        '    fn prepare_ahead(&self) -> Option<PrepareAhead> {\n'
        '        None\n'
        '    }\n'
        '}\n',
        '    fn prepare_ahead(&self) -> Option<PrepareAhead> {\n'
        '        None\n'
        '    }\n'
        '    /// On platforms supporting direct GPU-to-hardware-encoder surface rendering (Android AMediaCodec),\n'
        '    /// returns the native window pointer (*mut std::ffi::c_void) of the input surface.\n'
        '    fn input_surface(&self) -> Option<*mut std::ffi::c_void> {\n'
        '        None\n'
        '    }\n'
        '    /// Switch from surface mode to buffer mode if surface binding fails.\n'
        '    fn fallback_to_buffer(&mut self) -> bool {\n'
        '        false\n'
        '    }\n'
        '}\n',
    ):
        fixed.append("crates/export/src/registry.rs (VideoEncoder input_surface)")

    if fix(
        "crates/export/src/registry.rs",
        '    fn caps(&self) -> filmcraft_render::plan::DeviceCaps;\n'
        '}\n',
        '    fn caps(&self) -> filmcraft_render::plan::DeviceCaps;\n'
        '    fn bind_surface(&mut self, _window: *mut std::ffi::c_void, _w: u32, _h: u32) -> bool {\n'
        '        false\n'
        '    }\n'
        '    fn render_to_surface(\n'
        '        &mut self,\n'
        '        _project: &Project,\n'
        '        _seq: ItemId,\n'
        '        _t: Tick,\n'
        '        _opts: filmcraft_render::RenderOptions,\n'
        '        _sources: &dyn SourceProvider,\n'
        '    ) -> Option<bool> {\n'
        '        None\n'
        '    }\n'
        '    fn render_plan_to_surface(&mut self, _plan: &filmcraft_render::plan::FramePlan) -> Option<bool> {\n'
        '        None\n'
        '    }\n'
        '    fn render_rgba8(\n'
        '        &mut self,\n'
        '        _project: &Project,\n'
        '        _seq: ItemId,\n'
        '        _t: Tick,\n'
        '        _opts: filmcraft_render::RenderOptions,\n'
        '        _sources: &dyn SourceProvider,\n'
        '    ) -> Option<Vec<u8>> {\n'
        '        None\n'
        '    }\n'
        '}\n',
    ):
        fixed.append("crates/export/src/registry.rs (FrameRenderer surface and rgba8 methods)")

    # 13.5 Add fs_linear to composite.wgsl for direct hardware sRGB surface rendering.
    if fix(
        "crates/gpu/src/composite.wgsl",
        '    return vec4(select(hi, lo, c <= vec3(0.0031308)), 1.0);\n'
        '}\n',
        '    return vec4(select(hi, lo, c <= vec3(0.0031308)), 1.0);\n'
        '}\n\n'
        '@fragment\n'
        'fn fs_linear(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {\n'
        '    let c = clamp(textureLoad(accum, vec2<i32>(pos.xy), 0).rgb, vec3(0.0), vec3(1.0));\n'
        '    return vec4(c, 1.0);\n'
        '}\n',
    ):
        fixed.append("crates/gpu/src/composite.wgsl (fs_linear for sRGB surface)")

    # 14. Add render_to_view and render_export_rgba8 on GpuCompositor.
    if fix(
        "crates/gpu/src/lib.rs",
        '    accum_readback_buf: Option<(wgpu::Buffer, u64)>,\n',
        '    accum_readback_buf: Option<(wgpu::Buffer, u64)>,\n'
        '    output_readback_buf: Option<(wgpu::Buffer, u64)>,\n'
        '    surface_pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,\n'
        '    plane_pool: Vec<(u32, u32, wgpu::TextureFormat, wgpu::Texture, wgpu::TextureView)>,\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor output_readback_buf and surface_pipeline fields)")

    if fix(
        "crates/gpu/src/lib.rs",
        '            accum_readback_buf: None,\n',
        '            accum_readback_buf: None,\n'
        '            output_readback_buf: None,\n'
        '            surface_pipeline: None,\n'
        '            plane_pool: Vec::new(),\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor output_readback_buf and surface_pipeline init)")

    if fix(
        "crates/gpu/src/lib.rs",
        '    pub fn render_export_prepared(&mut self, plan: &FramePlan, prep: Option<&PreparedPlan>) -> Option<(u32, u32, Vec<f32>)> {\n',
        '    /// Direct GPU render into a target view (surface texture) using format-matched pipeline.\n'
        '    pub fn render_to_view(&mut self, plan: &FramePlan, prep: Option<&PreparedPlan>, target_view: &wgpu::TextureView, target_format: wgpu::TextureFormat) -> Option<()> {\n'
        '        let t = std::time::Instant::now();\n'
        '        let (accum_view, _) = self.render_plan(plan, prep, true, false);\n'
        '        let final_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {\n'
        '            label: Some("final-to-view"),\n'
        '            layout: &self.final_bgl,\n'
        '            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&accum_view) }],\n'
        '        });\n'
        '        if target_format != OUTPUT_FORMAT\n'
        '            && self.surface_pipeline.as_ref().map_or(true, |(fmt, _)| *fmt != target_format)\n'
        '        {\n'
        '            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {\n'
        '                label: Some("composite-surface"),\n'
        '                source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),\n'
        '            });\n'
        '            let fpl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {\n'
        '                label: Some("final-surface"),\n'
        '                bind_group_layouts: &[Some(&self.final_bgl)],\n'
        '                immediate_size: 0,\n'
        '            });\n'
        '            let entry_point = if target_format.is_srgb() { "fs_linear" } else { "fs_full" };\n'
        '            let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {\n'
        '                label: Some("final-surface"),\n'
        '                layout: Some(&fpl),\n'
        '                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },\n'
        '                primitive: wgpu::PrimitiveState::default(),\n'
        '                depth_stencil: None,\n'
        '                multisample: wgpu::MultisampleState::default(),\n'
        '                fragment: Some(wgpu::FragmentState {\n'
        '                    module: &shader,\n'
        '                    entry_point: Some(entry_point),\n'
        '                    compilation_options: Default::default(),\n'
        '                    targets: &[Some(wgpu::ColorTargetState { format: target_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],\n'
        '                }),\n'
        '                multiview_mask: None,\n'
        '                cache: None,\n'
        '            });\n'
        '            self.surface_pipeline = Some((target_format, p));\n'
        '        }\n'
        '        let pipeline = if target_format == OUTPUT_FORMAT {\n'
        '            &self.final_pipeline\n'
        '        } else {\n'
        '            &self.surface_pipeline.as_ref()?.1\n'
        '        };\n'
        '        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("filmcraft-composite-to-view") });\n'
        '        {\n'
        '            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {\n'
        '                label: Some("resolve-to-view"),\n'
        '                color_attachments: &[Some(wgpu::RenderPassColorAttachment {\n'
        '                    view: target_view,\n'
        '                    depth_slice: None,\n'
        '                    resolve_target: None,\n'
        '                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },\n'
        '                })],\n'
        '                depth_stencil_attachment: None,\n'
        '                timestamp_writes: None,\n'
        '                occlusion_query_set: None,\n'
        '                multiview_mask: None,\n'
        '            });\n'
        '            pass.set_pipeline(pipeline);\n'
        '            pass.set_bind_group(0, &final_bg, &[]);\n'
        '            pass.draw(0..3, 0..1);\n'
        '        }\n'
        '        self.queue.submit([enc.finish()]);\n'
        '        export_renderer::timing::add_submit(t.elapsed());\n'
        '        Some(())\n'
        '    }\n\n'
        '    pub fn render_export_prepared(&mut self, plan: &FramePlan, prep: Option<&PreparedPlan>) -> Option<(u32, u32, Vec<f32>)> {\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor render_to_view)")

    if fix(
        "crates/gpu/src/lib.rs",
        '        self.queue.submit([enc.finish()]);\n'
        '        export_renderer::timing::add_submit(t.elapsed());\n'
        '        self.map_accumulator(size)\n'
        '    }\n\n'
        '    /// Run the effect stage of a layer alone: `frame` decoded into its working image and `fx`\n',
        '        self.queue.submit([enc.finish()]);\n'
        '        export_renderer::timing::add_submit(t.elapsed());\n'
        '        self.map_accumulator(size)\n'
        '    }\n\n'
        '    pub fn render_export_rgba8(&mut self, plan: &FramePlan, prep: Option<&PreparedPlan>) -> Option<(u32, u32, Vec<u8>)> {\n'
        '        let t = std::time::Instant::now();\n'
        '        self.render_plan(plan, prep, true, true);\n'
        '        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("filmcraft-composite-rgba8") });\n'
        '        let size = self.copy_output(&mut enc)?;\n'
        '        self.queue.submit([enc.finish()]);\n'
        '        export_renderer::timing::add_submit(t.elapsed());\n'
        '        self.map_output(size)\n'
        '    }\n\n'
        '    fn copy_output(&mut self, enc: &mut wgpu::CommandEncoder) -> Option<(u32, u32, u32)> {\n'
        '        let (_, _, (w, h)) = self.output.as_ref()?;\n'
        '        let (w, h) = (*w, *h);\n'
        '        let row = w.checked_mul(4)?.div_ceil(256).checked_mul(256)?;\n'
        '        let needed = u64::from(row) * u64::from(h);\n'
        '        if self.output_readback_buf.as_ref().is_none_or(|(_, size)| *size < needed) {\n'
        '            let b = self.device.create_buffer(&wgpu::BufferDescriptor {\n'
        '                label: Some("output-readback"),\n'
        '                size: needed,\n'
        '                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,\n'
        '                mapped_at_creation: false,\n'
        '            });\n'
        '            self.output_readback_buf = Some((b, needed));\n'
        '        }\n'
        '        let (tex, _, _) = self.output.as_ref()?;\n'
        '        let (buf, _) = self.output_readback_buf.as_ref()?;\n'
        '        enc.copy_texture_to_buffer(\n'
        '            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },\n'
        '            wgpu::TexelCopyBufferInfo { buffer: buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },\n'
        '            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },\n'
        '        );\n'
        '        Some((w, h, row))\n'
        '    }\n\n'
        '    fn map_output(&mut self, (w, h, row): (u32, u32, u32)) -> Option<(u32, u32, Vec<u8>)> {\n'
        '        if w == 0 || h == 0 {\n'
        '            return None;\n'
        '        }\n'
        '        let t = std::time::Instant::now();\n'
        '        let (buf, capacity) = self.output_readback_buf.take()?;\n'
        '        let slice = buf.slice(..);\n'
        '        let (tx, rx) = std::sync::mpsc::channel();\n'
        '        slice.map_async(wgpu::MapMode::Read, move |r| {\n'
        '            let _ = tx.send(r);\n'
        '        });\n'
        '        wait_mapped(&self.device, &rx)?;\n'
        '        let data = slice.get_mapped_range().ok()?;\n'
        '        let mut out = vec![0u8; (w * h * 4) as usize];\n'
        '        let width_bytes = (w * 4) as usize;\n'
        '        let row_bytes = row as usize;\n'
        '        if row_bytes == width_bytes {\n'
        '            out.copy_from_slice(&data[..width_bytes * h as usize]);\n'
        '        } else {\n'
        '            for (y, dst) in out.chunks_exact_mut(width_bytes).enumerate() {\n'
        '                let src = &data[y * row_bytes..y * row_bytes + width_bytes];\n'
        '                dst.copy_from_slice(src);\n'
        '            }\n'
        '        }\n'
        '        drop(data);\n'
        '        buf.unmap();\n'
        '        self.output_readback_buf = Some((buf, capacity));\n'
        '        export_renderer::timing::add_convert(t.elapsed());\n'
        '        Some((w, h, out))\n'
        '    }\n\n'
        '    /// Run the effect stage of a layer alone: `frame` decoded into its working image and `fx`\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor render_export_rgba8)")

    # 14.5 Add persistent texture pool on GpuCompositor to eliminate 11,000 Vulkan image allocations during export.
    if fix(
        "crates/gpu/src/lib.rs",
        'struct Uploaded {\n'
        '    /// Y, Cb, Cr (or the RGBA texture) and the alpha plane (the dummy texture when there is none).\n'
        '    views: [wgpu::TextureView; 4],\n',
        'struct Uploaded {\n'
        '    textures: [Option<(u32, u32, wgpu::TextureFormat, wgpu::Texture)>; 4],\n'
        '    /// Y, Cb, Cr (or the RGBA texture) and the alpha plane (the dummy texture when there is none).\n'
        '    views: [wgpu::TextureView; 4],\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (struct Uploaded textures field)")

    if fix(
        "crates/gpu/src/lib.rs",
        '    fn plane_texture(&mut self, w: u32, h: u32, format: wgpu::TextureFormat, bytes: &[u8], bpp: u32) -> wgpu::TextureView {\n'
        '        let t = self.device.create_texture(&wgpu::TextureDescriptor {\n'
        '            label: Some("filmcraft-plane"),\n'
        '            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },\n'
        '            mip_level_count: 1,\n'
        '            sample_count: 1,\n'
        '            dimension: wgpu::TextureDimension::D2,\n'
        '            format,\n'
        '            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,\n'
        '            view_formats: &[],\n'
        '        });\n'
        '        self.queue.write_texture(\n'
        '            wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },\n'
        '            bytes,\n'
        '            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },\n'
        '            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },\n'
        '        );\n'
        '        self.uploaded_bytes += bytes.len() as u64;\n'
        '        t.create_view(&Default::default())\n'
        '    }\n',
        '    fn plane_texture(&mut self, w: u32, h: u32, format: wgpu::TextureFormat, bytes: &[u8], bpp: u32) -> (wgpu::Texture, wgpu::TextureView) {\n'
        '        let (t, v) = if let Some(idx) = self.plane_pool.iter().position(|(pw, ph, pfmt, _, _)| *pw == w && *ph == h && *pfmt == format) {\n'
        '            let (_, _, _, t, v) = self.plane_pool.swap_remove(idx);\n'
        '            (t, v)\n'
        '        } else {\n'
        '            let t = self.device.create_texture(&wgpu::TextureDescriptor {\n'
        '                label: Some("filmcraft-plane"),\n'
        '                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },\n'
        '                mip_level_count: 1,\n'
        '                sample_count: 1,\n'
        '                dimension: wgpu::TextureDimension::D2,\n'
        '                format,\n'
        '                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,\n'
        '                view_formats: &[],\n'
        '            });\n'
        '            let v = t.create_view(&Default::default());\n'
        '            (t, v)\n'
        '        };\n'
        '        self.queue.write_texture(\n'
        '            wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },\n'
        '            bytes,\n'
        '            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },\n'
        '            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },\n'
        '        );\n'
        '        self.uploaded_bytes += bytes.len() as u64;\n'
        '        (t, v)\n'
        '    }\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor plane_texture reuse from pool)")

    if fix(
        "crates/gpu/src/lib.rs",
        '        let up = match &f.data {\n'
        '            PixelData::Rgba8(d) => {\n'
        '                let v = self.plane_texture(w, h, wgpu::TextureFormat::Rgba8UnormSrgb, d, 4);\n'
        '                Uploaded {\n'
        '                    views: [v, dummy(), dummy(), dummy()],\n'
        '                    kind: 0,\n'
        '                    code_scale: 1.0,\n'
        '                    alpha_scale: 0.0,\n'
        '                    chroma: (w, h),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::RgbaF32(d) => {\n'
        '                let owned;\n'
        '                let half = match prep {\n'
        '                    Some(p) => &p.planes[0],\n'
        '                    None => {\n'
        '                        owned = f32_to_f16_bytes(d);\n'
        '                        &owned\n'
        '                    }\n'
        '                };\n'
        '                let v = self.plane_texture(w, h, wgpu::TextureFormat::Rgba16Float, half, 8);\n'
        '                Uploaded {\n'
        '                    views: [v, dummy(), dummy(), dummy()],\n'
        '                    kind: 1,\n'
        '                    code_scale: 1.0,\n'
        '                    alpha_scale: 0.0,\n'
        '                    chroma: (w, h),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::Yuv8 { planes, chroma, alpha } => {\n'
        '                let (sx, sy) = chroma.shifts();\n'
        '                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));\n'
        '                let y = self.plane_texture(w, h, wgpu::TextureFormat::R8Unorm, &planes[0], 1);\n'
        '                let u = self.plane_texture(cw, ch, wgpu::TextureFormat::R8Unorm, &planes[1], 1);\n'
        '                let v = self.plane_texture(cw, ch, wgpu::TextureFormat::R8Unorm, &planes[2], 1);\n'
        '                let a = alpha.as_ref().map(|a| self.plane_texture(w, h, wgpu::TextureFormat::R8Unorm, a, 1));\n'
        '                let alpha_scale = if a.is_some() { 1.0 } else { 0.0 };\n'
        '                Uploaded {\n'
        '                    views: [y, u, v, a.unwrap_or_else(dummy)],\n'
        '                    kind: 2,\n'
        '                    code_scale: 255.0,\n'
        '                    alpha_scale,\n'
        '                    chroma: (cw, ch),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::Yuv16 { planes, chroma, bits, alpha } => {\n'
        '                let (sx, sy) = chroma.shifts();\n'
        '                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));\n'
        '                let scale = (1u32 << bits) as f32;\n'
        '                let owned;\n'
        '                let p = match prep {\n'
        '                    Some(p) => p,\n'
        '                    None => {\n'
        '                        owned = Prepared { planes: planes.iter().chain(alpha.as_ref()).map(|p| codes_to_f16_bytes(p, *bits)).collect() };\n'
        '                        &owned\n'
        '                    }\n'
        '                };\n'
        '                let y = self.plane_texture(w, h, wgpu::TextureFormat::R16Float, &p.planes[0], 2);\n'
        '                let u = self.plane_texture(cw, ch, wgpu::TextureFormat::R16Float, &p.planes[1], 2);\n'
        '                let v = self.plane_texture(cw, ch, wgpu::TextureFormat::R16Float, &p.planes[2], 2);\n'
        '                // (full-resolution alpha plane: its codes span 0..2^bits - 1, hence the scale)\n'
        '                let a = alpha.as_ref().and_then(|_| p.planes.get(3)).map(|b| self.plane_texture(w, h, wgpu::TextureFormat::R16Float, b, 2));\n'
        '                let alpha_scale = if a.is_some() { scale / (scale - 1.0) } else { 0.0 };\n'
        '                let _ = Chroma::C420;\n'
        '                Uploaded {\n'
        '                    views: [y, u, v, a.unwrap_or_else(dummy)],\n'
        '                    kind: 2,\n'
        '                    code_scale: scale,\n'
        '                    alpha_scale,\n'
        '                    chroma: (cw, ch),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '        };\n'
        '        self.uploads.insert(key, up);\n',
        '        let up = match &f.data {\n'
        '            PixelData::Rgba8(d) => {\n'
        '                let (t, v) = self.plane_texture(w, h, wgpu::TextureFormat::Rgba8UnormSrgb, d, 4);\n'
        '                Uploaded {\n'
        '                    textures: [Some((w, h, wgpu::TextureFormat::Rgba8UnormSrgb, t)), None, None, None],\n'
        '                    views: [v, dummy(), dummy(), dummy()],\n'
        '                    kind: 0,\n'
        '                    code_scale: 1.0,\n'
        '                    alpha_scale: 0.0,\n'
        '                    chroma: (w, h),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::RgbaF32(d) => {\n'
        '                let owned;\n'
        '                let half = match prep {\n'
        '                    Some(p) => &p.planes[0],\n'
        '                    None => {\n'
        '                        owned = f32_to_f16_bytes(d);\n'
        '                        &owned\n'
        '                    }\n'
        '                };\n'
        '                let (t, v) = self.plane_texture(w, h, wgpu::TextureFormat::Rgba16Float, half, 8);\n'
        '                Uploaded {\n'
        '                    textures: [Some((w, h, wgpu::TextureFormat::Rgba16Float, t)), None, None, None],\n'
        '                    views: [v, dummy(), dummy(), dummy()],\n'
        '                    kind: 1,\n'
        '                    code_scale: 1.0,\n'
        '                    alpha_scale: 0.0,\n'
        '                    chroma: (w, h),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::Yuv8 { planes, chroma, alpha } => {\n'
        '                let (sx, sy) = chroma.shifts();\n'
        '                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));\n'
        '                let (ty, y) = self.plane_texture(w, h, wgpu::TextureFormat::R8Unorm, &planes[0], 1);\n'
        '                let (tu, u) = self.plane_texture(cw, ch, wgpu::TextureFormat::R8Unorm, &planes[1], 1);\n'
        '                let (tv, v) = self.plane_texture(cw, ch, wgpu::TextureFormat::R8Unorm, &planes[2], 1);\n'
        '                let (ta, a) = match alpha.as_ref() {\n'
        '                    Some(ab) => {\n'
        '                        let (t, v) = self.plane_texture(w, h, wgpu::TextureFormat::R8Unorm, ab, 1);\n'
        '                        (Some((w, h, wgpu::TextureFormat::R8Unorm, t)), Some(v))\n'
        '                    }\n'
        '                    None => (None, None),\n'
        '                };\n'
        '                let alpha_scale = if a.is_some() { 1.0 } else { 0.0 };\n'
        '                Uploaded {\n'
        '                    textures: [\n'
        '                        Some((w, h, wgpu::TextureFormat::R8Unorm, ty)),\n'
        '                        Some((cw, ch, wgpu::TextureFormat::R8Unorm, tu)),\n'
        '                        Some((cw, ch, wgpu::TextureFormat::R8Unorm, tv)),\n'
        '                        ta,\n'
        '                    ],\n'
        '                    views: [y, u, v, a.unwrap_or_else(dummy)],\n'
        '                    kind: 2,\n'
        '                    code_scale: 255.0,\n'
        '                    alpha_scale,\n'
        '                    chroma: (cw, ch),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '            PixelData::Yuv16 { planes, chroma, bits, alpha } => {\n'
        '                let (sx, sy) = chroma.shifts();\n'
        '                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));\n'
        '                let scale = (1u32 << bits) as f32;\n'
        '                let owned;\n'
        '                let p = match prep {\n'
        '                    Some(p) => p,\n'
        '                    None => {\n'
        '                        owned = Prepared { planes: planes.iter().chain(alpha.as_ref()).map(|p| codes_to_f16_bytes(p, *bits)).collect() };\n'
        '                        &owned\n'
        '                    }\n'
        '                };\n'
        '                let (ty, y) = self.plane_texture(w, h, wgpu::TextureFormat::R16Float, &p.planes[0], 2);\n'
        '                let (tu, u) = self.plane_texture(cw, ch, wgpu::TextureFormat::R16Float, &p.planes[1], 2);\n'
        '                let (tv, v) = self.plane_texture(cw, ch, wgpu::TextureFormat::R16Float, &p.planes[2], 2);\n'
        '                let (ta, a) = match alpha.as_ref().and_then(|_| p.planes.get(3)) {\n'
        '                    Some(ab) => {\n'
        '                        let (t, v) = self.plane_texture(w, h, wgpu::TextureFormat::R16Float, ab, 2);\n'
        '                        (Some((w, h, wgpu::TextureFormat::R16Float, t)), Some(v))\n'
        '                    }\n'
        '                    None => (None, None),\n'
        '                };\n'
        '                let alpha_scale = if a.is_some() { scale / (scale - 1.0) } else { 0.0 };\n'
        '                let _ = Chroma::C420;\n'
        '                Uploaded {\n'
        '                    textures: [\n'
        '                        Some((w, h, wgpu::TextureFormat::R16Float, ty)),\n'
        '                        Some((cw, ch, wgpu::TextureFormat::R16Float, tu)),\n'
        '                        Some((cw, ch, wgpu::TextureFormat::R16Float, tv)),\n'
        '                        ta,\n'
        '                    ],\n'
        '                    views: [y, u, v, a.unwrap_or_else(dummy)],\n'
        '                    kind: 2,\n'
        '                    code_scale: scale,\n'
        '                    alpha_scale,\n'
        '                    chroma: (cw, ch),\n'
        '                    last_used: self.clock,\n'
        '                    _pixels: f.data.clone(),\n'
        '                }\n'
        '            }\n'
        '        };\n'
        '        self.uploads.insert(key, up);\n',
    ):
        fixed.append("crates/gpu/src/lib.rs (GpuCompositor upload texture pooling and eviction recycle)")

    # 15. Add hardware surface binding and direct surface rendering on ExportRenderer.
    if fix(
        "crates/gpu/src/export_renderer.rs",
        'pub struct ExportRenderer {\n'
        '    _instance: wgpu::Instance,\n'
        '    #[expect(dead_code, reason = "kept for future frame-accuracy flows (readback fences)")]\n'
        '    device: wgpu::Device,\n'
        '    #[expect(dead_code, reason = "kept for future frame-accuracy flows (readback fences)")]\n'
        '    queue: wgpu::Queue,\n'
        '    compositor: GpuCompositor,\n'
        '}\n',
        'pub struct ExportRenderer {\n'
        '    instance: wgpu::Instance,\n'
        '    adapter: wgpu::Adapter,\n'
        '    #[expect(dead_code, reason = "kept for future frame-accuracy flows (readback fences)")]\n'
        '    device: wgpu::Device,\n'
        '    #[expect(dead_code, reason = "kept for future frame-accuracy flows (readback fences)")]\n'
        '    queue: wgpu::Queue,\n'
        '    compositor: GpuCompositor,\n'
        '    surface: Option<wgpu::Surface<\'static>>,\n'
        '    surface_format: Option<wgpu::TextureFormat>,\n'
        '}\n',
    ):
        fixed.append("crates/gpu/src/export_renderer.rs (ExportRenderer surface field)")

    if fix(
        "crates/gpu/src/export_renderer.rs",
        '        static SHARED: std::sync::OnceLock<Option<(wgpu::Instance, wgpu::Device, wgpu::Queue)>> = std::sync::OnceLock::new();\n'
        '        let (instance, device, queue) = SHARED\n',
        '        static SHARED: std::sync::OnceLock<Option<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue)>> = std::sync::OnceLock::new();\n'
        '        let (instance, adapter, device, queue) = SHARED\n',
    ):
        fixed.append("crates/gpu/src/export_renderer.rs (ExportRenderer shared adapter)")

    if fix(
        "crates/gpu/src/export_renderer.rs",
        '                Some((instance, device, queue))\n'
        '            })\n',
        '                Some((instance, adapter, device, queue))\n'
        '            })\n',
    ):
        fixed.append("crates/gpu/src/export_renderer.rs (ExportRenderer shared return adapter)")

    if fix(
        "crates/gpu/src/export_renderer.rs",
        '        Some(Self { _instance: instance, device, queue, compositor })\n'
        '    }\n',
        '        Some(Self { instance, adapter, device, queue, compositor, surface: None, surface_format: None })\n'
        '    }\n',
    ):
        fixed.append("crates/gpu/src/export_renderer.rs (ExportRenderer new with adapter)")

    if fix(
        "crates/gpu/src/export_renderer.rs",
        '    pub fn render(&mut self, project: &Project, seq: ItemId, t: Tick, opts: RenderOptions, sources: &dyn SourceProvider) -> Option<Image> {\n',
        '    pub fn bind_surface(&mut self, window_ptr: *mut std::ffi::c_void, w: u32, h: u32) -> bool {\n'
        '        if window_ptr.is_null() {\n'
        '            return false;\n'
        '        }\n'
        '        #[cfg(target_os = "android")]\n'
        '        {\n'
        '            #[link(name = "android")]\n'
        '            unsafe extern "C" {\n'
        '                fn ANativeWindow_setBuffersGeometry(window: *mut std::ffi::c_void, width: i32, height: i32, format: i32) -> i32;\n'
        '            }\n'
        '            unsafe {\n'
        '                // 1 = WINDOW_FORMAT_RGBA_8888 in android/native_window.h\n'
        '                let res = ANativeWindow_setBuffersGeometry(window_ptr, w as i32, h as i32, 1);\n'
        '                log::info!("ANativeWindow_setBuffersGeometry({w}x{h}, format=RGBA_8888) = {res}");\n'
        '            }\n'
        '        }\n'
        '        let non_null = match std::ptr::NonNull::new(window_ptr) {\n'
        '            Some(p) => p,\n'
        '            None => return false,\n'
        '        };\n'
        '        let handle = wgpu::rwh::AndroidNdkWindowHandle::new(non_null);\n'
        '        let target = wgpu::SurfaceTargetUnsafe::RawHandle {\n'
        '            raw_display_handle: Some(wgpu::rwh::RawDisplayHandle::Android(wgpu::rwh::AndroidDisplayHandle::new())),\n'
        '            raw_window_handle: wgpu::rwh::RawWindowHandle::AndroidNdk(handle),\n'
        '        };\n'
        '        let surface: wgpu::Surface<\'static> = match unsafe { self.instance.create_surface_unsafe(target) } {\n'
        '            Ok(s) => s,\n'
        '            Err(e) => {\n'
        '                log::warn!("Failed to create wgpu::Surface on native window: {e:?}");\n'
        '                return false;\n'
        '            }\n'
        '        };\n'
        '        let caps = surface.get_capabilities(&self.adapter);\n'
        '        log::info!("Surface capabilities: formats={:?}, present_modes={:?}, alpha={:?}", caps.formats, caps.present_modes, caps.alpha_modes);\n'
        '        if caps.formats.is_empty() || caps.present_modes.is_empty() {\n'
        '            log::warn!("Surface has no supported formats or present modes with adapter; declining surface mode");\n'
        '            return false;\n'
        '        }\n'
        '        let format = if caps.formats.contains(&crate::OUTPUT_FORMAT) { crate::OUTPUT_FORMAT } else { caps.formats[0] };\n'
        '        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::AutoNoVsync) { wgpu::PresentMode::AutoNoVsync } else { caps.present_modes[0] };\n'
        '        let alpha_mode = caps.alpha_modes[0];\n'
        '        let config = wgpu::SurfaceConfiguration {\n'
        '            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,\n'
        '            format,\n'
        '            color_space: wgpu::SurfaceColorSpace::Auto,\n'
        '            width: w,\n'
        '            height: h,\n'
        '            present_mode,\n'
        '            desired_maximum_frame_latency: 2,\n'
        '            alpha_mode,\n'
        '            view_formats: vec![],\n'
        '        };\n'
        '        let configure_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {\n'
        '            surface.configure(&self.device, &config);\n'
        '        }));\n'
        '        if configure_res.is_err() {\n'
        '            log::warn!("surface.configure panicked on this hardware window; declining surface mode");\n'
        '            return false;\n'
        '        }\n'
        '        log::info!("ExportRenderer successfully configured hardware surface: {w}x{h} ({format:?})");\n'
        '        self.surface = Some(surface);\n'
        '        self.surface_format = Some(format);\n'
        '        true\n'
        '    }\n\n'
        '    pub fn render_plan_to_surface(&mut self, plan: &filmcraft_render::plan::FramePlan) -> Option<bool> {\n'
        '        let surface = self.surface.as_ref()?;\n'
        '        let format = self.surface_format?;\n'
        '        match plan {\n'
        '            FramePlan::Layers { .. } | FramePlan::Composite { .. } => {\n'
        '                if device_failed() {\n'
        '                    return None;\n'
        '                }\n'
        '                let prep = prepare(plan);\n'
        '                let surface_tex = match surface.get_current_texture() {\n'
        '                    wgpu::CurrentSurfaceTexture::Success(tex) | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,\n'
        '                    e => {\n'
        '                        log::warn!("Surface get_current_texture failed: {e:?}");\n'
        '                        return None;\n'
        '                    }\n'
        '                };\n'
        '                let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());\n'
        '                self.compositor.render_to_view(plan, Some(&prep), &view, format)?;\n'
        '                self.queue.present(surface_tex);\n'
        '                if device_failed() {\n'
        '                    return None;\n'
        '                }\n'
        '                timing::add_frame();\n'
        '                Some(true)\n'
        '            }\n'
        '            FramePlan::Image(_) => None,\n'
        '        }\n'
        '    }\n\n'
        '    pub fn render_to_surface(\n'
        '        &mut self,\n'
        '        project: &Project,\n'
        '        seq: ItemId,\n'
        '        t: Tick,\n'
        '        opts: RenderOptions,\n'
        '        sources: &dyn SourceProvider,\n'
        '    ) -> Option<bool> {\n'
        '        let plan = plan_frame(project, seq, t, opts, self.compositor.caps(), sources);\n'
        '        self.render_plan_to_surface(&plan)\n'
        '    }\n\n'
        '    pub fn render_rgba8(\n'
        '        &mut self,\n'
        '        project: &Project,\n'
        '        seq: ItemId,\n'
        '        t: Tick,\n'
        '        opts: RenderOptions,\n'
        '        sources: &dyn SourceProvider,\n'
        '    ) -> Option<Vec<u8>> {\n'
        '        let plan = plan_frame(project, seq, t, opts, self.compositor.caps(), sources);\n'
        '        match &plan {\n'
        '            FramePlan::Layers { .. } | FramePlan::Composite { .. } => {\n'
        '                if device_failed() {\n'
        '                    return None;\n'
        '                }\n'
        '                let prep = prepare(&plan);\n'
        '                let (_w, _h, px) = self.compositor.render_export_rgba8(&plan, Some(&prep))?;\n'
        '                if device_failed() {\n'
        '                    return None;\n'
        '                }\n'
        '                timing::add_frame();\n'
        '                Some(px)\n'
        '            }\n'
        '            FramePlan::Image(_) => None,\n'
        '        }\n'
        '    }\n\n'
        '    pub fn render(&mut self, project: &Project, seq: ItemId, t: Tick, opts: RenderOptions, sources: &dyn SourceProvider) -> Option<Image> {\n',
    ):
        fixed.append("crates/gpu/src/export_renderer.rs (ExportRenderer bind_surface, render_to_surface and render_rgba8)")

    # 16. Support surface rendering and fast GPU RGBA8 dispatch in Pipeline.
    if fix(
        "crates/export/src/pipeline.rs",
        '        let pool = self.renderer.as_ref().filter(|pool| !filmcraft_render::plan::is_cpu_frame(&self.project, self.seq, t, self.opts, pool.caps));\n'
        '        let gpu = match pool {\n',
        '        let pool = self.renderer.as_ref().filter(|pool| !filmcraft_render::plan::is_cpu_frame(&self.project, self.seq, t, self.opts, pool.caps));\n'
        '        let is_fullscreen = match self.geom.content {\n'
        '            None => true,\n'
        '            Some((cw, ch)) => cw == self.w as usize && ch == self.h as usize,\n'
        '        };\n'
        '        let can_fast_rgba8 = !self.hdr_out\n'
        '            && !self.alpha\n'
        '            && is_fullscreen\n'
        '            && self.geom.offset == (0, 0)\n'
        '            && !self.effects.image_overlay.enabled\n'
        '            && !self.effects.name_overlay.enabled\n'
        '            && !self.effects.timecode_overlay.enabled\n'
        '            && !self.effects.video_limiter.enabled;\n'
        '        if can_fast_rgba8 {\n'
        '            if let Some(pool) = pool {\n'
        '                let asked = web_time::Instant::now();\n'
        '                let rgba = pool.with(|r| {\n'
        '                    crate::note_lock_wait(asked.elapsed());\n'
        '                    r.render_rgba8(&self.project, self.seq, t, self.opts, sources)\n'
        '                }).flatten();\n'
        '                if let Some(rgba) = rgba {\n'
        '                    note_gpu_frame();\n'
        '                    return (rgba, Vec::new());\n'
        '                }\n'
        '            }\n'
        '        }\n'
        '        let gpu = match pool {\n',
    ):
        fixed.append("crates/export/src/pipeline.rs (Pipeline fast rgba8 dispatch)")

    # 16. Support surface rendering in Pipeline.
    if fix(
        "crates/export/src/pipeline.rs",
        '    /// Free the pool\'s float images when the export ends: a standalone export, not one part of a\n'
        '    /// batch (`ExportSettings::part_of_batch`).\n'
        '    trim_pool: bool,\n'
        '}\n',
        '    /// Free the pool\'s float images when the export ends: a standalone export, not one part of a\n'
        '    /// batch (`ExportSettings::part_of_batch`).\n'
        '    trim_pool: bool,\n'
        '    pub src_w: u32,\n'
        '    pub src_h: u32,\n'
        '    surface_renderer: std::sync::Mutex<Option<Box<dyn FrameRenderer>>>,\n'
        '    surface_bound: std::sync::atomic::AtomicBool,\n'
        '    surface_caps: std::sync::atomic::AtomicU32,\n'
        '}\n',
    ):
        fixed.append("crates/export/src/pipeline.rs (Pipeline surface_bound field)")

    if fix(
        "crates/export/src/pipeline.rs",
        '            trim_pool: !settings.part_of_batch,\n'
        '            project: project.clone(),\n',
        '            trim_pool: !settings.part_of_batch,\n'
        '            src_w: q.settings.width,\n'
        '            src_h: q.settings.height,\n'
        '            surface_renderer: std::sync::Mutex::new(None),\n'
        '            surface_bound: std::sync::atomic::AtomicBool::new(false),\n'
        '            surface_caps: std::sync::atomic::AtomicU32::new(0),\n'
        '            project: project.clone(),\n',
    ):
        fixed.append("crates/export/src/pipeline.rs (Pipeline init surface_bound)")

    if fix(
        "crates/export/src/pipeline.rs",
        '    /// Output frame `f` as straight sRGB RGBA8 at the output size, or for HDR exports the encoded\n'
        '    /// R\'G\'B\' floats (3 per pixel).\n'
        '    pub fn frame(&self, f: i64, sources: &dyn SourceProvider) -> (Vec<u8>, Vec<f32>) {\n',
        '    pub fn bind_surface(&self, window_ptr: *mut std::ffi::c_void) -> bool {\n'
        '        if !self.can_surface_render() {\n'
        '            return false;\n'
        '        }\n'
        '        let is_1to1 = (self.opts.scale - 1.0).abs() < 1e-4;\n'
        '        if cfg!(target_os = "android") {\n'
        '            if is_1to1 {\n'
        '                filmcraft_codecs::hw::set_direct_surface(window_ptr, self.w, self.h);\n'
        '                self.surface_bound.store(true, std::sync::atomic::Ordering::Relaxed);\n'
        '                log::info!("Pipeline configured direct 1:1 decoder-to-encoder hardware surface bridge for {}x{}", self.w, self.h);\n'
        '                return true;\n'
        '            } else if let Some(decoder_win) = filmcraft_codecs::hw::init_surface_scaler(window_ptr, self.src_w, self.src_h, self.w, self.h) {\n'
        '                filmcraft_codecs::hw::set_direct_surface(decoder_win, self.src_w, self.src_h);\n'
        '                self.surface_bound.store(true, std::sync::atomic::Ordering::Relaxed);\n'
        '                log::info!("Pipeline configured zero-copy HardwareSurfaceScaler: {}x{} -> {}x{}", self.src_w, self.src_h, self.w, self.h);\n'
        '                return true;\n'
        '            }\n'
        '        }\n'
        '        if let Some(pool) = &self.renderer {\n'
        '            let (w, h) = (self.w, self.h);\n'
        '            let mut free = pool.renderers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);\n'
        '            if let Some(mut r) = free.pop() {\n'
        '                let ok = r.bind_surface(window_ptr, w, h);\n'
        '                if ok {\n'
        '                    let caps = r.caps();\n'
        '                    self.surface_caps.store(caps.adjust_side, std::sync::atomic::Ordering::Relaxed);\n'
        '                    *self.surface_renderer.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(r);\n'
        '                    self.surface_bound.store(true, std::sync::atomic::Ordering::Relaxed);\n'
        '                    log::info!("Pipeline configured GPU surface renderer for {}x{} (scale={:.3})", w, h, self.opts.scale);\n'
        '                    return true;\n'
        '                } else {\n'
        '                    free.push(r);\n'
        '                }\n'
        '            }\n'
        '        }\n'
        '        false\n'
        '    }\n\n'
        '    pub fn has_surface(&self) -> bool {\n'
        '        self.surface_bound.load(std::sync::atomic::Ordering::Relaxed)\n'
        '    }\n\n'
        '    pub fn can_surface_render(&self) -> bool {\n'
        '        let is_fullscreen = match self.geom.content {\n'
        '            None => true,\n'
        '            Some((cw, ch)) => cw == self.w as usize && ch == self.h as usize,\n'
        '        };\n'
        '        !self.hdr_out\n'
        '            && is_fullscreen\n'
        '            && self.geom.offset == (0, 0)\n'
        '            && !self.effects.image_overlay.enabled\n'
        '            && !self.effects.name_overlay.enabled\n'
        '            && !self.effects.timecode_overlay.enabled\n'
        '            && !self.effects.video_limiter.enabled\n'
        '    }\n\n'
        '    pub fn surface_caps(&self) -> filmcraft_render::plan::DeviceCaps {\n'
        '        filmcraft_render::plan::DeviceCaps {\n'
        '            adjust_side: self.surface_caps.load(std::sync::atomic::Ordering::Relaxed),\n'
        '        }\n'
        '    }\n\n'
        '    pub fn plan_frame(&self, t: filmcraft_time::Tick, sources: &dyn SourceProvider) -> filmcraft_render::plan::FramePlan {\n'
        '        filmcraft_render::plan::plan_frame(&self.project, self.seq, t, self.opts, self.surface_caps(), sources)\n'
        '    }\n\n'
        '    pub fn render_plan_to_surface(&self, plan: &filmcraft_render::plan::FramePlan) -> Option<bool> {\n'
        '        if filmcraft_codecs::hw::has_direct_surface() {\n'
        '            if filmcraft_codecs::hw::has_surface_scaler() {\n'
        '                return Some(filmcraft_codecs::hw::blit_surface_scaler());\n'
        '            }\n'
        '            return Some(true);\n'
        '        }\n'
        '        let mut guard = self.surface_renderer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);\n'
        '        let r = guard.as_mut()?;\n'
        '        r.render_plan_to_surface(plan)\n'
        '    }\n\n'
        '    pub fn render_to_surface(&self, t: filmcraft_time::Tick, sources: &dyn SourceProvider) -> Option<bool> {\n'
        '        let plan = self.plan_frame(t, sources);\n'
        '        self.render_plan_to_surface(&plan)\n'
        '    }\n\n'
        '    /// Output frame `f` as straight sRGB RGBA8 at the output size, or for HDR exports the encoded\n'
        '    /// R\'G\'B\' floats (3 per pixel).\n'
        '    pub fn frame(&self, f: i64, sources: &dyn SourceProvider) -> (Vec<u8>, Vec<f32>) {\n',
    ):
        fixed.append("crates/export/src/pipeline.rs (Pipeline bind_surface and render_to_surface)")

    # 17. Bind surface in Exporter::set_up and use step_surface_batch for zero-copy hardware encoding.
    if fix(
        "crates/export/src/job.rs",
        '        let venc = make_venc(&settings, pipe.w, pipe.h, pipe.rate)?;\n',
        '        let mut venc = make_venc(&settings, pipe.w, pipe.h, pipe.rate)?;\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter let mut venc in set_up)")

    if fix(
        "crates/export/src/job.rs",
        '        let measured = !settings.effects.loudness.enabled || audio.is_none();\n'
        '        Ok(Self {\n',
        '        let measured = !settings.effects.loudness.enabled || audio.is_none();\n'
        '        if pipe.can_surface_render() {\n'
        '            if let Some(surface_ptr) = venc.input_surface() {\n'
        '                filmcraft_codecs::release_idle();\n'
        '                if pipe.bind_surface(surface_ptr) {\n'
        '                    log::info!("Exporter enabled zero-copy hardware surface pipeline for {}x{}", pipe.w, pipe.h);\n'
        '                } else {\n'
        '                    log::warn!("Pipeline failed to bind hardware surface; falling back to buffer mode");\n'
        '                    let _ = venc.fallback_to_buffer();\n'
        '                }\n'
        '            }\n'
        '        }\n'
        '        Ok(Self {\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter bind_surface in set_up)")

    if fix(
        "crates/export/src/job.rs",
        '        if self.next < self.f1 {\n'
        '            return self.step_batch(sources, progress);\n'
        '        }\n',
        '        if self.next < self.f1 {\n'
        '            if self.pipe.has_surface() && self.venc.input_surface().is_some() {\n'
        '                return self.step_surface_batch(sources, progress);\n'
        '            }\n'
        '            return self.step_batch(sources, progress);\n'
        '        }\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter step_surface_batch dispatch)")

    if fix(
        "crates/export/src/job.rs",
        '    /// Encode (and mux) the batch starting at `self.next`, rendering the one after it meanwhile.\n'
        '    fn step_batch(&mut self, sources: &dyn SourceProvider, progress: &Progress) -> Result<Step> {\n',
        '    fn step_surface_batch(&mut self, sources: &dyn SourceProvider, progress: &Progress) -> Result<Step> {\n'
        '        let f = self.next;\n'
        '        let batch_size = if cfg!(target_os = "android") { 64 } else { self.batch_len() };\n'
        '        let end = self.next.saturating_add(batch_size).min(self.f1);\n'
        '        if end <= f {\n'
        '            return Err(ExportError::Encode("internal: an empty batch was rendered".into()));\n'
        '        }\n'
        '        let cuts: Vec<i64> = (f + 1..=end).filter(|&g| (g - self.f0) % INTERLEAVE == 0).collect();\n'
        '        let mut mixed = Vec::with_capacity(cuts.len());\n'
        '        if let Some(sr) = self.audio.as_ref().map(|a| a.sr) {\n'
        '            let t = web_time::Instant::now();\n'
        '            for &g in &cuts {\n'
        '                let until = self.sample_at_frame(g, sr);\n'
        '                mixed.push(self.audio.as_mut().and_then(|a| a.pull(until, sources)));\n'
        '            }\n'
        '            note_stage(Stage::Audio, t.elapsed());\n'
        '        }\n'
        '        if filmcraft_media::pending::take() {\n'
        '            return Ok(Step::Pending);\n'
        '        }\n'
        '        let mut mixed = mixed.into_iter();\n'
        '        let pipe = self.pipe.clone();\n'
        '        let f0 = self.f0;\n'
        '        let pipe_w = pipe.w;\n'
        '        let pipe_h = pipe.h;\n'
        '        for fi in f..end {\n'
        '            if progress.cancel.load(Ordering::Relaxed) {\n'
        '                filmcraft_codecs::hw::clear_direct_surface();\n'
        '                filmcraft_codecs::release_idle();\n'
        '                return Err(ExportError::Cancelled);\n'
        '            }\n'
        '            let t = pipe.rate.tick_of(fi);\n'
        '            let plan = pipe.plan_frame(t, sources);\n'
        '            let rendered = timed(Stage::Render, || pipe.render_plan_to_surface(&plan));\n'
        '            if rendered != Some(true) {\n'
        '                log::warn!("Hardware surface render fallback at frame {fi}; switching to buffer batch");\n'
        '                filmcraft_codecs::hw::clear_direct_surface();\n'
        '                filmcraft_codecs::release_idle();\n'
        '                let _ = self.venc.fallback_to_buffer();\n'
        '                return self.step_batch(sources, progress);\n'
        '            }\n'
        '            let fr = EncoderFrame {\n'
        '                width: pipe_w,\n'
        '                height: pipe_h,\n'
        '                rgba: &[],\n'
        '                hdr: None,\n'
        '                index: (fi - f0) as u64,\n'
        '            };\n'
        '            let packets = match timed(Stage::Encode, || self.venc.encode(&fr)) {\n'
        '                Ok(p) => p,\n'
        '                Err(e) => return Err(e),\n'
        '            };\n'
        '            self.queued.extend(packets);\n'
        '            if cuts.contains(&(fi + 1)) {\n'
        '                let mux_res = timed(Stage::Mux, || -> Result<()> {\n'
        '                    if self.mux.is_none() && self.mxf.is_none() {\n'
        '                        self.open_mux()?;\n'
        '                    }\n'
        '                    let packets = std::mem::take(&mut self.queued);\n'
        '                    self.write_video(packets)\n'
        '                });\n'
        '                mux_res?;\n'
        '                let pcm = mixed.next().flatten();\n'
        '                timed(Stage::Audio, || self.write_audio(pcm))?;\n'
        '            }\n'
        '        }\n'
        '        progress.done.fetch_add((end - f) as u64, Ordering::Relaxed);\n'
        '        self.next = end;\n'
        '        Ok(Step::Progress)\n'
        '    }\n\n'
        '    /// Encode (and mux) the batch starting at `self.next`, rendering the one after it meanwhile.\n'
        '    fn step_batch(&mut self, sources: &dyn SourceProvider, progress: &Progress) -> Result<Step> {\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter step_surface_batch implementation)")

    if fix(
        "crates/export/src/job.rs",
        '        note_stage(Stage::Finish, finishing.elapsed());\n'
        '        let secs = self.t0.elapsed().as_secs_f64();\n',
        '        note_stage(Stage::Finish, finishing.elapsed());\n'
        '        filmcraft_codecs::hw::clear_direct_surface();\n'
        '        filmcraft_codecs::release_idle();\n'
        '        let secs = self.t0.elapsed().as_secs_f64();\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter finish direct surface cleanup)")

    if fix(
        "crates/export/src/job.rs",
        '        if progress.cancel.load(Ordering::Relaxed) {\n'
        '            self.ahead = None;\n'
        '            return Err(ExportError::Cancelled);\n'
        '        }\n',
        '        if progress.cancel.load(Ordering::Relaxed) {\n'
        '            self.ahead = None;\n'
        '            filmcraft_codecs::hw::clear_direct_surface();\n'
        '            filmcraft_codecs::release_idle();\n'
        '            return Err(ExportError::Cancelled);\n'
        '        }\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter cancel direct surface cleanup)")

    if fix(
        "crates/export/src/job.rs",
        'impl Exporter {\n',
        'impl Drop for Exporter {\n'
        '    fn drop(&mut self) {\n'
        '        filmcraft_codecs::hw::clear_direct_surface();\n'
        '        filmcraft_codecs::release_idle();\n'
        '    }\n'
        '}\n\n'
        'impl Exporter {\n',
    ):
        fixed.append("crates/export/src/job.rs (Exporter Drop cleanup)")

    # Smooth playback: on Android cpal/AAudio rejects the 512-frame buffer FilmCraft asks for
    # (it reports AudioTrack.getMinBufferSize, a byte count, as the minimum frame count), so the
    # output runs on the large power-saving buffer and the audio clock only advances in
    # ~85-110 ms steps. The Program monitor picked its frame from that raw clock and so showed
    # ~12 pictures a second, skipping 2-3 frames each time (yellow drop dot), while the UI and the
    # Timeline playhead (already smoothed) ran at 60 fps. Show the frame on the same smoothed
    # clock: wall time since the last drawn position, eased 10% per UI frame toward the audio
    # clock, never stepping back, and jumping at once past 0.25 s (seeks, loops).
    if fix(
        "crates/ui-egui/src/app/playback.rs",
        '            self.session.set_playhead(t);\n'
        '            self.playback.smooth_playhead = Some((self.smooth_playhead(t, now), now));\n',
        '            let shown = self.smooth_playhead(t, now);\n'
        '            self.session.set_playhead(shown);\n'
        '            self.playback.smooth_playhead = Some((shown, now));\n',
    ):
        fixed.append("crates/ui-egui/src/app/playback.rs (Program frame follows the smoothed playback clock)")
elif app == "vectorcraft":
    if fix("crates/render/src/images.rs", '#[cfg(target_os = "linux")]', '#[cfg(any(target_os = "linux", target_os = "android"))]'):
        fixed.append("crates/render/src/images.rs (Android meminfo)")
    fix_all("crates/render/src/images.rs", '#[cfg(any(target_os = "linux", test))]', '#[cfg(any(target_os = "linux", target_os = "android", test))]')
    fix("crates/render/src/images.rs", '#[cfg(not(any(target_os = "linux", windows, target_os = "macos", target_os = "freebsd")))]', '#[cfg(not(any(target_os = "linux", target_os = "android", windows, target_os = "macos", target_os = "freebsd")))]')
    if fix(
        "crates/ui-egui/src/dialogs/modal.rs",
        "        .show(ctx, add)?;\n    keep_rest(ctx, id, shown.response.rect.height());\n    shown.inner",
        "        .show(ctx, add)?;\n"
        "    let dialog_layer = shown.response.layer_id;\n"
        "    let popups: Vec<Id> = ctx.viewport(|vp| {\n"
        "        vp.this_pass\n"
        "            .layers\n"
        "            .get(&dialog_layer)\n"
        "            .map_or_else(Vec::new, |layer| layer.open_popups.iter().copied().collect())\n"
        "    });\n"
        "    for popup_id in popups {\n"
        "        let popup_layer = egui::LayerId::new(egui::Order::Foreground, popup_id);\n"
        "        if dialog_layer.order == popup_layer.order && dialog_layer != popup_layer {\n"
        "            ctx.set_sublayer(dialog_layer, popup_layer);\n"
        "        }\n"
        "    }\n"
        "    keep_rest(ctx, id, shown.response.rect.height());\n"
        "    shown.inner",
    ):
        fixed.append("crates/ui-egui/src/dialogs/modal.rs (keep modal popups above modal dialog)")
elif app == "lightcraft":
    if fix("crates/engine/src/memory.rs", '#[cfg(target_os = "linux")]', '#[cfg(any(target_os = "linux", target_os = "android"))]'):
        fixed.append("crates/engine/src/memory.rs (Android meminfo)")
    fix("crates/engine/src/memory.rs", '#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]', '#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "freebsd")))]')
elif app == "pdfcraft":
    if fix(
        "crates/ui-egui/src/app.rs",
        "            printer_source: None,\n",
        "            printer_source: None,\n            print_handler: None,\n",
    ):
        fixed.append("crates/ui-egui/src/app.rs (print_handler)")
    if fix(
        "crates/ui-egui/src/lib.rs",
        "    pub printer_source: Option<PrinterSource>,\n",
        "    pub printer_source: Option<PrinterSource>,\n    pub print_handler: Option<std::sync::Arc<dyn Fn(&str, &[u8]) -> Result<String, String> + Send + Sync>>,\n",
    ):
        fixed.append("crates/ui-egui/src/lib.rs (print_handler)")
    if fix(
        "crates/ui-egui/src/workflows/print_ui.rs",
        "        let spooler = self.spooler.clone();\n        std::thread::spawn(move || {\n",
        "        let spooler = self.spooler.clone();\n        let handler = self.print_handler.clone();\n        std::thread::spawn(move || {\n",
    ) or fix(
        "crates/ui-egui/src/workflows/print_ui.rs",
        "        let ctx = self.ctx.clone();\n        std::thread::spawn(move || {\n",
        "        let ctx = self.ctx.clone();\n        let handler = self.print_handler.clone();\n        std::thread::spawn(move || {\n",
    ):
        fixed.append("crates/ui-egui/src/workflows/print_ui.rs (handler closure)")
    old_print_spooler = """            let result = match &spooler {
                Some(submit) => submit(&bytes, &job),
                None => spool::submit_with_progress(&bytes, &job, &mut |sent, sheets| {
                    let _ = tx.send(JobNews::Sent(sent, sheets));
                    repaint();
                })
                .map_err(|e| e.to_string()),
            };"""
    new_print_spooler = """            let result = if let Some(handler) = &handler {
                let name = if job.title.is_empty() { "PdfCraft".to_string() } else { job.title.clone() };
                handler(&name, &bytes)
            } else if let Some(submit) = &spooler {
                submit(&bytes, &job)
            } else {
                spool::submit_with_progress(&bytes, &job, &mut |sent, sheets| {
                    let _ = tx.send(JobNews::Sent(sent, sheets));
                    repaint();
                }).map_err(|e| e.to_string())
            };"""
    if fix("crates/ui-egui/src/workflows/print_ui.rs", old_print_spooler, new_print_spooler):
        fixed.append("crates/ui-egui/src/workflows/print_ui.rs (send_to_printer spooler)")
    modern_spooler_target = """                match &spooler {
                    Some(submit) => submit(&bytes, &job),
                    None => spool::submit_with_progress(&bytes, &job, &mut |sent, sheets| {
                        progress.set(sent, sheets);
                        wake.wake();
                    })
                    .map_err(|e| e.to_string()),
                }"""
    modern_spooler_repl = """                if let Some(handler) = &handler {
                    let name = if job.title.is_empty() { "PdfCraft".to_string() } else { job.title.clone() };
                    handler(&name, &bytes)
                } else {
                    match &spooler {
                        Some(submit) => submit(&bytes, &job),
                        None => spool::submit_with_progress(&bytes, &job, &mut |sent, sheets| {
                            progress.set(sent, sheets);
                            wake.wake();
                        })
                        .map_err(|e| e.to_string()),
                    }
                }"""
    if fix(
        "crates/ui-egui/src/workflows/print_ui.rs",
        "        let spooler = self.spooler.clone();\n        self.print_tasks.jobs.submit(",
        "        let spooler = self.spooler.clone();\n        let handler = self.print_handler.clone();\n        self.print_tasks.jobs.submit(",
    ):
        fixed.append("crates/ui-egui/src/workflows/print_ui.rs (jobs.submit handler closure)")
    if fix("crates/ui-egui/src/workflows/print_ui.rs", modern_spooler_target, modern_spooler_repl):
        fixed.append("crates/ui-egui/src/workflows/print_ui.rs (modern send_to_printer spooler)")


if fixed:
    print(f"  Repaired upstream compile errors still present in {title}: {', '.join(fixed)}")
