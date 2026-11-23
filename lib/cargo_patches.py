#!/usr/bin/env python3
"""Consolidated Cargo and Workspace patching for ArtCraft Android apps.

Handles vendored crate injection into [patch.crates-io], workspace dependency
validation, winit/egui-wgpu platform adjustments, and wgpu Cargo registry
hot-patching for Vulkan swapchain and Android hardware buffer support.

Extracted from lib/build-common.sh as part of modularization (issue #73).
"""

from pathlib import Path
import os
import re
import stat
import sys
import tomllib


def patch_crates_io(
    cargo_toml: Path,
    crate_name: str,
    path_rel: str,
    git_url: str | None = None,
) -> None:
    """Patch a crate path into [patch.crates-io] and optionally [patch.<git_url>]."""
    if path_rel == "auto" and crate_name == "egui-winit":
        path_rel = (
            "vendor/egui/crates/egui-winit"
            if (cargo_toml.parent / "vendor/egui/crates/egui-winit").is_dir()
            else "vendor/egui-winit"
        )
    text = cargo_toml.read_text()
    entry = f'{crate_name} = {{ path = "{path_rel}" }}'

    header = "[patch.crates-io]"
    if header in text:
        parts = text.split(header, 1)
        next_sec = re.search(r"\n\[", parts[1])
        patch_sec = parts[1][: next_sec.start()] if next_sec else parts[1]
        rest = parts[1][next_sec.start() :] if next_sec else ""
        patch_sec = re.sub(
            rf"^\s*{re.escape(crate_name)}\s*=\s*\{{.*?\}}\s*(?:#.*)?\n?",
            "",
            patch_sec,
            flags=re.M,
        )
        text = parts[0] + header + "\n" + entry + "\n" + patch_sec.lstrip("\n") + rest
    else:
        text = text.rstrip() + "\n\n" + header + "\n" + entry + "\n"

    if git_url and git_url in text:
        git_header = f'[patch."{git_url}"]'
        if git_header in text:
            section = text.split(git_header)[1].split("\n[")[0]
            if entry not in section:
                text = text.replace(git_header, git_header + "\n" + entry, 1)
        else:
            text = text.rstrip() + "\n\n" + git_header + "\n" + entry + "\n"

    tomllib.loads(text)
    cargo_toml.write_text(text)


def patch_arboard(root_toml: Path, src_root: Path) -> None:
    """Patch arboard dependency and sync lockfile version if needed."""
    s = root_toml.read_text()

    uses_workspace = False
    for cargo in src_root.rglob("Cargo.toml"):
        if "vendor/" in str(cargo):
            continue
        try:
            txt = cargo.read_text()
            if re.search(r"arboard\s*=\s*\{[^}]*workspace\s*=\s*true", txt):
                uses_workspace = True
                break
        except Exception:
            pass

    entry_path = 'arboard = { path = "vendor/arboard" }'

    def repl_arboard(m: re.Match) -> str:
        inner = m.group(0)
        features = re.search(r"features\s*=\s*\[([^\]]+)\]", inner)
        default_feats = re.search(r"default-features\s*=\s*(false|true)", inner)
        parts = ['path = "vendor/arboard"']
        if default_feats:
            parts.append(f"default-features = {default_feats.group(1)}")
        if features:
            parts.append(f"features = [{features.group(1)}]")
        joined = ", ".join(parts)
        return f"arboard = {{ {joined} }}"

    s = re.sub(r"^\s*arboard\s*=\s*\{[^}]*\}\s*(?:#.*)?$", repl_arboard, s, flags=re.M)
    s = re.sub(
        r'^\s*arboard\s*=\s*"[^"]*"\s*(?:#.*)?$',
        'arboard = { path = "vendor/arboard" }',
        s,
        flags=re.M,
    )

    header = "[patch.crates-io]"
    if header in s:
        parts = s.split(header, 1)
        next_sec = re.search(r"\n\[", parts[1])
        patch_sec = parts[1][: next_sec.start()] if next_sec else parts[1]
        rest = parts[1][next_sec.start() :] if next_sec else ""
        patch_sec = re.sub(r"^\s*arboard\s*=\s*\{.*?\}\s*(?:#.*)?\n?", "", patch_sec, flags=re.M)
        s = parts[0] + header + "\n" + entry_path + "\n" + patch_sec.lstrip("\n") + rest
    else:
        s = s.rstrip() + "\n\n" + header + "\n" + entry_path + "\n"

    tomllib.loads(s)
    root_toml.write_text(s)

    lock_path = src_root / "Cargo.lock"
    if lock_path.exists():
        try:
            lock_text = lock_path.read_text()
            m = re.search(
                r'\[\[package\]\]\s*name\s*=\s*"arboard"\s*version\s*=\s*"([^"]+)"',
                lock_text,
            )
            if m:
                lock_ver = m.group(1)
                vendored_toml = src_root / "vendor" / "arboard" / "Cargo.toml"
                if vendored_toml.exists():
                    vtext = vendored_toml.read_text()
                    vtext = re.sub(
                        r'(name\s*=\s*"arboard"\s*\nversion\s*=\s*)"[^"]+"',
                        rf'\g<1>"{lock_ver}"',
                        vtext,
                        count=1,
                    )
                    vendored_toml.write_text(vtext)
        except Exception as e:
            print(f"WARNING: failed to sync arboard version with lockfile: {e}", file=sys.stderr)


