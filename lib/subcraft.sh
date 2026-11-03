#!/usr/bin/env bash
# SubCraft's Android file hand-off — the app-side patch.
#
# SubCraft has no public way to hand the app a file it did not itself pick. Its own file dialogs go
# through the blocking `rfd::FileDialog`, which the builder's vendored `rfd` routes to Android's
# Storage Access Framework (see vendor/rfd and adapters/subcraft/), and the *answer* lands in the
# private `files::PICKED` queue. So a file the system opens the app with (an `ACTION_VIEW` on a
# `.subcraft` project, a mesh shared from a file manager, a dropped file) had nowhere to go.
#
# This exposes one public entry point — `files::deliver_android(purpose, paths)` — that pushes into
# that same queue. The Android shell calls it, so an incoming file is handled by exactly the code a
# picked file is: the app is never modified beyond adding this function.
#
# Guarded and idempotent: it applies only while the original text is still present, so it no-ops once
# the function is there, and it is re-applied on a fresh checkout.
#
# To test WITHOUT the patch, comment out the call to this script in lib/build-common.sh.
#
# Usage: subcraft.sh <source-dir> <app> <title>
set -euo pipefail

SRC="${1:?source dir}"
APP="${2:?app}"
TITLE="${3:?title}"

if [[ "$APP" != "subcraft" ]]; then
    exit 0
fi

python3 - "$SRC" "$APP" <<'PY'
from pathlib import Path
import os
import sys

src, app = sys.argv[1], sys.argv[2]
rel = Path("crates") / "ui" / "src" / "files.rs"
path = Path(src) / rel
if not path.is_file():
    raise SystemExit(f"subcraft.sh: {rel} not found under {src}")

text = path.read_text()
fixed = []

needle = """fn deliver(purpose: Purpose, paths: Vec<PathBuf>) {
    if !paths.is_empty() {
        PICKED.lock().expect("picked files").push((purpose, paths));
    }
}
"""
addition = needle + """
/// Hand the app files it did not pick itself: a project the system opened it with, a mesh shared
/// from another app, or a file dropped on the window. The Android shell calls this after staging the
/// document; it goes into the same queue [`pick`] fills, so [`poll`] handles it identically.
pub fn deliver_android(purpose: Purpose, paths: Vec<PathBuf>) {
    deliver(purpose, paths);
}
"""

if "pub fn deliver_android(" in text:
    pass
elif needle in text:
    text = text.replace(needle, addition, 1)
    fixed.append(str(rel))
else:
    raise SystemExit(
        f"subcraft.sh: the files.rs deliver() shape changed; update this patch. "
        f"Expected in {rel}: {needle!r}"
    )

if fixed:
    path.write_text(text)
    print(f"  Patched SubCraft's Android file hand-off for {app}: {', '.join(fixed)}")
elif os.environ.get("ARTCRAFT_SUBCRAFT_TRACE") == "1":
    print(f"  SubCraft's Android file hand-off already present in {rel}")

# Never ask wgpu for a texture larger than the device allows. wgpu treats an oversized
# `create_texture` as a fatal validation error, so a run whose window/viewport size arrives wrong for
# a frame would take the whole process down (the `viewport msaa` texture at 10292 px against an 8192
# limit). Scaling to the limit keeps the frame renderable; the real size is fixed at the source (see
# `ensure_screen_rect` in adapters/subcraft/src/lib.rs and the viewport clamp below).
target_rel = Path("crates") / "render" / "src" / "target.rs"
tpath = Path(src) / target_rel
if tpath.is_file():
    ttext = tpath.read_text()
    old_t = """impl RenderTarget {
    pub fn new(device: &wgpu::Device, size: [u32; 2], samples: u32) -> Self {
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let make = |label, format, samples, usage, view_formats: &[wgpu::TextureFormat]| {"""
    new_t = """impl RenderTarget {
    pub fn new(device: &wgpu::Device, size: [u32; 2], samples: u32) -> Self {
        // An oversized texture is a fatal wgpu validation error, and the viewport is a preview:
        // clamp to what the device can actually create rather than crash the whole app.
        let max_dim = device.limits().max_texture_dimension_2d;
        let size = [size[0].min(max_dim).max(1), size[1].min(max_dim).max(1)];
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let make = |label, format, samples, usage, view_formats: &[wgpu::TextureFormat]| {"""
    if "max_texture_dimension_2d" not in ttext and old_t in ttext:
        ttext = ttext.replace(old_t, new_t)
        tpath.write_text(ttext)
        print(f"  Patched SubCraft's render target size clamping")

# Do not size the 3D viewport's offscreen target from egui's placeholder viewport.
#
# On the first frame winit has not reported the window size yet, so egui falls back to its built-in
# 10000×10000-point placeholder. SubCraft applies its UI-size preference (a zoom factor) in that same
# frame, and egui rescales the placeholder by the zoom change (`10000 / 0.95 = 10526` points). The
# viewport then asks wgpu for a render target of ~12700 px: far past the device's
# `max_texture_dimension_2d` (8192), which wgpu treats as a fatal validation error — and even clamped,
# the MSAA and depth attachments for a target that size reserve gigabytes and the system's low-memory
# killer ends the process.
#
# A tab is never larger than the window that holds it, so the render target is clamped to the window's
# physical size, and while the window rect is still the placeholder the tab is skipped entirely. The
# next frame carries the real size (see `ensure_screen_rect` in adapters/subcraft/src/lib.rs) and
# renders normally.
vp_rel = Path("crates") / "ui" / "src" / "viewport.rs"
vpath = Path(src) / vp_rel
if vpath.is_file():
    vtext = vpath.read_text()
    old_v = """    let ppp = ui.ctx().pixels_per_point();
    let size = [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32];
    if size[0] < 8 || size[1] < 8 {
        return;
    }
    state.last_view_size = size;"""
    old_v_8192 = """    let ppp = ui.ctx().pixels_per_point();
    let size = [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32];
    if size[0] < 8 || size[1] < 8 || size[0] > 8192 || size[1] > 8192 {
        let clamped = [size[0].min(8192).max(1), size[1].min(8192).max(1)];
        state.last_view_size = clamped;
    } else {
        state.last_view_size = size;
    }"""
    old_v_window = """    let ppp = ui.ctx().pixels_per_point();
    let size = [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32];
    // The tab cannot be bigger than the window that holds it. Clamping to the window's physical size
    // keeps the offscreen target no larger than the surface (an oversized one is unusable anyway and
    // can exceed the GPU's texture limit).
    let window = ui.ctx().input(|i| i.viewport_rect());
    let max = [
        (window.width() * ppp).round().max(8.0) as u32,
        (window.height() * ppp).round().max(8.0) as u32,
    ];
    let size = [size[0].min(max[0]).max(1), size[1].min(max[1]).max(1)];
    if size[0] < 8 || size[1] < 8 {
        return;
    }
    state.last_view_size = size;"""
    new_v = """    // egui's placeholder viewport is 10000 points (rescaled by any zoom change on the first frame);
    // no real Android surface is anywhere near 6000 points, so this is the unset window.
    let window = ui.ctx().input(|i| i.viewport_rect());
    if window.width() >= 6000.0 || window.height() >= 6000.0 {
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let size = [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32];
    // The tab cannot be bigger than the window that holds it; clamping to the window's physical size
    // keeps the offscreen target no larger than the surface (a bigger one is unusable anyway).
    let max = [
        (window.width() * ppp).round().max(8.0) as u32,
        (window.height() * ppp).round().max(8.0) as u32,
    ];
    let size = [size[0].min(max[0]).max(1), size[1].min(max[1]).max(1)];
    if size[0] < 8 || size[1] < 8 {
        return;
    }
    state.last_view_size = size;"""
    if "window.width() >= 6000.0" not in vtext:
        for needle in (old_v, old_v_8192, old_v_window):
            if needle in vtext:
                vtext = vtext.replace(needle, new_v, 1)
                vpath.write_text(vtext)
                print(f"  Patched SubCraft's viewport size clamp")
                break
PY