def patch_winit_ime(src: Path) -> None:
    """Sync winit IME-reset protocol with storytold fork egui-winit."""
    event_rs = src / "vendor" / "winit" / "src" / "event.rs"
    window_rs = src / "vendor" / "winit" / "src" / "window.rs"
    mod_rs = src / "vendor" / "winit" / "src" / "platform_impl" / "android" / "mod.rs"
    if not event_rs.is_file() or not window_rs.is_file() or not mod_rs.is_file():
        return

    changed = False

    ev = event_rs.read_text()
    if "ImeResetCompleted" not in ev:
        anchor = "    Ime(Ime),\n"
        if anchor in ev:
            ev = ev.replace(
                anchor,
                anchor
                + """\
    /// The ordered cancellation fence requested by [`Window::reset_ime_with_request`].
    ///
    /// ## Platform-specific
    ///
    /// - **Android:** No-op; Android drives its own input-method pipeline.
    ImeResetCompleted { request_id: u64 },
""",
            )
            event_rs.write_text(ev)
            changed = True

    wn = window_rs.read_text()
    if "fn reset_ime_with_request" not in wn:
        m = re.search(r"set_ime_purpose\(purpose\)\)\s*\n    \}\n", wn)
        if m:
            end = m.end()
            wn = (
                wn[:end]
                + """
    /// Requests an IME reset, identifying the request with `request_id`. No-op on Android.
    #[inline]
    pub fn reset_ime_with_request(&self, _request_id: u64) {}
"""
                + wn[end:]
            )
            window_rs.write_text(wn)
            changed = True

    text = mod_rs.read_text()
    text = re.sub(r"    pub fn reset_ime_with_request\(&self, _request_id: u64\) \{\}\n", "", text)
    text = text.replace(
        "    pub fn set_ime_purpose(&self, _purpose: ImePurpose) {}\n"
        "    pub fn set_ime_purpose(&self, _purpose: ImePurpose) {}\n",
        "    pub fn set_ime_purpose(&self, _purpose: ImePurpose) {}\n",
    )
    if "fn reset_ime_with_request" not in text:
        marker = "    pub fn set_ime_purpose(&self, _purpose: ImePurpose) {}\n"
        idx = text.find(marker)
        if idx >= 0:
            insert = """
    /// No-op on Android: the platform drives its own input-method pipeline and does not use the
    /// winit IME-reset protocol.
    pub fn reset_ime_with_request(&self, _request_id: u64) {}
"""
            text = text[: idx + len(marker)] + insert + text[idx + len(marker) :]
            mod_rs.write_text(text)
            changed = True

    for ew_path in (
        src / "vendor" / "egui" / "crates" / "egui-winit" / "src" / "lib.rs",
        src / "vendor" / "egui-winit" / "src" / "lib.rs",
    ):
        if ew_path.is_file():
            ew = ew_path.read_text()
            ew_changed = False
            if "ImeResetCompleted" not in ew:
                target1 = "WindowEvent::DoubleTapGesture { .. } => EventResponse {"
                repl1 = (
                    "WindowEvent::DoubleTapGesture { .. }\n"
                    "            | WindowEvent::ImeResetCompleted { .. } => EventResponse {"
                )
                if target1 in ew:
                    ew = ew.replace(target1, repl1, 1)
                    ew_changed = True
                target2 = 'WindowEvent::ActivationTokenDone { .. } => "WindowEvent::ActivationTokenDone",\n'
                repl2 = (
                    target2
                    + '        WindowEvent::ImeResetCompleted { .. } => "WindowEvent::ImeResetCompleted",\n'
                )
                if target2 in ew:
                    ew = ew.replace(target2, repl2, 1)
                    ew_changed = True
            if ew_changed:
                ew_path.write_text(ew)
                changed = True

    if changed:
        print("  Synced winit IME-reset support with fork egui-winit")


def patch_egui_dock(src: Path) -> None:
    """Point egui_dock in [workspace.dependencies] at vendored copy."""
    ws_toml = src / "Cargo.toml"
    if not ws_toml.is_file():
        return
    ws_text = ws_toml.read_text()
    new_ws, n = re.subn(
        r"^\s*egui_dock\s*=\s*\{[^}]*\}\s*(?:#.*)?\n?",
        'egui_dock = { path = "vendor/egui_dock", features = ["serde"] }\n',
        ws_text,
        flags=re.M,
    )
    if n > 0:
        ws_toml.write_text(new_ws)
        print("  Replaced egui_dock with vendored path in [workspace.dependencies]")
    elif 'egui_dock = { path = "vendor/egui_dock"' not in ws_text:
        marker = "[workspace.dependencies]\n"
        idx = ws_text.find(marker)
        if idx >= 0:
            ws_text = (
                ws_text[: idx + len(marker)]
                + 'egui_dock = { path = "vendor/egui_dock", features = ["serde"] }\n'
                + ws_text[idx + len(marker) :]
            )
            ws_toml.write_text(ws_text)
            print("  Added egui_dock to [workspace.dependencies]")


def patch_subcraft_accesskit(src: Path) -> None:
    """Repair relative path to accesskit_winit in subcraft fork egui-winit."""
    efw = src / "vendor" / "egui" / "crates" / "egui-winit" / "Cargo.toml"
    if not efw.is_file():
        return
    text = efw.read_text()
    new = re.sub(r'path = "[^"]*/winit"', 'path = "../../../winit"', text)
    new = new.replace(
        'accesskit_winit = { path = "../../../winit", optional = true }',
        "accesskit_winit = { workspace = true, optional = true }",
    )
    if new != text:
        efw.write_text(new)
        print("  Fixed subcraft fork egui-winit accesskit_winit path")


def patch_egui_wgpu_surface(wgpu_target: Path) -> None:
    """Fix egui-wgpu Android surface-loss recovery in winit.rs."""
    winit_rs = wgpu_target / "src" / "winit.rs"
    if not winit_rs.is_file():
        return
    text = winit_rs.read_text()
    if 'cfg!(target_os = "android")' not in text:
        target = """        let Some(old_state) = self.surfaces.remove(&viewport_id) else {
            return Ok(());
        };"""
        replacement = """        let Some(old_state) = self.surfaces.remove(&viewport_id) else {
            return Ok(());
        };

        if cfg!(target_os = "android") {
            log::info!("egui-wgpu: dropping the lost surface of {viewport_id:?} (Android)");
            return Ok(());
        }"""
        if target in text:
            text = text.replace(target, replacement, 1)
            winit_rs.write_text(text)
            print("  Patched existing vendored egui-wgpu recreate_surface for Android surface handling")
    else:
        buggy = """        let (requested, resizing, mut retry) = (old_state.requested, old_state.resizing, old_state.retry);

        if cfg!(target_os = "android") {
            log::info!("egui-wgpu: dropping the lost surface of {viewport_id:?} (Android)");
            return Ok(());
        }

        self.instance.poll_all(true);

        drop(old_state);"""
        clean = """        if cfg!(target_os = "android") {
            log::info!("egui-wgpu: dropping the lost surface of {viewport_id:?} (Android)");
            return Ok(());
        }"""
        if buggy in text:
            text = text.replace(buggy, clean)
        if "let mut retry = old_state.retry" not in text and "retry.recreated();" in text:
            text = text.replace(
                "retry.recreated();",
                "let mut retry = old_state.retry;\n        retry.recreated();",
                1,
            )
        if "requested.0" in text:
            text = text.replace(
                "requested.0,\n            requested.1,\n            resizing,\n            retry,",
                "old_state.requested.0,\n            old_state.requested.1,\n            old_state.resizing,\n            retry,",
            )
        winit_rs.write_text(text)
        print("  Cleaned up egui-wgpu recreate_surface patch for Android surface handling")


def ensure_workspace_dependencies(src: Path, app: str) -> None:
    """Ensure missing [workspace.dependencies] are populated for inherited crates."""
    for key, relpaths in [
        ("egui-wgpu", ["vendor/egui-wgpu", "vendor/egui/crates/egui-wgpu"]),
        ("egui-winit", ["vendor/egui-winit", "vendor/egui/crates/egui-winit"]),
        ("winit", ["vendor/winit"]),
        ("arboard", ["vendor/arboard"]),
    ]:
        try:
            text = (src / "Cargo.toml").read_text()
        except OSError:
            continue
        if "[workspace.dependencies]" not in text:
            continue
        section = text.split("[workspace.dependencies]", 1)[1].split("\n[", 1)[0]
        if re.search(rf"^\s*{re.escape(key)}\s*=", section, flags=re.M):
            continue
        relpath = next((p for p in relpaths if (src / p).is_dir()), None)
        if not relpath:
            continue
        marker = "[workspace.dependencies]\n"
        idx = text.find(marker)
        if idx >= 0:
            text = (
                text[: idx + len(marker)]
                + f'{key} = {{ path = "{relpath}" }}\n'
                + text[idx + len(marker) :]
            )
            (src / "Cargo.toml").write_text(text)
            print(f"  Added {key} to [workspace.dependencies]")


def strip_winit_wayland(src: Path) -> None:
    """Empty desktop wayland features from vendor/winit to avoid fontconfig conflicts."""
    winit_toml = src / "vendor" / "winit" / "Cargo.toml"
    if not winit_toml.is_file():
        return
    text = winit_toml.read_text()
    for name in ("wayland", "wayland-dlopen", "wayland-csd-adwaita"):
        text = re.sub(rf'(?m)^\s*"{re.escape(name)}",\s*\n', "", text)
    for name in (
        "wayland",
        "wayland-dlopen",
        "wayland-csd-adwaita",
        "wayland-csd-adwaita-crossfont",
        "wayland-csd-adwaita-notitle",
    ):
        text = re.sub(
            rf"(?ms)^(\s*{re.escape(name)}\s*=\s*\[\s*\n)(.*?)^(\s*\]\s*\n)",
            r"\1\3",
            text,
        )
    for name in ("wayland-dlopen", "wayland-csd-adwaita-notitle"):
        text = re.sub(
            rf"(?m)^(\s*{re.escape(name)}\s*=\s*\[)[^\]]*(\]\s*)$",
            r"\1\2",
            text,
        )
    winit_toml.write_text(text)
    print("  Emptied wayland features in vendor/winit/Cargo.toml (fontconfig links conflict)")


def patch_wgpu_registry(cargo_home: Path | None = None) -> None:
    """Hot-patch wgpu crates in Cargo registry for Adreno Vulkan swapchain support."""
    if cargo_home is None:
        cargo_home = Path(os.environ.get("CARGO_HOME", os.path.expanduser("~/.cargo")))
    search_dirs = [cargo_home / "registry" / "src", Path.home() / ".cargo" / "registry" / "src"]

    for base in search_dirs:
        if not base.exists():
            continue
        # 1. Patch wgpu-hal swapchain latency calculation
        for p in base.glob("**/wgpu-hal-*/src/vulkan/swapchain/native.rs"):
            try:
                p.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IRGRP | stat.S_IROTH)
                txt = p.read_text()
                old = "maximum_frame_latency: (caps.min_image_count - 1)..=(max_image_count - 1)"
                if old in txt:
                    new = (
                        "maximum_frame_latency: {\n"
                        "                let min_lat = caps.min_image_count.saturating_sub(1);\n"
                        "                let max_lat = if max_image_count == 0 { min_lat.max(2) } else { max_image_count.saturating_sub(1) };\n"
                        "                let (lo, hi) = if min_lat <= max_lat { (min_lat, max_lat) } else { (max_lat, min_lat) };\n"
                        "                lo..=hi\n"
                        "            }"
                    )
                    p.write_text(txt.replace(old, new))
                    print(f"  -> patched wgpu-hal swapchain latency in {p.name}")
            except Exception:
                pass

        # 2. Patch wgpu-core clamp panic guard
        for p in base.glob("**/wgpu-core-*/src/device/resource.rs"):
            try:
                p.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IRGRP | stat.S_IROTH)
                txt = p.read_text()
                old = (
                    "                let maximum_frame_latency = config.desired_maximum_frame_latency.clamp(\n"
                    "                    *caps.maximum_frame_latency.start(),\n"
                    "                    *caps.maximum_frame_latency.end(),\n"
                    "                );"
                )
                if old in txt:
                    new = (
                        "                let max_lat_lo = *caps.maximum_frame_latency.start();\n"
                        "                let max_lat_hi = (*caps.maximum_frame_latency.end()).max(max_lat_lo);\n"
                        "                let maximum_frame_latency = config.desired_maximum_frame_latency.clamp(\n"
                        "                    max_lat_lo,\n"
                        "                    max_lat_hi,\n"
                        "                );"
                    )
                    p.write_text(txt.replace(old, new))
                    print(f"  -> patched wgpu-core clamp guard in {p.name}")
            except Exception:
                pass

        # 3. Patch wgpu-hal to enable VK_ANDROID_external_memory_android_hardware_buffer
        for p in base.glob("**/wgpu-hal-*/src/vulkan/adapter.rs"):
            try:
                p.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IRGRP | stat.S_IROTH)
                txt = p.read_text()
                old = "if self.supports_extension(ext::external_memory_dma_buf::NAME) {"
                if old in txt and "VK_ANDROID_external_memory_android_hardware_buffer" not in txt:
                    new = (
                        'const ANDROID_HWB_EXT: &std::ffi::CStr = c"VK_ANDROID_external_memory_android_hardware_buffer";\n'
                        "        if self.supports_extension(ANDROID_HWB_EXT) {\n"
                        "            extensions.push(ANDROID_HWB_EXT);\n"
                        "        }\n"
                        "        if self.supports_extension(ext::external_memory_dma_buf::NAME) {"
                    )
                    p.write_text(txt.replace(old, new, 1))
                    print(f"  -> patched wgpu-hal android_hardware_buffer extension in {p.name}")
            except Exception:
                pass


def main(argv: list[str] | None = None) -> None:
    if argv is None:
        argv = sys.argv[1:]

    if not argv:
        print(
            "Usage: cargo_patches.py <command> [args...]\n\n"
            "Commands:\n"
            "  patch-crates-io <cargo_toml> <crate_name> <path_rel> [git_url]\n"
            "  patch-arboard <cargo_toml> <src_root>\n"
            "  patch-winit-ime <src_root>\n"
            "  patch-egui-dock <src_root>\n"
            "  patch-subcraft-accesskit <src_root>\n"
            "  patch-egui-wgpu-surface <wgpu_target_dir>\n"
            "  ensure-workspace-deps <src_root> <app>\n"
            "  strip-winit-wayland <src_root>\n"
            "  patch-wgpu-registry [cargo_home]\n",
            file=sys.stderr,
        )
        sys.exit(1)

    cmd = argv[0]
    if cmd == "patch-crates-io":
        if len(argv) < 4:
            raise SystemExit("Usage: patch-crates-io <cargo_toml> <crate_name> <path_rel> [git_url]")
        patch_crates_io(
            Path(argv[1]),
            argv[2],
            argv[3],
            argv[4] if len(argv) > 4 else None,
        )
    elif cmd == "patch-arboard":
        if len(argv) < 3:
            raise SystemExit("Usage: patch-arboard <cargo_toml> <src_root>")
        patch_arboard(Path(argv[1]), Path(argv[2]))
    elif cmd == "patch-winit-ime":
        if len(argv) < 2:
            raise SystemExit("Usage: patch-winit-ime <src_root>")
        patch_winit_ime(Path(argv[1]))
    elif cmd == "patch-egui-dock":
        if len(argv) < 2:
            raise SystemExit("Usage: patch-egui-dock <src_root>")
        patch_egui_dock(Path(argv[1]))
    elif cmd == "patch-subcraft-accesskit":
        if len(argv) < 2:
            raise SystemExit("Usage: patch-subcraft-accesskit <src_root>")
        patch_subcraft_accesskit(Path(argv[1]))
    elif cmd == "patch-egui-wgpu-surface":
        if len(argv) < 2:
            raise SystemExit("Usage: patch-egui-wgpu-surface <wgpu_target_dir>")
        patch_egui_wgpu_surface(Path(argv[1]))
    elif cmd == "ensure-workspace-deps":
        if len(argv) < 3:
            raise SystemExit("Usage: ensure-workspace-deps <src_root> <app>")
        ensure_workspace_dependencies(Path(argv[1]), argv[2])
    elif cmd == "strip-winit-wayland":
        if len(argv) < 2:
            raise SystemExit("Usage: strip-winit-wayland <src_root>")
        strip_winit_wayland(Path(argv[1]))
    elif cmd == "patch-wgpu-registry":
        patch_wgpu_registry(Path(argv[1]) if len(argv) > 1 else None)
    else:
        raise SystemExit(f"Unknown command: {cmd}")


if __name__ == "__main__":
    main()
