#!/usr/bin/env python3
"""Generate NativeActivity launcher wrappers for ArtCraft apps.

Synthesizes Android NativeActivity launcher crates, Cargo manifests,
JNI bindings, SAF bridges, and Java Activities for ArtCraft applications.
Extracted from lib/build-common.sh as part of modularization (issue #73).
"""

from pathlib import Path
import re
import shutil
import sys
import tomllib

if len(sys.argv) < 10:
    print(
        "Usage: launcher_generator.py <root> <src> <app> <title> <target_sdk> <appname> <packagename> <appicon> <no_buttons>",
        file=sys.stderr,
    )
    sys.exit(1)


root, src_s, app, title, target_sdk, appname, packagename, appicon, no_buttons_str = sys.argv[1:10]
root, src = Path(root), Path(src_s)
if not src.is_absolute():
    src = root / src
no_buttons = no_buttons_str.lower() in ("true", "1", "yes")
text_bar_file = "mobile_text_bar_disabled.rs" if no_buttons else "mobile_text_bar.rs"
# Shared file-association table (Android open/share intent filters), also used by the adapter
# prepare scripts so every launcher advertises the same associations.
sys.path.insert(0, str(root / "lib"))
import file_associations
android_dir = src / "apps" / f"{app}-android"
marker = android_dir / ".artcraft-android-builder-generated"

# Bump this whenever the generated launcher template changes so stale wrappers (which live in the
# source checkout and would otherwise be preserved forever) are regenerated automatically instead
# of failing against a newer app API.
GENERATOR_VERSION = "83"

# A complete adapter supplied by the user wins over auto-detection. This folder is maintained
# inside the builder, while its contents are copied into this source checkout.
custom = root / "adapters" / app
if (custom / "Cargo.toml").is_file() and (custom / "src" / "lib.rs").is_file():
    if android_dir.exists() and not marker.exists():
        raise SystemExit(f"Refusing to overwrite non-builder directory: {android_dir}")
    if android_dir.exists():
        shutil.rmtree(android_dir)
    shutil.copytree(custom, android_dir, ignore=shutil.ignore_patterns("README.md", ".git"))
    # Ship the shared Android system-font module alongside the adapter so its launcher can pull the
    # device's own fonts in (see templates/android/system_fonts.rs). Kept here rather than inside
    # each adapter so the one copy stays in step with the generator.
    shutil.copy2(root / "templates" / "android" / "system_fonts.rs", android_dir / "src" / "system_fonts.rs")
    if not (custom / "src" / "mcp_http.rs").is_file():
        shutil.copy2(root / "templates" / "android" / "mcp_http.rs", android_dir / "src" / "mcp_http.rs")
    shutil.copy2(root / "templates" / "android" / text_bar_file, android_dir / "src" / "mobile_text_bar.rs")
    shutil.copy2(root / "templates" / "android" / "touch_gestures.rs", android_dir / "src" / "touch_gestures.rs")
    if app in ("subcraft", "tracedraw", "picassosuite", "bimcraft", "earthcraft", "mapcraft", "transit", "anicraft", "papercraft", "ferrite", "pixelcraft", "clothcraft"):
        tg = android_dir / "src" / "touch_gestures.rs"
        if tg.is_file():
            tg.write_text(re.sub(r'(?m)^\s*source:\s*egui::MouseWheelSource::.*?\n', '', tg.read_text()))
    # The custom IME bridge is removed to allow egui native IME. Do not copy ime_bridge.rs
    # The adapter's launcher gets the same icons as the generated ones: density mipmaps plus the
    # API 26+ adaptive layers (see lib/app_icons.py). An adapter that ships its own `assets/app-icon`
    # seeds the app tree with it when the app has no icon of its own (SubCraft ships only a Windows
    # .ico), so the artwork comes from the adapter instead of another app's branding. Icon generation
    # is best-effort: an app with no usable artwork keeps the generic Android icon rather than
    # failing the whole build (like the PhotoCraft adapter's prepare.sh).
    import app_icons

    if (custom / "assets" / "app-icon").is_dir() and not (src / "assets" / "app-icon").is_dir():
        shutil.copytree(custom / "assets" / "app-icon", src / "assets" / "app-icon")
    try:
        app_icons.generate(src, app, android_dir / "res", Path(appicon) if appicon else None)
    except SystemExit as error:
        print(f"No icon could be generated for {app} ({error}); using a generic Android icon.")
        manifest = android_dir / "Cargo.toml"
        manifest.write_text("\n".join(
            line for line in manifest.read_text().splitlines() if not line.startswith("icon = ")
        ) + "\n")
    # Advertise the app's file associations (Android open/share intent filters) in the copied
    # manifest, from the same table the generated launchers use.
    file_associations.patch_cargo_toml(android_dir / "Cargo.toml", app)
    marker.write_text(f"{GENERATOR_VERSION}\nGenerated/copied by artcraft-android-builder; custom adapter.\n")
    print(f"Installed custom adapter from {custom}")
    raise SystemExit(0)

if android_dir.exists():
    if marker.is_file():
        if marker.read_text(errors="replace").splitlines()[:1] == [GENERATOR_VERSION]:
            print("Android launcher already generated; preserving it.")
            # The shared modules are refreshed even when the launcher itself is kept: they are
            # copied into it rather than generated, so a change to one of them (the system-font
            # bridge) has to reach the launcher that is already there. IME is handled natively by
            # Android's input method framework via NativeActivity — egui's TextEdit widgets
            # request the soft keyboard automatically, no bridge needed.
            for shared in ("system_fonts.rs", "mobile_text_bar.rs", "touch_gestures.rs"):
                real_shared = text_bar_file if shared == "mobile_text_bar.rs" else shared
                src_shared = root / "templates" / "android" / real_shared
                dst_shared = android_dir / "src" / shared
                if src_shared.is_file() and dst_shared.parent.is_dir():
                    shutil.copy2(src_shared, dst_shared)
            if app in ("subcraft", "tracedraw", "picassosuite", "bimcraft", "earthcraft", "mapcraft", "transit", "anicraft", "papercraft", "ferrite", "pixelcraft", "clothcraft"):
                tg = android_dir / "src" / "touch_gestures.rs"
                if tg.is_file():
                    tg.write_text(re.sub(r'(?m)^\s*source:\s*egui::MouseWheelSource::.*?\n', '', tg.read_text()))
            raise SystemExit(0)
        print("Regenerating the Android launcher for this builder version.")
        shutil.rmtree(android_dir)
    else:
        raise SystemExit(f"Refusing to overwrite {android_dir}: it exists but is not marked as builder-generated. If this is your own adapter, place a complete adapter in adapters/{app}/ and move the directory manually first.")

root_manifest = src / "Cargo.toml"
if not root_manifest.is_file():
    raise SystemExit(f"No workspace Cargo.toml found in {src}")
try:
    ws = tomllib.loads(root_manifest.read_text())
except Exception as e:
    raise SystemExit(f"Cannot parse workspace Cargo.toml: {e}")
if "workspace" not in ws:
    raise SystemExit("Source root is not a Cargo workspace; a custom adapter is required.")
members = ws["workspace"].get("members", [])
if not any("apps/*" in m or m in (f"apps/{app}-android", f"apps/{app}-android/") for m in members):
    if (src / "apps").is_dir():
        manifest_text = root_manifest.read_text()
        m = re.search(r'(?ms)(members\s*=\s*\[)(.*?)(\])', manifest_text)
        if m:
            prefix, current, suffix = m.group(1), m.group(2), m.group(3)
            new_member = f"apps/{app}-android"
            if f'"{new_member}"' not in current and f"'{new_member}'" not in current:
                if '\n' in current:
                    new_current = current.rstrip() + f'\n    "{new_member}",\n'
                    manifest_text = manifest_text[:m.start()] + prefix + new_current + suffix + manifest_text[m.end():]
                else:
                    new_current = current.strip()
                    if new_current and not new_current.endswith(','):
                        new_current += ', '
                    elif new_current:
                        new_current += ' '
                    new_current += f'"{new_member}"'
                    manifest_text = manifest_text[:m.start()] + prefix + ' ' + new_current + ' ' + suffix + manifest_text[m.end():]
                root_manifest.write_text(manifest_text)
                ws = tomllib.loads(manifest_text)
                members = ws["workspace"].get("members", [])
                print(f"Added {new_member} to workspace members in Cargo.toml")
    if not any("apps/*" in m or m in (f"apps/{app}-android", f"apps/{app}-android/") for m in members):
        raise SystemExit("Workspace does not include apps/*; auto-add failed. Create a custom adapter or update the workspace members explicitly.")

engine_manifest = src / "crates" / "engine" / "Cargo.toml"
ui_manifest = src / "crates" / "ui-egui" / "Cargo.toml"
ui_src = src / "crates" / "ui-egui" / "src"
for p in (engine_manifest, ui_manifest, ui_src / "lib.rs"):
    if not p.exists():
        # The shared layout is what every app of the suite is expected to have. A checkout that
        # is missing it is far more often an interrupted clone than an app with a different
        # layout, so say which of the two it is instead of pointing at the adapter.
        crates = src / "crates"
        present = sorted(d.name for d in crates.iterdir() if d.is_dir()) if crates.is_dir() else []
        if len(present) < 5:
            raise SystemExit(
                f"The source checkout is incomplete: crates/ holds {len(present)} directories "
                f"({', '.join(present) if present else 'none'}) instead of the full workspace. "
                f"Move {src} aside and run the build again to re-clone it."
            )
        raise SystemExit(f"Expected shared engine/UI layout is missing: {p.relative_to(src)}. This app needs an app-specific adapter under adapters/{app}/.")

def pkg_name(path: Path) -> str:
    data = tomllib.loads(path.read_text())
    try:
        return data["package"]["name"]
    except KeyError as e:
        raise SystemExit(f"Manifest has no package.name: {path}") from e

engine_pkg = pkg_name(engine_manifest)
ui_pkg = pkg_name(ui_manifest)
ui_root = (ui_src / "lib.rs").read_text(errors="replace")
ui_text = "\n".join(p.read_text(errors="replace") for p in ui_src.rglob("*.rs"))
candidates = set()
for p in ui_src.rglob("*.rs"):
    if p.name.endswith(".rs"):
        text = p.read_text(errors="replace")
        candidates.update(re.findall(r"\bpub\s+(?:struct|enum)\s+([A-Za-z_][A-Za-z0-9]*App)\b", text))

# Most apps expose the conventional shape: a public `<Stem>App` built with
# `<App>::new(Session::new(), Services::default())` that implements `eframe::App` directly. A few
# differ in the constructor, in which crate defines the pieces, or in whether the type is the
# eframe app at all (their desktop `main.rs` wraps an inner type exposing `logic`/`ui`). Record those
# differences here so one launcher template still launches every app in the suite.
SPECIAL = {
    "filmcraft": {"app_type": "FilmcraftApp", "construct": "{app}::new(Session::default())"},
    "pdfcraft": {"construct": "{app}::new()"},
    "effectcraft": {"app_type": "EffectcraftApp", "construct": "{app}::new(effectcraft_host::session())"},
    "lightcraft": {"app_type": "LightcraftApp", "construct": "{app}::new(Session::new().with_fs(), Services::default())"},
    "figcraft": {"app_type": "FigApp", "construct": "{app}::new(Session::new(), Box::new(NoServices))"},
    "wordcraft": {
        "app_type": "WordApp",
        # Android starts at Word's File › Home start screen (Blank document, the templates and
        # Recent) instead of a blank document or the "The Open Studio Handbook" sample, which made
        # the app look like it had opened someone else's file. `WordApp::new` never shows the start
        # screen itself, so we call `start_screen()` after building the app (see `create_hook`).
        "construct": "{app}::new(Session::new({engine}::blank_document(&{engine}::Region::default())), Services::default())",
    },
    "soundcraft": {"app_type": "SoundApp", "construct": "{app}::new({engine}::Engine::default(), None, Services::default())"},
    "cadcraft": {"app_type": "CadApp", "construct": "{app}::new(Session::empty(), Services::default())"},
    "solidcraft": {"app_type": "SolveApp", "construct": "{app}::new(Session::default(), Services::default())"},
    "materialcraft": {"app_type": "MaterialCraftApp", "construct": "{app}::headless({engine}::sample_session(), {ui}::UiState::default())"},
    "texturecraft": {
        "app_type": "TextureCraftApp",
        "construct": "{app}::new(cc, {engine}::sample_session(1024, 256).expect(\"sample session\"))",
    },
}
spec = SPECIAL.get(app, {})

# Apps that can use the Android Storage Access Framework (SAF) through the builder-generated
# NativeActivity subclass + JNI bridge (see templates/android/). Each entry supplies the app's
# picker glue. Apps written for asynchronous pickers drive them from `saf_poll`; apps whose file
# API is synchronous use the blocking helpers below, which is safe because `android_main` runs on
# its own Rust thread, not the Java UI thread that shows the system picker. Apps with a full custom
# adapter (adapters/<app>/) take a different path and never reach here. The upstream app crates are
# never modified.
SYNC_SAF_IMPORTS = "use std::sync::atomic::{AtomicU64, Ordering};\nuse std::sync::Arc;"
SYNC_SAF_STATE = '''
pub type SafInbox = ();

#[derive(Default)]
pub struct SafState {
    next_id: AtomicU64,
    mirror: saf::SaveMirror,
}

pub fn saf_new_state() -> Arc<SafState> {
    Arc::new(SafState::default())
}

fn saf_next_id(state: &SafState) -> u64 {
    state.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

/// Open the Android picker and block until the user answers. The picker runs on the Java UI thread
/// while `android_main` runs on its own thread, so waiting here does not freeze the UI. Empty on
/// cancel or error; `pick_blocking` has its own timeout so a stuck picker cannot hang the app.
fn saf_open_paths(bridge: &saf::Bridge, state: &SafState) -> Vec<String> {
    let id = saf_next_id(state);
    match bridge.pick_blocking(id, None, saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => {
            for path in &paths {
                state.mirror.track(id, path);
            }
            paths
        }
        Ok(saf::PickAnswer::Cancelled) => Vec::new(),
        Err(error) => {
            log::error!("Android open picker: {error}");
            Vec::new()
        }
    }
}

/// Save dialog: the staged app-private destination, watched so the app's own write is mirrored to
/// the chosen SAF document on the next frames.
fn saf_save_path(bridge: &saf::Bridge, state: &SafState, name: &str) -> Option<String> {
    let id = saf_next_id(state);
    match bridge.pick_blocking(id, Some(name), saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => {
            let path = paths.into_iter().next();
            if let Some(path) = &path {
                state.mirror.track(id, path);
            }
            path
        }
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android save picker: {error}");
            None
        }
    }
}

/// Folder dialog: a real directory path on the device's main storage (SAF folders elsewhere cannot
/// be mapped to one), or `None` when cancelled.
fn saf_folder_path(bridge: &saf::Bridge, state: &SafState) -> Option<String> {
    let id = saf_next_id(state);
    match bridge.pick_folder_blocking(id, saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => paths.into_iter().next(),
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android folder picker: {error}");
            None
        }
    }
}
'''
SYNC_SAF_POLL = '''
fn saf_poll(bridge: &saf::Bridge, state: &SafState, _inbox: &SafInbox) {
    state.mirror.sync(bridge);
}
'''
RFD_SAF_POLL = '''
fn saf_poll(bridge: &saf::Bridge, state: &SafState, _inbox: &SafInbox) {
    for _ in 0..16 {
        let raw = match bridge.poll() {
            Ok(Some(raw)) => raw,
            Ok(None) => break,
            Err(error) => {
                log::error!("Could not poll the Android file picker: {error}");
                break;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                log::error!("Ignoring a malformed Android picker result ({error}): {raw}");
                continue;
            }
        };
        let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) else { continue };
        let kind = value.get("kind").and_then(serde_json::Value::as_str).unwrap_or_default();
        let status = value.get("status").and_then(serde_json::Value::as_str).unwrap_or("error");
        let paths: Option<Vec<std::path::PathBuf>> = if status == "ok" {
            Some(saf::picker_paths(&value).into_iter().map(std::path::PathBuf::from).collect())
        } else {
            None
        };
        // The staged destination is written by the app itself; watch it so any writes are
        // mirrored to the chosen SAF document.
        if let Some(ref paths) = paths {
            for path in paths {
                state.mirror.track(id, &path.to_string_lossy());
            }
        }
        rfd::artcraft_saf::deliver(id, paths);
    }
    state.mirror.sync(bridge);
}
'''
RFD_SAF_SETUP = '''
    let launcher_bridge = saf_bridge.clone_ref();
    rfd::artcraft_saf::install(std::sync::Arc::new(move |id, kind| {
        match kind {
            rfd::artcraft_saf::Kind::Open { multiple } => { launcher_bridge.request_open(id, multiple).ok(); }
            rfd::artcraft_saf::Kind::Save(ref name) => { launcher_bridge.request_save(id, name).ok(); }
            rfd::artcraft_saf::Kind::Folder => { launcher_bridge.request_folder(id).ok(); }
        }
    }));
    let blocking_bridge = saf_bridge.clone_ref();
    let blocking_state = std::sync::Arc::clone(&saf_state);
    rfd::artcraft_saf::install_blocking(std::sync::Arc::new(move |kind| {
        let id = saf_next_id(&blocking_state);
        let answer = match kind {
            rfd::artcraft_saf::Kind::Open { multiple } => {
                let answer = blocking_bridge.pick_open_blocking(id, multiple, saf::PICK_TIMEOUT);
                if let Ok(saf::PickAnswer::Paths(ref paths)) = answer {
                    for path in paths {
                        blocking_state.mirror.track(id, path);
                    }
                }
                answer
            }
            rfd::artcraft_saf::Kind::Save(ref name) => {
                let answer = blocking_bridge.pick_blocking(id, Some(name), saf::PICK_TIMEOUT);
                if let Ok(saf::PickAnswer::Paths(ref paths)) = answer {
                    if let Some(path) = paths.first() {
                        blocking_state.mirror.track(id, path);
                    }
                }
                answer
            }
            rfd::artcraft_saf::Kind::Folder => blocking_bridge.pick_folder_blocking(id, saf::PICK_TIMEOUT),
        };
        match answer {
            Ok(saf::PickAnswer::Paths(paths)) => Some(paths.into_iter().map(std::path::PathBuf::from).collect()),
            Ok(saf::PickAnswer::Cancelled) => None,
            Err(error) => {
                log::error!("File picker failed: {error}");
                None
            }
        }
    }));
'''
SAF_RECIPES = {
    "designcraft": {
        "activity": "DesignCraftActivity",
        "imports": "use designcraft_ui_egui::{ImportRequest, ImportedFile};\nuse std::collections::HashMap;\nuse std::sync::atomic::{AtomicU64, Ordering};\nuse std::sync::{Arc, Mutex};",
        "state": '''
pub type SafInbox = designcraft_ui_egui::Inbox;

#[derive(Default)]
pub struct SafState {
    next_id: AtomicU64,
    requests: Mutex<HashMap<u64, ImportRequest>>,
    saves: Mutex<HashMap<u64, Vec<u8>>>,
    mirror: saf::SaveMirror,
}

/// Pick a file (or a folder-free `open`) and block until the user answers. Safe because the SAF
/// bridge makes a frame-thread wait non-blocking for the JVM callbacks (`NonblockingCallbacks`),
/// and the pick has its own timeout. `None` on cancel or error. The path is the staged app-private
/// copy DesignCraft reads.
fn saf_pick_open(bridge: &saf::Bridge, state: &SafState) -> Option<String> {
    let id = saf_next_id(state);
    match bridge.pick_blocking(id, None, saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => {
            let path = paths.into_iter().next();
            if let Some(path) = &path {
                state.mirror.track(id, path);
            }
            path
        }
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android open picker: {error}");
            None
        }
    }
}

/// Save dialog: the staged app-private destination, watched so the app's own write is mirrored to
/// the chosen SAF document on the next frames (DesignCraft's file API is path-based). `None` on
/// cancel.
fn saf_pick_save(bridge: &saf::Bridge, state: &SafState, name: &str) -> Option<String> {
    let id = saf_next_id(state);
    match bridge.pick_blocking(id, Some(name), saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => {
            let path = paths.into_iter().next();
            if let Some(path) = &path {
                state.mirror.track(id, path);
            }
            path
        }
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android save picker: {error}");
            None
        }
    }
}

pub fn saf_new_state() -> Arc<SafState> {
    Arc::new(SafState::default())
}

fn saf_next_id(state: &SafState) -> u64 {
    state.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

/// Write to the staged private path, then push it through SAF when it was a chosen save
/// destination (a plain private path is a no-op commit).
fn saf_write_and_commit(bridge: &saf::Bridge, path: &str, bytes: &[u8]) -> Result<(), String> {
    let target = std::path::Path::new(path);
    if let Some(parent) = target.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(target, bytes).map_err(|error| format!("Could not write {}: {error}", target.display()))?;
    match bridge.commit(path)? {
        status if status == "NONE" || status == "OK" => Ok(()),
        status if status.starts_with("ERROR:") => Err(status.trim_start_matches("ERROR:").trim().to_owned()),
        status => Err(format!("Unexpected Android save result: {status}")),
    }
}
''',
        "services": '''
fn saf_services(bridge: saf::Bridge, state: Arc<SafState>, inbox: SafInbox) -> Services {
    let open_bridge = bridge.clone_ref();
    let save_bridge = bridge.clone_ref();
    let write_bridge = bridge.clone_ref();
    let pick_open_bridge = bridge.clone_ref();
    let pick_save_bridge = bridge.clone_ref();
    let open_state = Arc::clone(&state);
    let save_state = Arc::clone(&state);
    let pick_open_state = Arc::clone(&state);
    let pick_save_state = Arc::clone(&state);
    Services {
        read: Some(Box::new(|path: &str| std::fs::read(path).map_err(|error| format!("Could not read {path}: {error}")))),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| saf_write_and_commit(&write_bridge, path, bytes))),
        open_async: Some(Box::new(move |request: ImportRequest| {
            let id = saf_next_id(&open_state);
            open_state.requests.lock().unwrap_or_else(|error| error.into_inner()).insert(id, request);
            if let Err(error) = open_bridge.request_open(id, false) {
                log::error!("Could not start the Android open picker: {error}");
                open_state.requests.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
        })),
        // DesignCraft's `DownloadFn` returns a `Result`, so report a picker that will not start
        // instead of dropping the error (older sources returned `()` and only logged it).
        download: Some(Box::new(move |name: &str, bytes: &[u8]| {
            let id = saf_next_id(&save_state);
            save_state.saves.lock().unwrap_or_else(|error| error.into_inner()).insert(id, bytes.to_vec());
            if let Err(error) = save_bridge.request_save(id, name) {
                log::error!("Could not start the Android save picker: {error}");
                save_state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                return Err(error);
            }
            Ok(())
        })),
        inbox: Some(inbox),
        ..Services::default()
    }
}
''',
        "poll": '''
fn saf_poll(bridge: &saf::Bridge, state: &SafState, inbox: &SafInbox) {
    for _ in 0..16 {
        let raw = match bridge.poll() {
            Ok(Some(raw)) => raw,
            Ok(None) => break,
            Err(error) => {
                log::error!("Could not poll the Android file picker: {error}");
                break;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                log::error!("Ignoring a malformed Android picker result ({error}): {raw}");
                continue;
            }
        };
        let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) else { continue };
        let kind = value.get("kind").and_then(serde_json::Value::as_str).unwrap_or_default();
        let status = value.get("status").and_then(serde_json::Value::as_str).unwrap_or("error");
        match (kind, status) {
            ("open", "ok") => {
                let Some(request) = state.requests.lock().unwrap_or_else(|error| error.into_inner()).remove(&id) else { continue };
                let paths: Vec<String> = value
                    .get("paths")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry.as_str().map(str::to_owned))
                    .collect();
                for path in paths {
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            let name = std::path::Path::new(&path)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "imported-file".to_owned());
                            inbox.lock().unwrap_or_else(|error| error.into_inner()).push(ImportedFile { request: request.clone(), name, bytes });
                        }
                        Err(error) => log::error!("Could not read the picked file {path}: {error}"),
                    }
                }
            }
            ("save", "ok") => {
                let bytes = state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                if let Some(bytes) = bytes {
                    match value.get("path").and_then(serde_json::Value::as_str) {
                        Some(path) => {
                            if let Err(error) = saf_write_and_commit(bridge, path, &bytes) {
                                log::error!("Could not write the saved file: {error}");
                            }
                        }
                        None => log::error!("Android save picker returned no destination path"),
                    }
                }
            }
            ("open", "cancel") | ("save", "cancel") => {
                state.requests.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
            _ => {
                if let Some(message) = value.get("message").and_then(serde_json::Value::as_str) {
                    log::error!("Android file picker error: {message}");
                }
                state.requests.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
        }
    }
    // A save through `pick_save` is written by the app itself to the staged path; push it to the
    // chosen SAF document once the write lands.
    state.mirror.sync(bridge);
}
''',
    },
    "deckcraft": {
        "activity": "DeckCraftActivity",
        "imports": "mod audio;\nuse deckcraft_ui_egui::{InboxEvent, InboxFile};\nuse std::collections::HashMap;\nuse std::sync::atomic::{AtomicU64, Ordering};\nuse std::sync::{Arc, Mutex};",
        "state": '''
pub type SafInbox = deckcraft_ui_egui::Inbox;

#[derive(Default)]
pub struct SafState {
    next_id: AtomicU64,
    saves: Mutex<HashMap<u64, Vec<u8>>>,
    mirror: saf::SaveMirror,
}

pub fn saf_new_state() -> Arc<SafState> {
    Arc::new(SafState::default())
}

fn saf_next_id(state: &SafState) -> u64 {
    state.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

/// Write to the staged private path, then push it through SAF when it was a chosen save
/// destination (a plain private path is a no-op commit).
fn saf_write_and_commit(bridge: &saf::Bridge, path: &str, bytes: &[u8]) -> Result<(), String> {
    let target = std::path::Path::new(path);
    if let Some(parent) = target.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(target, bytes).map_err(|error| format!("Could not write {}: {error}", target.display()))?;
    match bridge.commit(path)? {
        status if status == "NONE" || status == "OK" => Ok(()),
        status if status.starts_with("ERROR:") => Err(status.trim_start_matches("ERROR:").trim().to_owned()),
        status => Err(format!("Unexpected Android save result: {status}")),
    }
}

/// Pick a file to open and block until the user answers. Safe because the SAF bridge makes a
/// frame-thread wait non-blocking for the JVM callbacks (`NonblockingCallbacks`), and the pick has
/// its own timeout. `None` on cancel or error; the path is the staged app-private copy.
fn saf_pick_open(bridge: &saf::Bridge, state: &SafState) -> Option<String> {
    let id = saf_next_id(state);
    match bridge.pick_blocking(id, None, saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => paths.into_iter().next(),
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android open picker: {error}");
            None
        }
    }
}

/// Save dialog: the staged app-private destination, watched so the app's own write is mirrored to
/// the chosen SAF document on the next frames (DeckCraft's file API is path-based). `None` on
/// cancel.
fn saf_pick_save(bridge: &saf::Bridge, state: &SafState, name: &str) -> Option<String> {
    let id = saf_next_id(state);
    match bridge.pick_blocking(id, Some(name), saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => {
            let path = paths.into_iter().next();
            if let Some(path) = &path {
                state.mirror.track(id, path);
            }
            path
        }
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android save picker: {error}");
            None
        }
    }
}
''',
        "services": '''
fn saf_services(bridge: saf::Bridge, state: Arc<SafState>, inbox: SafInbox) -> Services {
    let open_bridge = bridge.clone_ref();
    let save_bridge = bridge.clone_ref();
    let write_bridge = bridge.clone_ref();
    let pick_open_bridge = bridge.clone_ref();
    let pick_save_bridge = bridge.clone_ref();
    let open_inbox = inbox.clone();
    let save_state = Arc::clone(&state);
    let pick_open_state = Arc::clone(&state);
    let pick_save_state = Arc::clone(&state);
    Services {
        read: Some(Box::new(|path: &str| std::fs::read(path).map_err(|error| format!("Could not read {path}: {error}")))),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| saf_write_and_commit(&write_bridge, path, bytes))),
        // Save As / Export use the synchronous `pick_save`, which Android never provided, so those
        // menu items were a silent no-op. The picker blocks on the frame thread, which the SAF
        // bridge makes safe (`NonblockingCallbacks`), exactly as EffectCraft does.
        pick_open: Some(Box::new(move |_purpose: &str| saf_pick_open(&pick_open_bridge, &pick_open_state))),
        pick_save: Some(Box::new(move |name: &str| saf_pick_save(&pick_save_bridge, &pick_save_state, name))),
        open_async: Some(Box::new(move |_purpose: &str, id: u64| {
            // DeckCraft hands us the request id it already tracks; echo it back on completion.
            if let Err(error) = open_bridge.request_open(id, false) {
                log::error!("Could not start the Android open picker: {error}");
                open_inbox.lock().unwrap_or_else(|error| error.into_inner()).push(InboxEvent::Cancelled { id });
            }
        })),
        download: Some(Box::new(move |name: &str, bytes: &[u8]| {
            let id = saf_next_id(&save_state);
            save_state.saves.lock().unwrap_or_else(|error| error.into_inner()).insert(id, bytes.to_vec());
            if let Err(error) = save_bridge.request_save(id, name) {
                log::error!("Could not start the Android save picker: {error}");
                save_state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
        })),
        inbox: Some(inbox),
        // Media playback is clocked by the cpal output device (apps/deckcraft/src/audio.rs), exactly
        // as on the desktop; a backend that fails to initialise degrades to silent playback.
        audio_out: std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Box::new(audio::CpalOut::default()) as Box<dyn deckcraft_media::AudioOut>
        })).ok(),
        ..Services::default()
    }
}
''',
        "poll": '''
fn saf_poll(bridge: &saf::Bridge, state: &SafState, inbox: &SafInbox) {
    for _ in 0..16 {
        let raw = match bridge.poll() {
            Ok(Some(raw)) => raw,
            Ok(None) => break,
            Err(error) => {
                log::error!("Could not poll the Android file picker: {error}");
                break;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                log::error!("Ignoring a malformed Android picker result ({error}): {raw}");
                continue;
            }
        };
        let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) else { continue };
        let kind = value.get("kind").and_then(serde_json::Value::as_str).unwrap_or_default();
        let status = value.get("status").and_then(serde_json::Value::as_str).unwrap_or("error");
        match (kind, status) {
            ("open", "ok") => {
                let paths: Vec<String> = value
                    .get("paths")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry.as_str().map(str::to_owned))
                    .collect();
                for path in paths {
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            let name = std::path::Path::new(&path)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "imported-file".to_owned());
                            inbox.lock().unwrap_or_else(|error| error.into_inner()).push(InboxEvent::File(InboxFile { id, name, bytes }));
                        }
                        Err(error) => log::error!("Could not read the picked file {path}: {error}"),
                    }
                }
            }
            ("open", "cancel") => {
                inbox.lock().unwrap_or_else(|error| error.into_inner()).push(InboxEvent::Cancelled { id });
            }
            ("save", "ok") => {
                let bytes = state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                if let Some(bytes) = bytes {
                    match value.get("path").and_then(serde_json::Value::as_str) {
                        Some(path) => {
                            if let Err(error) = saf_write_and_commit(bridge, path, &bytes) {
                                log::error!("Could not write the saved file: {error}");
                            }
                        }
                        None => log::error!("Android save picker returned no destination path"),
                    }
                }
            }
            ("save", "cancel") => {
                state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
            _ => {
                if kind == "open" {
                    inbox.lock().unwrap_or_else(|error| error.into_inner()).push(InboxEvent::Cancelled { id });
                }
                if let Some(message) = value.get("message").and_then(serde_json::Value::as_str) {
                    log::error!("Android file picker error: {message}");
                }
                state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
        }
    }
    // A save through `pick_save` is written by the app itself to the staged path; push it to the
    // chosen SAF document once the write lands.
    state.mirror.sync(bridge);
}
''',
    },
    "gridcraft": {
        "activity": "GridCraftActivity",
        "imports": "use std::collections::HashMap;\nuse std::sync::atomic::{AtomicU64, Ordering};\nuse std::sync::{Arc, Mutex};",
        "state": '''
pub type SafInbox = gridcraft_ui_egui::Inbox;

#[derive(Default)]
pub struct SafState {
    next_id: AtomicU64,
    /// Which inbox each open request should deliver to: the app's shared inbox for File › Open,
    /// or a dialog's private inbox for its picture picker.
    opens: Mutex<HashMap<u64, SafInbox>>,
    saves: Mutex<HashMap<u64, Vec<u8>>>,
    mirror: saf::SaveMirror,
}

pub fn saf_new_state() -> Arc<SafState> {
    Arc::new(SafState::default())
}

fn saf_next_id(state: &SafState) -> u64 {
    state.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

/// Write to the staged private path, then push it through SAF when it was a chosen save
/// destination (a plain private path is a no-op commit).
fn saf_write_and_commit(bridge: &saf::Bridge, path: &str, bytes: &[u8]) -> Result<(), String> {
    let target = std::path::Path::new(path);
    if let Some(parent) = target.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(target, bytes).map_err(|error| format!("Could not write {}: {error}", target.display()))?;
    match bridge.commit(path)? {
        status if status == "NONE" || status == "OK" => Ok(()),
        status if status.starts_with("ERROR:") => Err(status.trim_start_matches("ERROR:").trim().to_owned()),
        status => Err(format!("Unexpected Android save result: {status}")),
    }
}

/// Pick a file to open and block until the user answers. Safe because the SAF bridge makes a
/// frame-thread wait non-blocking for the JVM callbacks (`NonblockingCallbacks`), and the pick has
/// its own timeout. `None` on cancel or error; the path is the staged app-private copy.
fn saf_pick_open(bridge: &saf::Bridge, state: &SafState) -> Option<String> {
    let id = saf_next_id(state);
    match bridge.pick_blocking(id, None, saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => paths.into_iter().next(),
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android open picker: {error}");
            None
        }
    }
}

/// Save dialog: the staged app-private destination, watched so the app's own write is mirrored to
/// the chosen SAF document on the next frames (GridCraft's file API is path-based). `None` on
/// cancel.
///
/// GridCraft's Save As hands over the bare title of a new workbook ("Book1") and adds the
/// ".xlsx" itself, but only after the picker has returned. Android would then create the
/// document without an extension, and the app would write "Book1.xlsx" beside the staged
/// copy: the document the user chose kept its bare name and stayed empty, because the write
/// went to a different file. Give the picker the name the app is going to write, so the
/// staged path is the path the app writes.
fn saf_pick_save(bridge: &saf::Bridge, state: &SafState, name: &str) -> Option<String> {
    let id = saf_next_id(state);
    let name = if !name.is_empty() && std::path::Path::new(name).extension().is_none() {
        format!("{name}.xlsx")
    } else {
        name.to_owned()
    };
    match bridge.pick_blocking(id, Some(&name), saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => {
            let path = paths.into_iter().next();
            if let Some(path) = &path {
                state.mirror.track(id, path);
            }
            path
        }
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android save picker: {error}");
            None
        }
    }
}
''',
        "services": '''
fn saf_services(bridge: saf::Bridge, state: Arc<SafState>, inbox: SafInbox) -> Services {
    let open_bridge = bridge.clone_ref();
    let picture_bridge = bridge.clone_ref();
    let save_bridge = bridge.clone_ref();
    let pick_open_bridge = bridge.clone_ref();
    let pick_save_bridge = bridge.clone_ref();
    let open_state = Arc::clone(&state);
    let picture_state = Arc::clone(&state);
    let save_state = Arc::clone(&state);
    let pick_open_state = Arc::clone(&state);
    let pick_save_state = Arc::clone(&state);
    let open_inbox = inbox.clone();
    let print_bridge = bridge.clone_ref();
    Services {
        // Android does not let an app enumerate printers: printer choice, paper, duplex, colour and
        // page range live in the system print dialog. Offer its one entry — named and defaulted so
        // File > Print's automatic selection settles on it and enables submission — and hand the
        // collated PDF to the Java print bridge, which stages it and opens that dialog. Without
        // these two, every File > Print said "Printing is unavailable here; export a PDF instead."
        printers: Some(Box::new(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(vec![gridcraft_ui_egui::printing::Printer {
                name: "Android print system".to_owned(),
                label: "".to_owned(),
                default: true,
            }]);
            rx
        })),
        print_native: Some(Box::new(move |bytes: Vec<u8>, _settings: gridcraft_ui_egui::printing::Settings| {
            let (tx, rx) = std::sync::mpsc::channel();
            // Java answers inside this call ("OK", or the reason it failed); an empty receipt
            // renders as the clean "Print job submitted." message, as the desktop spooler would.
            let receipt = print_bridge
                .print_document("GridCraft", &bytes)
                .and_then(|answer| {
                    if answer == "OK" {
                        Ok(String::new())
                    } else {
                        Err(answer)
                    }
                });
            let _ = tx.send(receipt);
            rx
        })),
        // Save As / Export use the synchronous `pick_save`, which Android never provided, so those
        // menu items were a silent no-op. The picker blocks on the frame thread, which the SAF
        // bridge makes safe (`NonblockingCallbacks`), exactly as EffectCraft does.
        pick_open: Some(Box::new(move || saf_pick_open(&pick_open_bridge, &pick_open_state))),
        pick_save: Some(Box::new(move |name: &str| saf_pick_save(&pick_save_bridge, &pick_save_state, name))),
        open_async: Some(Box::new(move || {
            let id = saf_next_id(&open_state);
            open_state.opens.lock().unwrap_or_else(|error| error.into_inner()).insert(id, open_inbox.clone());
            if let Err(error) = open_bridge.request_open(id, false) {
                log::error!("Could not start the Android open picker: {error}");
                open_state.opens.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
        })),
        pick_picture_async: Some(Box::new(move |target: SafInbox| {
            let id = saf_next_id(&picture_state);
            picture_state.opens.lock().unwrap_or_else(|error| error.into_inner()).insert(id, target);
            if let Err(error) = picture_bridge.request_open(id, false) {
                log::error!("Could not start the Android picture picker: {error}");
                picture_state.opens.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
        })),
        download: Some(Box::new(move |name: &str, bytes: &[u8]| {
            let id = saf_next_id(&save_state);
            save_state.saves.lock().unwrap_or_else(|error| error.into_inner()).insert(id, bytes.to_vec());
            if let Err(error) = save_bridge.request_save(id, name) {
                log::error!("Could not start the Android save picker: {error}");
                save_state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
        })),
        inbox: Some(inbox),
        ..Services::default()
    }
}
''',
        "poll": '''
fn saf_poll(bridge: &saf::Bridge, state: &SafState, _inbox: &SafInbox) {
    for _ in 0..16 {
        let raw = match bridge.poll() {
            Ok(Some(raw)) => raw,
            Ok(None) => break,
            Err(error) => {
                log::error!("Could not poll the Android file picker: {error}");
                break;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                log::error!("Ignoring a malformed Android picker result ({error}): {raw}");
                continue;
            }
        };
        let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) else { continue };
        let kind = value.get("kind").and_then(serde_json::Value::as_str).unwrap_or_default();
        let status = value.get("status").and_then(serde_json::Value::as_str).unwrap_or("error");
        match (kind, status) {
            ("open", "ok") => {
                let target = state.opens.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                let paths: Vec<String> = value
                    .get("paths")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry.as_str().map(str::to_owned))
                    .collect();
                if let Some(target) = target {
                    for path in paths {
                        match std::fs::read(&path) {
                            Ok(bytes) => {
                                let name = std::path::Path::new(&path)
                                    .file_name()
                                    .map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "imported-file".to_owned());
                                target.lock().unwrap_or_else(|error| error.into_inner()).push((name, bytes));
                            }
                            Err(error) => log::error!("Could not read the picked file {path}: {error}"),
                        }
                    }
                }
            }
            ("open", "cancel") => {
                state.opens.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
            ("save", "ok") => {
                let bytes = state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                if let Some(bytes) = bytes {
                    match value.get("path").and_then(serde_json::Value::as_str) {
                        Some(path) => {
                            if let Err(error) = saf_write_and_commit(bridge, path, &bytes) {
                                log::error!("Could not write the saved file: {error}");
                            }
                        }
                        None => log::error!("Android save picker returned no destination path"),
                    }
                }
            }
            ("save", "cancel") => {
                state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
            _ => {
                if kind == "open" {
                    state.opens.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                }
                if let Some(message) = value.get("message").and_then(serde_json::Value::as_str) {
                    log::error!("Android file picker error: {message}");
                }
                state.saves.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
            }
        }
    }
    // A save through `pick_save` is written by the app itself to the staged path; push it to the
    // chosen SAF document once the write lands.
    state.mirror.sync(bridge);
}
''',
    },
    "wordcraft": {
        "activity": "WordCraftActivity",
        "imports": "use std::sync::atomic::{AtomicU64, Ordering};\nuse std::sync::Arc;",
        "post_construct": """    load_wordcraft_user_fonts(&android_app);
""",
        "state": '''
pub type SafInbox = wordcraft_ui_egui::Inbox;

#[derive(Default)]
pub struct SafState {
    next_id: AtomicU64,
    mirror: saf::SaveMirror,
}

static LAST_LOADED_FONT_STATE: std::sync::Mutex<Option<Vec<(std::path::PathBuf, u64, u64)>>> = std::sync::Mutex::new(None);

pub fn load_wordcraft_user_fonts(app: &AndroidApp) {
    let mut candidate_dirs: Vec<std::path::PathBuf> = Vec::new();

    // 1. Shared storage fonts folders
    for shared in ["/storage/emulated/0/Fonts", "/sdcard/Fonts"] {
        let p = std::path::PathBuf::from(shared);
        if p.is_dir() && !candidate_dirs.contains(&p) {
            candidate_dirs.push(p);
        }
    }

    // 2. App-private external data fonts folder (/storage/emulated/0/Android/data/com.artcraft.wordcraft/files/fonts)
    if let Some(base) = app_data_dir(app) {
        let p = base.join("fonts");
        let _ = std::fs::create_dir_all(&p);
        if !candidate_dirs.contains(&p) {
            candidate_dirs.push(p);
        }
    }

    // 3. Settings dir fonts folder (e.g. config/wordcraft/fonts)
    let settings_font_dir = wordcraft_control_key::settings_dir().map(|d| d.join("fonts"));
    if let Some(ref dir) = settings_font_dir {
        let _ = std::fs::create_dir_all(dir);
        if !candidate_dirs.contains(dir) {
            candidate_dirs.push(dir.clone());
        }
    }

    // Fingerprint candidate folders to avoid redundant reloads
    let current_state: Vec<(std::path::PathBuf, u64, u64)> = candidate_dirs
        .iter()
        .map(|d| {
            let entries = std::fs::read_dir(d).map(|rd| rd.flatten().count() as u64).unwrap_or(0);
            let modified = std::fs::metadata(d)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|s| s.as_nanos() as u64)
                .unwrap_or(0);
            (d.clone(), entries, modified)
        })
        .collect();

    {
        let mut guard = LAST_LOADED_FONT_STATE.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref() == Some(&current_state) {
            return;
        }
        *guard = Some(current_state);
    }

    let db = wordcraft_fonts::FontDb::global();
    for dir in &candidate_dirs {
        if dir.is_dir() {
            let (loaded, skipped) = db.load_user_fonts(dir);
            if !loaded.is_empty() || !skipped.is_empty() {
                log::info!("WordCraft user fonts: {} loaded, {} skipped from {}", loaded.len(), skipped.len(), dir.display());
            }
        }
    }

    // Direct user_font_dir to the persistent writable folder so font.add saves there
    if let Some(ref dir) = settings_font_dir {
        db.load_user_fonts(dir);
    } else if let Some(base) = app_data_dir(app) {
        db.load_user_fonts(&base.join("fonts"));
    }

    let _ = db.load_system_fonts();
    wordcraft_engine::packs::use_font_store(true);
}

/// Show WordCraft's native file dialog without blocking the frame loop (#94, #511). WordCraft has
/// an asynchronous dialog API (`Services::file_dialog`) that exists for exactly this: the pick runs
/// on a worker thread and the app picks the answer up on a later frame (`poll_file_dialog`).
///
/// Blocking the frame thread instead is what broke Save As / Open / Export on Android: the frame
/// thread *is* the event-loop pump, so while it was parked the app never received Android's
/// `Suspended` lifecycle event and its wgpu surface was never torn down. On resume Android hands
/// over a new `ANativeWindow`, and recreating the surface for a window that still had a live
/// `VkSurfaceKHR` aborted the process with `AndroidSurface failed: ERROR_NATIVE_WINDOW_IN_USE_KHR`.
fn saf_file_dialog(
    bridge: saf::Bridge,
    state: Arc<SafState>,
    request: wordcraft_ui_egui::FileDialogRequest,
) -> std::sync::mpsc::Receiver<wordcraft_ui_egui::FileDialogAnswer> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let id = saf_next_id(&state);
        let (save_name, timeout) = match &request {
            // `..` and not a field list: the request has gained fields over time (the
            // folder the picker starts in, `dir`), and the launcher only cares about the
            // name, so it must not break when the source adds another one.
            wordcraft_ui_egui::FileDialogRequest::Save { name, .. } => (Some(name.as_str()), saf::PICK_TIMEOUT),
            // Every other request is an open; the catch-all keeps this working whether the
            // source has one open variant or several (`Open`, `OpenMany`).
            _ => (None, saf::PICK_TIMEOUT),
        };
        let answer = match bridge.pick_blocking_worker(id, save_name, timeout) {
            Ok(saf::PickAnswer::Paths(paths)) => {
                // WordCraft's file API is path-based: it writes the document itself, so watch the
                // staged destination and mirror any writes to the chosen SAF document later.
                for path in &paths {
                    state.mirror.track(id, path);
                }
                PICKED_PATHS
            }
            Ok(saf::PickAnswer::Cancelled) => PICK_CANCELLED,
            Err(error) => {
                log::error!("Android file picker: {error}");
                Err(error)
            }
        };
        let _ = tx.send(answer);
    });
    rx
}

pub fn saf_new_state() -> Arc<SafState> {
    Arc::new(SafState::default())
}

fn saf_next_id(state: &SafState) -> u64 {
    state.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}
''',
        "services": '''
fn saf_services(bridge: saf::Bridge, state: Arc<SafState>, inbox: SafInbox) -> Services {
    let print_bridge = bridge.clone_ref();
    let dialog_bridge = bridge.clone_ref();
    let dialog_state = Arc::clone(&state);
    Services {
        // Open / Save As / Export go through `ask_file`, which prefers this asynchronous dialog
        // (see `saf_file_dialog`): the pick runs off the frame thread, so the event loop keeps
        // pumping Android's lifecycle and the renderer swaps its surface safely around the picker.
        // A synchronous `pick_open`/`pick_save` here would park the frame thread and crash on a
        // surface that was never torn down (`ERROR_NATIVE_WINDOW_IN_USE_KHR`).
        file_dialog: Some(Box::new(move |request: wordcraft_ui_egui::FileDialogRequest| {
            saf_file_dialog(dialog_bridge.clone_ref(), Arc::clone(&dialog_state), request)
        })),
        // NOTE: no `open_async`. Every dialog — Open, Insert/Change Picture, Add Font, recipients,
        // Save As, Export, Print-to-PDF — goes through `file_dialog` above, so the app knows what
        // each picked file is for (`AfterPick`) and there is exactly one place that consumes picker
        // answers (the dialog's worker thread). `open_async` would answer through `inbox` instead,
        // competing with the worker for the same answer queue.
        //
        // NOTE: no `download` service. `download` is the browser's save mechanism: when it is set,
        // `file.save`/`file.saveAs`/`file.exportPdf`/`file.exportPng` ignore the picked path and
        // ask a second save dialog for the bytes, so exporting opened the picker twice (the first
        // destination stayed empty). Android saves like the desktop does — the engine writes to the
        // picked path and `SaveMirror` commits that staged file to the chosen SAF document — so the
        // path-based `file_dialog` above does the whole job with one dialog.
        // Android keeps printer choice inside the system print dialog: it reaches every printer
        // the device knows (Wi-Fi, network, cloud print services), not just ones an in-app list
        // could discover itself. Offer exactly that one entry — named and defaulted so File >
        // Print's auto-selection settles on it and enables submission — and the dialog the
        // submit opens does the real printer/paper/duplex/color/page-range picking.
        printers: Some(Box::new(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(vec![wordcraft_ui_egui::printing::Printer {
                name: "Android print system".to_owned(),
                label: "Android print system".to_owned(),
                default: true,
            }]);
            rx
        })),
        // The document arrives as a PDF WordCraft already collated (range and settings baked
        // in), so handing it straight to the system spooler is the whole job; Java answers
        // inside the same call, letting us report success or the reason for failure right
        // where the desktop spooler would have.
        print_native: Some(Box::new(move |bytes: Vec<u8>, _settings: wordcraft_ui_egui::printing::Settings, _collate: bool, _options: Vec<(String, String)>| {
            let (tx, rx) = std::sync::mpsc::channel();
            let receipt = print_bridge
                .print_document("WordCraft", &bytes)
                .and_then(|answer| {
                    if answer == "OK" {
                        // Empty receipt renders as the clean "Print job submitted." message.
                        Ok(String::new())
                    } else {
                        Err(answer)
                    }
                });
            let _ = tx.send(receipt);
            rx
        })),
        templates_dir: wordcraft_control_key::settings_dir().map(|d| d.join("Templates")),
        inbox: Some(inbox),
        ..Services::default()
    }
}
''',
        "poll": '''
fn saf_poll(bridge: &saf::Bridge, state: &SafState, _inbox: &SafInbox) {
    // Picker answers are consumed by the dialog's worker thread (`wait_answer_worker`), never here:
    // both would `poll()` the same queue and the frame could steal the answer the dialog is waiting
    // for. The frame only mirrors a finished path-based save to its chosen SAF document, once the
    // engine has written the staged file.
    state.mirror.sync(bridge);
}
''',
    },
    # ---- Synchronous file-dialog apps: the app asks for a path and continues at once. ----
    "effectcraft": {
        "activity": "EffectCraftActivity",
        "imports": "use effectcraft_ui_egui::Hooks;\nmod audio_out;\n" + SYNC_SAF_IMPORTS,
        "state": SYNC_SAF_STATE + '''
/// Open the picker allowing several files at once (imports take a list); empty on cancel or error.
fn saf_open_paths_many(bridge: &saf::Bridge, state: &SafState) -> Vec<String> {
    let id = saf_next_id(state);
    match bridge.pick_open_blocking(id, true, saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => paths,
        Ok(saf::PickAnswer::Cancelled) => Vec::new(),
        Err(error) => {
            log::error!("Android open picker: {error}");
            Vec::new()
        }
    }
}
''',
        "services": '''
/// Show a native file dialog without blocking the frame loop: the dialog runs on a worker thread
/// and answers through `done`, which the app may call from any thread. Blocking on the frame thread
/// instead stalls `AndroidApp::poll_events`, which Android's lifecycle callbacks wait on, so the
/// picker's `onPause` never returns and the system kills the app with an input-dispatch ANR;
/// `Hooks::file_dialog_async` exists for exactly this (#511).
fn saf_dialog_async(bridge: saf::Bridge, state: Arc<SafState>, dialog: effectcraft_ui_egui::FileDialog, done: effectcraft_ui_egui::DialogDone) {
    std::thread::spawn(move || {
        use effectcraft_ui_egui::FileDialog;
        let paths = match dialog {
            FileDialog::Open { many: true, .. } => saf_open_paths_many(&bridge, &state),
            FileDialog::Open { .. } | FileDialog::OpenProject => {
                saf_open_paths(&bridge, &state).into_iter().take(1).collect()
            }
            FileDialog::Folder => saf_folder_path(&bridge, &state).into_iter().collect(),
            FileDialog::Save { default, .. } => saf_save_path(&bridge, &state, &default).into_iter().collect(),
        };
        done(paths);
    });
}

fn saf_hooks(bridge: saf::Bridge, state: Arc<SafState>, _inbox: SafInbox) -> Hooks {
    let dialog_bridge = bridge.clone_ref();
    let dialog_state = Arc::clone(&state);
    let files_bridge = bridge.clone_ref();
    let files_state = Arc::clone(&state);
    let folder_bridge = bridge.clone_ref();
    let folder_state = Arc::clone(&state);
    let project_bridge = bridge.clone_ref();
    let project_state = Arc::clone(&state);
    let save_bridge = bridge.clone_ref();
    let save_state = Arc::clone(&state);
    let save_file_bridge = bridge.clone_ref();
    let save_file_state = Arc::clone(&state);
    Hooks {
        file_dialog_async: Some(Box::new(move |dialog: effectcraft_ui_egui::FileDialog, done: effectcraft_ui_egui::DialogDone| saf_dialog_async(dialog_bridge.clone_ref(), Arc::clone(&dialog_state), dialog, done))),
        pick_files: Some(Box::new(move |_exts: &[&str]| saf_open_paths(&files_bridge, &files_state))),
        pick_folder: Some(Box::new(move || saf_folder_path(&folder_bridge, &folder_state))),
        pick_open_project: Some(Box::new(move || saf_open_paths(&project_bridge, &project_state).into_iter().next())),
        pick_save: Some(Box::new(move |name: &str| saf_save_path(&save_bridge, &save_state, name))),
        pick_save_file: Some(Box::new(move |default: &str, _ext: &str| saf_save_path(&save_file_bridge, &save_file_state, default))),
        ..Hooks::default()
    }
}
''',
        "poll": SYNC_SAF_POLL,
        "construct_expr": "EffectcraftApp::new(effectcraft_host::session())",
        "post_construct": "    inner.hooks = saf_hooks(saf_bridge.clone_ref(), std::sync::Arc::clone(&saf_state), saf_inbox.clone());\n    inner.hooks.audio_device = Some(Box::new(|out: &effectcraft_ui_egui::audio::AudioOutput| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| audio_out::open(out))).ok().flatten()));\n    inner.hooks.audio_devices = Some(Box::new(|| std::panic::catch_unwind(std::panic::AssertUnwindSafe(audio_out::devices)).unwrap_or_default()));\n",
    },
    "filmcraft": {
        "activity": "FilmCraftActivity",
        "imports": "use filmcraft_ui_egui::HostHooks;\nmod audio;\n" + SYNC_SAF_IMPORTS,
        "state": SYNC_SAF_STATE,
        "services": '''
struct GpuFrameRenderer(filmcraft_gpu::ExportRenderer);

impl filmcraft_export::FrameRenderer for GpuFrameRenderer {
    fn render(
        &mut self,
        project: &filmcraft_project::Project,
        seq: filmcraft_project::ItemId,
        t: filmcraft_time::Tick,
        opts: filmcraft_render::RenderOptions,
        sources: &dyn filmcraft_render::SourceProvider,
    ) -> Option<filmcraft_render::Image> {
        self.0.render(project, seq, t, opts, sources)
    }

    fn caps(&self) -> filmcraft_render::plan::DeviceCaps {
        self.0.caps()
    }

    fn bind_surface(&mut self, window: *mut std::ffi::c_void, w: u32, h: u32) -> bool {
        self.0.bind_surface(window, w, h)
    }

    fn render_plan_to_surface(&mut self, plan: &filmcraft_render::plan::FramePlan) -> Option<bool> {
        self.0.render_plan_to_surface(plan)
    }

    fn render_to_surface(
        &mut self,
        project: &filmcraft_project::Project,
        seq: filmcraft_project::ItemId,
        t: filmcraft_time::Tick,
        opts: filmcraft_render::RenderOptions,
        sources: &dyn filmcraft_render::SourceProvider,
    ) -> Option<bool> {
        self.0.render_to_surface(project, seq, t, opts, sources)
    }

    fn render_rgba8(
        &mut self,
        project: &filmcraft_project::Project,
        seq: filmcraft_project::ItemId,
        t: filmcraft_time::Tick,
        opts: filmcraft_render::RenderOptions,
        sources: &dyn filmcraft_render::SourceProvider,
    ) -> Option<Vec<u8>> {
        self.0.render_rgba8(project, seq, t, opts, sources)
    }
}

/// Show a native file dialog without blocking the frame loop: the app gets the receiver, keeps
/// running its frames and runs the continuation once this worker thread answers. Blocking on the
/// frame thread instead stalls `AndroidApp::poll_events`, which Android's lifecycle callbacks wait
/// on, so the picker's `onPause` never returns and the system kills the app with an input-dispatch
/// ANR (native_dialogs.rs exists for exactly this; see its module docs).
fn saf_dialog_async(bridge: saf::Bridge, state: Arc<SafState>, dialog: filmcraft_ui_egui::native_dialogs::FileDialog) -> std::sync::mpsc::Receiver<Vec<String>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use filmcraft_ui_egui::native_dialogs::FileDialog;
        let paths = match dialog {
            FileDialog::OpenFiles { .. } => saf_open_paths(&bridge, &state),
            FileDialog::OpenFile { .. } | FileDialog::OpenProject | FileDialog::Relink { .. } => {
                saf_open_paths(&bridge, &state).into_iter().take(1).collect()
            }
            FileDialog::SaveProject { name } => saf_save_path(&bridge, &state, &name).into_iter().collect(),
            FileDialog::SaveAs { name, .. } => saf_save_path(&bridge, &state, &name).into_iter().collect(),
            FileDialog::Folder { .. } => saf_folder_path(&bridge, &state).into_iter().collect(),
        };
        // Sending fails only when the app dropped the dialog meanwhile, so the answer is moot.
        let _ = sender.send(paths);
    });
    receiver
}

fn saf_hooks(bridge: saf::Bridge, state: Arc<SafState>, _inbox: SafInbox) -> HostHooks {
    let dialog_bridge = bridge.clone_ref();
    let dialog_state = Arc::clone(&state);
    let files_bridge = bridge.clone_ref();
    let files_state = Arc::clone(&state);
    let folder_bridge = bridge.clone_ref();
    let folder_state = Arc::clone(&state);
    let folder_at_bridge = bridge.clone_ref();
    let folder_at_state = Arc::clone(&state);
    let project_bridge = bridge.clone_ref();
    let project_state = Arc::clone(&state);
    let save_bridge = bridge.clone_ref();
    let save_state = Arc::clone(&state);
    let save_as_bridge = bridge.clone_ref();
    let save_as_state = Arc::clone(&state);
    let open_file_bridge = bridge.clone_ref();
    let open_file_state = Arc::clone(&state);
    HostHooks {
        run_dialog: Some(Box::new(move |dialog: filmcraft_ui_egui::native_dialogs::FileDialog| saf_dialog_async(dialog_bridge.clone_ref(), Arc::clone(&dialog_state), dialog))),
        pick_files: Some(Box::new(move |_exts: &[&str]| saf_open_paths(&files_bridge, &files_state))),
        pick_folder: Some(Box::new(move || saf_folder_path(&folder_bridge, &folder_state))),
        pick_folder_at: Some(Box::new(move |_start: &str| saf_folder_path(&folder_at_bridge, &folder_at_state))),
        pick_open_project: Some(Box::new(move || saf_open_paths(&project_bridge, &project_state).into_iter().next())),
        pick_save: Some(Box::new(move |name: &str| saf_save_path(&save_bridge, &save_state, name))),
        pick_save_as: Some(Box::new(move |_filter: &str, _exts: &[&str], name: &str| saf_save_path(&save_as_bridge, &save_as_state, name))),
        pick_open_file: Some(Box::new(move |_filter: &str, _exts: &[&str]| saf_open_paths(&open_file_bridge, &open_file_state).into_iter().next())),
        ..HostHooks::default()
    }
}
''',
        "poll": SYNC_SAF_POLL,
        "setup": (
            "    filmcraft_platform::register();\n"
            "    filmcraft_export::register_frame_renderer(|| {\n"
            "        filmcraft_gpu::ExportRenderer::new().map(|r| Box::new(GpuFrameRenderer(r)) as Box<dyn filmcraft_export::FrameRenderer>)\n"
            "    });\n"
        ),
        "construct_expr": "FilmcraftApp::new(Session::default())",
        "post_construct": "    inner.hooks = saf_hooks(saf_bridge.clone_ref(), std::sync::Arc::clone(&saf_state), saf_inbox.clone());\n    inner.audio = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Box::new(audio::CpalOut::new()) as Box<dyn filmcraft_ui_egui::AudioOut>)).ok();\n",
    },
    "lightcraft": {
        "activity": "LightCraftActivity",
        "imports": SYNC_SAF_IMPORTS,
        "state": SYNC_SAF_STATE + '''
/// Write to the staged private path, then push it through SAF when it was a chosen destination
/// (an ordinary private path commits nothing). LightCraft writes exports through `Services::write`.
fn saf_write_and_commit(bridge: &saf::Bridge, path: &str, bytes: &[u8]) -> Result<(), String> {
    let target = std::path::Path::new(path);
    if let Some(parent) = target.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(target, bytes).map_err(|error| format!("Could not write {}: {error}", target.display()))?;
    match bridge.commit(path)? {
        status if status == "NONE" || status == "OK" => Ok(()),
        status if status.starts_with("ERROR:") => Err(status.trim_start_matches("ERROR:").trim().to_owned()),
        status => Err(format!("Unexpected Android save result: {status}")),
    }
}

/// Open the picker allowing several files at once (imports take a list); empty on cancel or error.
fn saf_open_paths_many(bridge: &saf::Bridge, state: &SafState) -> Vec<String> {
    let id = saf_next_id(state);
    match bridge.pick_open_blocking(id, true, saf::PICK_TIMEOUT) {
        Ok(saf::PickAnswer::Paths(paths)) => {
            for path in &paths {
                state.mirror.track(id, path);
            }
            paths
        }
        Ok(saf::PickAnswer::Cancelled) => Vec::new(),
        Err(error) => {
            log::error!("Android open picker: {error}");
            Vec::new()
        }
    }
}
''',
        "services": '''
/// Start the picker for a command's dialog off the UI thread and answer through a channel, which
/// the app polls (`lightcraft_ui_egui::pick` was written for exactly this, #191). Blocking the
/// frame thread instead stalls `AndroidApp::poll_events`, which Android's lifecycle callbacks wait
/// on, so the picker's `onPause` never returns and the system kills the app with an input-dispatch
/// ANR.
fn saf_services(bridge: saf::Bridge, state: Arc<SafState>, _inbox: SafInbox) -> Services {
    let picker_bridge = bridge.clone_ref();
    let picker_state = Arc::clone(&state);
    let write_bridge = bridge.clone_ref();
    let files_bridge = bridge.clone_ref();
    let files_state = Arc::clone(&state);
    let denoise_bridge = bridge.clone_ref();
    let denoise_state = Arc::clone(&state);
    let model_bridge = bridge.clone_ref();
    let model_state = Arc::clone(&state);
    let presets_bridge = bridge.clone_ref();
    let presets_state = Arc::clone(&state);
    let tracklog_bridge = bridge.clone_ref();
    let tracklog_state = Arc::clone(&state);
    let display_bridge = bridge.clone_ref();
    let display_state = Arc::clone(&state);
    let keywords_bridge = bridge.clone_ref();
    let keywords_state = Arc::clone(&state);
    let curves_bridge = bridge.clone_ref();
    let curves_state = Arc::clone(&state);
    let catalog_bridge = bridge.clone_ref();
    let catalog_state = Arc::clone(&state);
    let folder_bridge = bridge.clone_ref();
    let folder_state = Arc::clone(&state);
    let save_preset_bridge = bridge.clone_ref();
    let save_preset_state = Arc::clone(&state);
    let save_keywords_bridge = bridge.clone_ref();
    let save_keywords_state = Arc::clone(&state);
    let save_curves_bridge = bridge.clone_ref();
    let save_curves_state = Arc::clone(&state);
    Services {
        picker: Some(Box::new(move |request: lightcraft_ui_egui::pick::PickRequest| {
            let bridge = picker_bridge.clone_ref();
            let state = Arc::clone(&picker_state);
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                use lightcraft_ui_egui::pick::PickKind;
                let paths = match request.kind {
                    PickKind::Files => saf_open_paths_many(&bridge, &state),
                    PickKind::File => saf_open_paths(&bridge, &state).into_iter().take(1).collect(),
                    PickKind::Folder => saf_folder_path(&bridge, &state).into_iter().collect(),
                    PickKind::Save => {
                        let name = request.file_name.clone().unwrap_or_default();
                        saf_save_path(&bridge, &state, &name).into_iter().collect()
                    }
                };
                let _ = sender.send(paths);
            });
            Ok(receiver)
        })),
        pick_files: Some(Box::new(move || saf_open_paths(&files_bridge, &files_state))),
        pick_denoise_model: Some(Box::new(move || saf_open_paths(&denoise_bridge, &denoise_state))),
        pick_model_file: Some(Box::new(move || saf_open_paths(&model_bridge, &model_state))),
        pick_preset_files: Some(Box::new(move || saf_open_paths(&presets_bridge, &presets_state))),
        pick_tracklog: Some(Box::new(move || saf_open_paths(&tracklog_bridge, &tracklog_state))),
        pick_display_profile: Some(Box::new(move || saf_open_paths(&display_bridge, &display_state))),
        pick_keyword_list: Some(Box::new(move || saf_open_paths(&keywords_bridge, &keywords_state))),
        pick_curve_preset_files: Some(Box::new(move || saf_open_paths(&curves_bridge, &curves_state))),
        pick_lightroom_catalog: Some(Box::new(move || saf_folder_path(&catalog_bridge, &catalog_state))),
        pick_folder: Some(Box::new(move || saf_folder_path(&folder_bridge, &folder_state))),
        save_preset_file: Some(Box::new(move |name: &str| saf_save_path(&save_preset_bridge, &save_preset_state, name))),
        save_keyword_list: Some(Box::new(move |name: &str| saf_save_path(&save_keywords_bridge, &save_keywords_state, name))),
        save_curve_preset_file: Some(Box::new(move |name: &str| saf_save_path(&save_curves_bridge, &save_curves_state, name))),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| saf_write_and_commit(&write_bridge, path, bytes))),
        ..Services::default()
    }
}
''',
        "poll": SYNC_SAF_POLL,
    },
    "cadcraft": {
        "activity": "CadCraftActivity",
        # Staged working copies under media/: drop one when the app lets go of it and no view is
        # showing the drawing.
        "recent_paths": "self.inner.ui.recent_files.clone()",
        "open_paths": "self.inner.session.docs.iter().filter_map(|d| d.path.clone()).collect::<Vec<_>>()",
        "imports": SYNC_SAF_IMPORTS,
        "state": SYNC_SAF_STATE,
        "services": '''
fn saf_services(bridge: saf::Bridge, state: Arc<SafState>, _inbox: SafInbox) -> Services {
    let open_bridge = bridge.clone_ref();
    let open_state = Arc::clone(&state);
    let save_bridge = bridge.clone_ref();
    let save_state = Arc::clone(&state);
    Services {
        pick_open: Some(Box::new(move || saf_open_paths(&open_bridge, &open_state).into_iter().next())),
        pick_save: Some(Box::new(move |name: &str| saf_save_path(&save_bridge, &save_state, name))),
        ..Services::default()
    }
}
''',
        "poll": SYNC_SAF_POLL,
    },
    "soundcraft": {
        "activity": "SoundCraftActivity",
        # SoundCraft's Services::pick_open/pick_save/pick_folder take/return PathBuf/&Path,
        # not String/&str like the generic SAF glue.
        "imports": SYNC_SAF_IMPORTS + "\nuse std::path::{Path, PathBuf};",
        "state": SYNC_SAF_STATE,
        "services": '''
fn saf_services(bridge: saf::Bridge, state: Arc<SafState>, _inbox: SafInbox) -> Services {
    let open_bridge = bridge.clone_ref();
    let open_state = Arc::clone(&state);
    let save_bridge = bridge.clone_ref();
    let save_state = Arc::clone(&state);
    let folder_bridge = bridge.clone_ref();
    let folder_state = Arc::clone(&state);
    Services {
        pick_open: Some(Box::new(move |_title: &str, _exts: &[&str]| saf_open_paths(&open_bridge, &open_state).into_iter().next().map(PathBuf::from))),
        pick_save: Some(Box::new(move |_title: &str, name: &Path| saf_save_path(&save_bridge, &save_state, name.to_str().unwrap_or_default()).map(PathBuf::from))),
        pick_folder: Some(Box::new(move |_title: &Path| saf_folder_path(&folder_bridge, &folder_state).map(PathBuf::from))),
    }
}
''',
        "poll": SYNC_SAF_POLL,
    },
    # ---- rfd-driven apps: pdfcraft, materialcraft, texturecraft call rfd directly (no Services to inject). ----
    # The vendored rfd's Android backend routes its async and sync pickers through rfd::artcraft_saf,
    # which the launcher below drives: the launcher's per-frame poll delivers the system picker's answer.
    "pdfcraft": {
        "activity": "PdfCraftActivity",
        # Staged working copies under media/: drop one when the app lets go of it (Clear Recent, a
        # per-entry ×, or the capped list aging it out) and no tab is showing the document.
        "recent_paths": "self.inner.recent.iter().map(|r| r.path.clone()).collect::<Vec<_>>()",
        "open_paths": "self.inner.views.iter().filter_map(|v| self.inner.session.get(v.id).and_then(|d| d.path.clone())).collect::<Vec<_>>()",
        "imports": SYNC_SAF_IMPORTS,
        "state": SYNC_SAF_STATE,
        "services": "",
        "poll": RFD_SAF_POLL,
        "setup": RFD_SAF_SETUP,
        "construct_expr": "PdfCraftApp::new()",
        "post_construct": """    let print_bridge = saf_bridge.clone_ref();
    inner.printer_source = Some(std::sync::Arc::new(|| vec![
        pdfcraft_engine::print::spool::Printer {
            name: "Android print system".to_owned(),
            default: true,
            two_sided: None,
            description: "System print spooler".to_owned(),
        }
    ]));
    let spool_bridge = saf_bridge.clone_ref();
    inner.spooler = Some(std::sync::Arc::new(move |bytes: &[u8], job: &pdfcraft_engine::print::spool::Job| {
        let name = if job.title.is_empty() { "PdfCraft" } else { &job.title };
        spool_bridge.print_document(name, bytes)
    }));
    inner.print_handler = Some(std::sync::Arc::new(move |title: &str, bytes: &[u8]| {
        print_bridge.print_document(title, bytes)
    }));
""",
    },
    # ---- figcraft: a Services trait (not a struct) injected as Box<dyn Services>. ----
    # figcraft's Services::pick_open/pick_save/pick_folder return DialogFuture (async futures),
    # not plain Option<PathBuf>. Rewrite the sync form to async.
    "figcraft": {
        "activity": "FigCraftActivity",
        "imports": "use figcraft_ui_egui::{DialogFuture, Services};\nuse std::pin::Pin;\n" + SYNC_SAF_IMPORTS,
        "state": SYNC_SAF_STATE,
        "services": '''
struct SafServices {
    bridge: saf::Bridge,
    state: Arc<SafState>,
    app: AndroidApp,
}

impl SafServices {
    fn new(bridge: saf::Bridge, state: Arc<SafState>, _inbox: SafInbox, app: AndroidApp) -> Self {
        Self { bridge, state, app }
    }
}

impl Services for SafServices {
    fn pick_open(&mut self, _title: &str, _exts: &[&str]) -> DialogFuture {
        let bridge = self.bridge.clone_ref();
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            saf_open_paths(&bridge, &state).into_iter().next().map(std::path::PathBuf::from)
        })
    }
    fn pick_save(&mut self, _title: &str, name: &str, _exts: &[&str]) -> DialogFuture {
        let bridge = self.bridge.clone_ref();
        let state = Arc::clone(&self.state);
        let name = name.to_owned();
        Box::pin(async move {
            saf_save_path(&bridge, &state, &name).map(std::path::PathBuf::from)
        })
    }
    fn pick_folder(&mut self, _title: &str) -> DialogFuture {
        let bridge = self.bridge.clone_ref();
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            saf_folder_path(&bridge, &state).map(std::path::PathBuf::from)
        })
    }
    fn clipboard_get_text(&mut self) -> Option<String> {
        let t = android_clipboard_get(&self.app);
        if t.is_empty() { None } else { Some(t) }
    }
    fn clipboard_get_html(&mut self) -> Option<String> {
        android_clipboard_get_html(&self.app)
    }
    fn clipboard_set_text(&mut self, s: &str) {
        android_clipboard_set(&self.app, s);
    }
    fn open_url(&mut self, url: &str) {
        #[cfg(target_os = "android")]
        egui_winit::open_url::open_url(url);
    }
}
''',
        "poll": SYNC_SAF_POLL,
        "construct_expr": "FigApp::new(Session::new(), Box::new(SafServices::new(saf_bridge.clone_ref(), std::sync::Arc::clone(&saf_state), saf_inbox.clone(), android_app.clone())))",
    },
    "solidcraft": {
        "activity": "SolidCraftActivity",
        "imports": "use std::pin::Pin;\n" + SYNC_SAF_IMPORTS,
        "state": SYNC_SAF_STATE,
        "services": '''
fn saf_services(bridge: saf::Bridge, state: Arc<SafState>, _inbox: SafInbox) -> Services {
    let open_bridge = bridge.clone_ref();
    let open_state = Arc::clone(&state);
    let save_bridge = bridge.clone_ref();
    let save_state = Arc::clone(&state);
    let file_bridge = bridge.clone_ref();
    let file_state = Arc::clone(&state);
    Services {
        pick_open: Some(Box::new(move |_exts: &[&str]| {
            let bridge = open_bridge.clone_ref();
            let state = Arc::clone(&open_state);
            Box::pin(async move {
                saf_open_paths(&bridge, &state).into_iter().next()
            })
        })),
        pick_save: Some(Box::new(move |name: &str, _exts: &[&str]| {
            let bridge = save_bridge.clone_ref();
            let state = Arc::clone(&save_state);
            let name = name.to_owned();
            Box::pin(async move {
                saf_save_path(&bridge, &state, &name)
            })
        })),
        pick_file: Some(Box::new(move |_label: &str, _exts: &[&str]| {
            let bridge = file_bridge.clone_ref();
            let state = Arc::clone(&file_state);
            Box::pin(async move {
                saf_open_paths(&bridge, &state).into_iter().next()
            })
        })),
        ..Services::default()
    }
}
''',
        "poll": SYNC_SAF_POLL,
    },
    "materialcraft": {
        "activity": "MaterialCraftActivity",
        "recent_paths": "self.inner.ui.recent.clone()",
        "open_paths": "self.inner.session.path.clone().into_iter().collect::<Vec<_>>()",
        "imports": SYNC_SAF_IMPORTS,
        "state": SYNC_SAF_STATE,
        "services": "",
        "poll": RFD_SAF_POLL,
        "setup": RFD_SAF_SETUP,
        "construct_expr": "MaterialCraftApp::headless(materialcraft_engine::sample_session(), materialcraft_ui_egui::UiState::default())",
    },
    "texturecraft": {
        "activity": "TextureCraftActivity",
        "recent_paths": "self.inner.ui.recent.clone()",
        "open_paths": "self.inner.session.path.clone().into_iter().collect::<Vec<_>>()",
        "imports": SYNC_SAF_IMPORTS,
        "state": SYNC_SAF_STATE,
        "services": "",
        "poll": RFD_SAF_POLL,
        "setup": RFD_SAF_SETUP,
        "construct_expr": "TextureCraftApp::new(cc, texturecraft_engine::sample_session(1024, 256).expect(\"sample session\"))",
    },
}

# WordCraft's file-dialog answer type changed upstream: it used to be a single optional path
# (`Result<Option<String>, String>`) and is now the list of picked paths, empty when the user
# cancelled (`Result<Vec<String>, String>`), so that "Add Font…" can pick several files. The
# picker's answer is written in whichever shape the checked-out source expects, so the launcher
# keeps building against both.
_answer_type = re.search(r"pub type FileDialogAnswer\s*=\s*Result<\s*(\w+)", ui_text)
if _answer_type and _answer_type.group(1) == "Option":
    SAF_RECIPES["wordcraft"]["state"] = (
        SAF_RECIPES["wordcraft"]["state"]
        .replace("PICKED_PATHS", "Ok(paths.into_iter().next())")
        .replace("PICK_CANCELLED", "Ok(None)")
    )
else:
    SAF_RECIPES["wordcraft"]["state"] = (
        SAF_RECIPES["wordcraft"]["state"]
        .replace("PICKED_PATHS", "Ok(paths)")
        .replace("PICK_CANCELLED", "Ok(Vec::new())")
    )

# How the launcher opens a document the system handed it (an ACTION_VIEW/ACTION_SEND intent it is
# registered for), once per staged file. Each entry is the body of the shell's `open_incoming`, so it
# can use `self.inner`, `self.saf_inbox` and the `saf` bridge helpers. Apps with an import inbox
# receive name+bytes exactly like a file chosen in the picker; the others are asked to open the
# staged path through their own document API. Apps absent here simply ignore incoming files.
INTENT_OPEN = {
    "designcraft": '''        if let Err(error) = self.inner.open_dropped(path) {
            log::error!("Could not open the incoming file {path}: {error}");
        }
''',
    "deckcraft": '''        if let Err(error) = self.inner.open_path(path) {
            log::error!("Could not open the incoming file {path}: {error}");
        }
''',
    "gridcraft": '''        match std::fs::read(path) {
            Ok(bytes) => self.saf_inbox.lock().unwrap().push((saf::picker_name(path), bytes)),
            Err(error) => log::error!("Could not read the incoming file {path}: {error}"),
        }
''',
    "wordcraft": '''        match std::fs::read(path) {
            Ok(bytes) => self.saf_inbox.lock().unwrap().push((saf::picker_name(path), bytes)),
            Err(error) => log::error!("Could not read the incoming file {path}: {error}"),
        }
''',
    "cadcraft": '''        self.inner.open_path(std::path::Path::new(path));
''',
    "pdfcraft": '''        self.inner.open_path(path);
''',
    "lightcraft": '''        if let Err(error) = lightcraft_ui_egui::import::open(&mut self.inner, vec![path.to_owned()]) {
            log::error!("Could not open the incoming file {path}: {error}");
        }
''',
    "soundcraft": '''        if let Err(error) = soundcraft_ui_egui::dialogs::run_or_ask(&mut self.inner, "session.open", serde_json::json!({ "path": path })) {
            log::error!("Could not open the incoming file {path}: {error}");
        }
''',
    "effectcraft": '''        if let Err(error) = self.inner.session.execute("file.open", serde_json::json!({ "path": path })) {
            log::error!("Could not open the incoming file {path}: {error}");
        }
''',
    "filmcraft": '''        if let Err(error) = self.inner.file_dialog("file.open", &serde_json::json!({ "path": path })) {
            log::error!("Could not open the incoming file {path}: {error}");
        }
''',
    "figcraft": '''        let _ = self.inner.run("file.open", serde_json::json!({ "path": path }));
''',
    "solidcraft": '''        self.inner.open_path(path);
''',
    "materialcraft": '''        materialcraft_ui_egui::windows::open_path(&mut self.inner, path);
''',
    "texturecraft": '''        texturecraft_ui_egui::windows::open_path(&mut self.inner, path);
''',
}
saf = SAF_RECIPES.get(app)

stem = app[:-len("craft")] if app.endswith("craft") else app
expected = stem[:1].upper() + stem[1:] + "App"
app_type = spec.get("app_type") or next((x for x in candidates if x.lower() == expected.lower()), None)
if app_type is None:
    matching = sorted(x for x in candidates if x.lower().startswith(stem.lower()))
    if len(matching) == 1:
        app_type = matching[0]
    elif len(candidates) == 1:
        app_type = next(iter(candidates))
    else:
        raise SystemExit(
            f"Cannot uniquely discover the public UI app type for {app}. Candidates: {sorted(candidates)}. "
            f"Create adapters/{app}/Cargo.toml and adapters/{app}/src/lib.rs; see adapters/README.md."
        )
if app_type not in ui_root and not re.search(r"pub\s+use[\s\S]{0,500}" + re.escape(app_type), ui_root):
    raise SystemExit(
        f"Found {app_type}, but it is not visibly re-exported from crates/ui-egui/src/lib.rs. "
        f"A custom adapter is required; see adapters/README.md."
    )

engine_ident = engine_pkg.replace("-", "_")
ui_ident = ui_pkg.replace("-", "_")
construct = spec.get("construct", "{app}::new(Session::new(), Services::default())").format(
    app=app_type, engine=engine_ident, ui=ui_ident
)

# ---- Saving on Android: settings, preferences and the recent-files list ----------------------
# The desktop apps load and save these in `apps/<app>/src/main.rs` (a per-user config folder via
# `HOME`/`XDG_CONFIG_HOME`), and the web apps keep them in the browser. Neither is compiled into
# the generated NativeActivity launcher, so on Android nothing was written and every launch started
# from the defaults — settings, preferences and "recent files" were forgotten (only PhotoCraft, a
# custom adapter, had wired this up). The launcher now reproduces each app's own load/save calls,
# pointed at the app's private files directory (`AndroidApp::internal_data_path()`), which survives
# restarts and updates.
#
# `@UI@`/`@ENGINE@` become the app's crate idents. `inner` is the app and `dir` its config folder;
# `persist_read`/`persist_write` are the shared helpers below.
PERSIST_RECIPES = {
    "wordcraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "ui.json") {
        if let Some(ui) = @UI@::UiState::from_json(&bytes) {
            inner.apply_prefs(ui);
        }
    }''',
        "save": '''match serde_json::to_vec_pretty(&inner.prefs()) {
        Ok(bytes) => persist_write(dir, "ui.json", &bytes),
        Err(error) => log::warn!("saving settings: {error}"),
    }
    if inner.session.dirty && inner.session.path.is_some() {
        let _ = inner.run("file.save", serde_json::json!({}));
    }''',
    },
    "designcraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "ui.json") {
        if let Ok(ui) = @UI@::UiState::from_preferences_json(&bytes) {
            inner.ui = ui;
        }
    }
    if let Some(bytes) = persist_read(dir, "prefs.json") {
        if let Ok(prefs) = serde_json::from_slice::<@ENGINE@::Prefs>(&bytes) {
            inner.session.prefs = prefs;
        }
    }
    let recovery_root = dir.join("Recovery");
    if let Ok(run_dir) = @ENGINE@::recovery::run_dir(&recovery_root) {
        inner.session.recovery_dir = Some(run_dir);
        let _ = inner.session.execute("file.recovery.open", &serde_json::json!({}));
    }''',
        "save": '''if let Ok(bytes) = serde_json::to_vec_pretty(&inner.ui) {
        persist_write(dir, "ui.json", &bytes);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&inner.session.prefs) {
        persist_write(dir, "prefs.json", &bytes);
    }
    if inner.session.recovery_dir.is_some() && inner.session.documents().iter().any(|d| d.is_dirty()) {
        let _ = inner.session.execute("file.recovery.save", &serde_json::json!({}));
    }''',
    },
    "lightcraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "ui.json") {
        match serde_json::from_slice::<@UI@::UiState>(&bytes) {
            Ok(ui) => inner.ui = ui.sanitized(),
            Err(error) => log::warn!("settings: {error}"),
        }
    }
    let lib_dir = dir.join("library");
    inner.ui.settings.library_path = lib_dir.to_string_lossy().into_owned();
    if let Err(error) = inner.session.open_library(&lib_dir, false) {
        log::warn!("opening library in {}: {error}", lib_dir.display());
    }''',
        "save": '''if let Ok(bytes) = serde_json::to_vec_pretty(&inner.ui) {
        persist_write(dir, "ui.json", &bytes);
    }
    let _ = inner.session.persist();
    inner.session.save_view();
    let _ = inner.session.save_prefs();''',
    },
    "gridcraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "ui.json") {
        if let Ok(ui) = serde_json::from_slice::<@UI@::UiState>(&bytes) {
            inner.ui = ui;
        }
    }
    if let Some(bytes) = persist_read(dir, "prefs.json") {
        if let Ok(prefs) = serde_json::from_slice(&bytes) {
            inner.session.prefs = prefs;
        }
    }''',
        "save": '''if let Ok(bytes) = serde_json::to_vec_pretty(&inner.ui) {
        persist_write(dir, "ui.json", &bytes);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&inner.session.prefs) {
        persist_write(dir, "prefs.json", &bytes);
    }''',
    },
    "deckcraft": {
        "load": '''let (ui, prefs) = @UI@::settings::load(dir);
    if let Some(ui) = ui {
        inner.ui = ui;
    }
    if let Some(prefs) = prefs {
        inner.session.prefs = prefs;
    }''',
        "save": '''@UI@::settings::save(dir, &inner.ui, &inner.session.prefs);''',
    },
    "soundcraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "ui.json") {
        if let Ok(ui) = serde_json::from_slice::<@UI@::UiState>(&bytes) {
            inner.ui = ui;
        }
    }
    inner.autosave_dir = Some(dir.join("autosave"));
    inner.leftovers = @ENGINE@::autosave::leftovers(&dir.join("autosave"));''',
        "save": '''if let Ok(bytes) = serde_json::to_vec_pretty(&inner.ui) {
        persist_write(dir, "ui.json", &bytes);
    }
    if inner.engine.is_dirty() && inner.engine.path.is_some() {
        let _ = inner.run("session.save", serde_json::json!({}));
    }''',
    },
    "cadcraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "prefs.json") {
        if let Ok(json) = std::str::from_utf8(&bytes) {
            inner.load_prefs(json);
        }
    }
    if let Some(bytes) = persist_read(dir, "profile.json") {
        if let Ok(json) = std::str::from_utf8(&bytes) {
            inner.load_profile(json);
        }
    }
    if let Some(bytes) = persist_read(dir, "autosave.json") {
        if let Ok(json) = std::str::from_utf8(&bytes) {
            @UI@::autosave::load_prefs(json);
        }
    }
    inner.enable_autosave(&dir.join("autosave"));
    inner.session.set_lisp_support_dir(Some(dir.join("support")));''',
        "save": '''persist_write(dir, "prefs.json", inner.prefs_json().as_bytes());
    persist_write(dir, "profile.json", inner.profile_json().as_bytes());
    persist_write(dir, "autosave.json", @UI@::autosave::prefs_json().as_bytes());
    if inner.session.docs.iter().any(|d| d.is_dirty() && d.path.is_some()) {
        let _ = inner.run("qsave", serde_json::json!({}));
    }''',
    },
    "effectcraft": {
        "load": '''inner.session.autosave.background = true;
    inner.session.config = Some(std::sync::Arc::new(@ENGINE@::config::DirConfig::new(dir)));
    inner.session.load_settings();
    if let Some(recovery) = inner.session.begin_recovery() {
        inner.offer_recovery(recovery);
    }''',
        "save": '''inner.session.save_prefs();
    if inner.session.is_dirty() {
        let _ = inner.session.autosave_now();
        let _ = inner.session.autosave_wait();
    }''',
    },
    "filmcraft": {
        "load": '''let cfg = @ENGINE@::autosave::AutosaveConfig::new(dir.to_path_buf());
    if let Err(error) = inner.session.start_autosave(cfg) {
        log::warn!("auto-save and crash recovery unavailable: {error}");
    } else if !inner.session.recovery_candidates().is_empty() {
        inner.dialog = Some(@UI@::Dialog::Recovery);
    }''',
        "save": '''let prefs = inner.session.prefs.clone();
    if let Err(error) = inner.session.set_prefs(prefs) {
        log::warn!("saving preferences: {error}");
    }
    let _ = inner.session.execute("file.autoSaveNow", serde_json::json!({}));
    inner.session.sync_persistence();
    if let Some(p) = inner.session.persistence.as_ref() {
        p.flush();
    }''',
    },
    "pdfcraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "ui.json") {
        if let Ok(json) = std::str::from_utf8(&bytes) {
            inner.restore(json);
        }
    }
    inner.enable_recovery(@UI@::RecoveryStore::new(dir.join("recovery")));''',
        "save": '''persist_write(dir, "ui.json", inner.persist().as_bytes());''',
    },
    "figcraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "state.json") {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(prefs) = value.get("prefs").and_then(|p| serde_json::from_value::<@ENGINE@::Prefs>(p.clone()).ok()) {
                inner.session.prefs = prefs;
            }
        }
    }''',
        "save": '''match serde_json::to_string(&serde_json::json!({ "prefs": inner.session.prefs })) {
        Ok(json) => persist_write(dir, "state.json", json.as_bytes()),
        Err(error) => log::warn!("saving preferences: {error}"),
    }''',
    },
    "solidcraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "prefs.json") {
        if let Ok(prefs) = std::str::from_utf8(&bytes) {
            inner.load_prefs(prefs);
        }
    }''',
        "save": '''persist_write(dir, "prefs.json", inner.prefs().as_bytes());''',
    },
    "materialcraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "ui.json") {
        if let Ok(ui) = serde_json::from_slice::<@UI@::UiState>(&bytes) {
            inner.ui = ui;
            inner.apply_prefs();
        }
    }''',
        "save": '''if let Ok(bytes) = serde_json::to_vec_pretty(&inner.ui) {
        persist_write(dir, "ui.json", &bytes);
    }''',
    },
    "texturecraft": {
        "load": '''if let Some(bytes) = persist_read(dir, "ui.json") {
        if let Ok(ui) = serde_json::from_slice::<@UI@::UiState>(&bytes) {
            inner.ui = ui;
        }
    }''',
        "save": '''if let Ok(bytes) = serde_json::to_vec_pretty(&inner.ui) {
        persist_write(dir, "ui.json", &bytes);
    }''',
    },
}

# ---- UI scale on high-density screens -----------------------------------------------------------
# A dense screen (more than 600 dpi, see SafActivity.isHighDensityDevice) already draws the system
# interface very large, so the app's own interface scale has to be set explicitly or it comes out
# oversized. Screens at or below that density — tablets in particular — are left untouched: the app
# keeps its own default, Auto, which is the display scale Android reports and is comfortable there.
#
# The choice is remembered in a `device_class` sidecar (`device_scale_first_time`), so a first
# launch, and an upgrade from a version without this bootstrap, get the scale once, while a user's
# own later change is kept.
#
# The apps do not share one scale key or representation (int percent, choice string, f32 factor,
# `InterfaceSize` percentage), so each entry below spells out, per app, the Rust run after
# `persist_load` (against the loaded state) on such a screen. The values are the ones each app's own
# settings JSON stores — e.g. EffectCraft's `uiScale` is a percent, WordCraft's an absolute
# `InterfaceSize`, DesignCraft's `uiScale` a factor — so the app accepts them like its own.
# LightCraft has no persisted interface scale at all, so it is absent and untouched. PhotoCraft and
# SubCraft ship their own launchers and carry their own copy of this bootstrap; VectorCraft is
# listed here only so the builder's table is complete (its adapter's copy is the one that runs).
# `@UI@`/`@ENGINE@` are filled in like the recipes above.
UI_SCALE_BOOTSTRAP = {
    "effectcraft": '''let _ = inner.session.execute("prefs.set", serde_json::json!({"key": "appearance.uiScale", "value": "75"}));''',
    "filmcraft": '''let _ = inner.session.execute("prefs.set", serde_json::json!({"key": "appearance.uiScale", "value": "75"}));''',
    # VectorCraft's scale is a factor (1.0 is the system's own size); 0.75 gives the canvas more room.
    "vectorcraft": '''let _ = inner.session.execute("prefs.set", serde_json::json!({"key": "uiScaling", "value": 0.75}));''',
    # WordCraft's sizes are absolute ("100%" is one screen pixel per point, not the system's scale),
    # so a dense screen gets 150 % of that baseline.
    "wordcraft": '''inner.ui.interface_size = @UI@::theme::InterfaceSize::from_percent(150.0).unwrap_or_default();''',
    "designcraft": '''inner.ui.ui_scale = 0.75;''',
    "deckcraft": '''inner.ui.ui_scale = 0.75;''',
    # SoundCraft's slider tops out at 1.5 (set to 0.75 / 75% for mobile).
    "soundcraft": '''inner.ui.ui_scale = 0.75;''',
    # GridCraft's scale is a percentage (75–200).
    "gridcraft": '''inner.ui.interface_scale = 75;''',
    "pdfcraft": '''inner.ui_scale = @UI@::ui_scale::valid(1.5);''',
    # FigCraft keeps the scale in the engine's preferences, as a factor.
    "figcraft": '''inner.session.prefs.ui_scale = 0.75;''',
    "cadcraft": '''inner.options.interface_scale = 0.75;''',
    # Mobile apps scaled to 75%.
    "subcraft": '''inner.ui.ui_scale = 0.75;''',
}

# Run once right after `persist_load`, to open a launch on the app's own homepage instead of an
# empty document. The desktop apps do this in their `main.rs` (Word: a Start screen when launched
# without a document, `#766`; the rest open the File tab's Backstage Home). The generated launcher
# reaches the same public method/command, so a launch shows Blank document / templates / Recent —
# and nothing the loader applied can overwrite it, since it runs after `persist_load`.
# `@UI@` becomes the app's UI crate ident, like the recipes above.
PERSIST_AFTER_LOAD = {
    "wordcraft": "            inner.start_screen();\n",
    "gridcraft": "            inner.open_backstage(@UI@::backstage::Page::Home);\n",
    "deckcraft": "            inner.open_backstage(@UI@::backstage::Page::Home);\n",
}

# Shared Rust helpers the recipes above call (single braces: not interpolated as format fields).
PERSIST_SUPPORT = r'''
fn migrate_dir_contents(src: &std::path::Path, dst: &std::path::Path) {
    if !src.is_dir() {
        return;
    }
    let _ = std::fs::create_dir_all(dst);
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let src_path = entry.path();
            let file_name = entry.file_name();
            let dst_path = dst.join(file_name);
            if src_path.is_dir() {
                migrate_dir_contents(&src_path, &dst_path);
                let _ = std::fs::remove_dir(&src_path);
            } else if src_path.is_file() {
                if std::fs::rename(&src_path, &dst_path).is_err() {
                    if std::fs::copy(&src_path, &dst_path).is_ok() {
                        let _ = std::fs::remove_file(&src_path);
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir(src);
}

/// The app's private storage directory. Android exposes two: the internal one
/// (`/data/user/0/<pkg>/files`), which a file manager or a computer never sees, and the app-specific
/// external one (`/storage/emulated/0/Android/data/<pkg>/files`), which the user can reach over USB
/// or MTP without root. The external directory is preferred so the settings, autosaves and exported
/// documents are actually findable; the internal directory is the fallback for the rare device where
/// external storage is unmounted or unavailable. Both survive restarts and app updates.
fn app_data_dir(app: &AndroidApp) -> Option<std::path::PathBuf> {
    if let Some(path) = app.external_data_path() {
        let s = path.to_string_lossy();
        if s.contains("/Android/data/") {
            let replaced = s.replace("/Android/data/", "/Android/media/");
            let media_path = std::path::PathBuf::from(replaced);
            let _ = std::fs::create_dir_all(&media_path);
            if path.exists() && path != media_path {
                migrate_dir_contents(&path, &media_path);
            }
            if let Some(parent) = path.parent() {
                if parent.exists() {
                    if let Ok(entries) = std::fs::read_dir(parent) {
                        for entry in entries.flatten() {
                            let ep = entry.path();
                            if ep != path {
                                if let Some(name) = ep.file_name() {
                                    let dst = media_path.join(name);
                                    if ep.is_dir() {
                                        migrate_dir_contents(&ep, &dst);
                                    } else {
                                        let _ = std::fs::rename(&ep, &dst);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            return Some(media_path);
        }
        return Some(path);
    }
    app.internal_data_path()
}

/// The app's private files directory plus a `config` subfolder, where the settings, preferences and
/// recent-files list are kept. The folder survives restarts and app updates, which is what the
/// desktop versions get from their per-user config folder.
fn persist_dir(app: &AndroidApp) -> Option<std::path::PathBuf> {
    app_data_dir(app).map(|base| base.join("config"))
}

/// Point the platform path helpers at app-private storage before the app reads them, so anything
/// the app writes itself (autosave, recovery copies, caches) also lands there.
///
/// Only the settings and caches are set here, and only to the external app directory, so they stay
/// reachable over USB. `HOME` and `XDG_DATA_HOME` are left to
/// [`super::system_fonts::install_font_env`], which keeps them on internal storage: they are the
/// roots the apps' font scanners read, and the symlinks to `/system/fonts` they need can only be
/// created there (`/storage/emulated/0` refuses `symlink`).
#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) {
    let Some(base) = app_data_dir(app) else { return };
    let config = base.join("config");
    let cache = base.join("cache");
    let tmp = base.join("tmp");
    let _ = std::fs::create_dir_all(config.join(".config"));
    let _ = std::fs::create_dir_all(&cache);
    let _ = std::fs::create_dir_all(&tmp);
    // SAFETY: `android_main` runs before the app starts any worker thread, so no other thread is
    // reading the environment here.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("XDG_DATA_HOME", &base);
        std::env::set_var("TMPDIR", &tmp);
        let internal = app.internal_data_path().map(|p| p.join("models")).unwrap_or_else(|| base.join("models"));
        let external = base.join("models");
        if !internal.join("text-detection.onnx").is_file() || !internal.join("text-recognition.onnx").is_file() {
            let _ = std::fs::create_dir_all(&internal);
            for name in ["text-detection.onnx", "text-recognition.onnx"] {
                if let Ok(cname) = std::ffi::CString::new(format!("models/{name}")) {
                    if let Some(mut asset) = app.asset_manager().open(&cname) {
                        let dest = internal.join(name);
                        if let Ok(mut f) = std::fs::File::create(&dest) {
                            let _ = std::io::copy(&mut asset, &mut f);
                        }
                    }
                }
            }
        }
        if internal.join("text-detection.onnx").is_file() {
            std::env::set_var("PDFCRAFT_MODELS", &internal);
        } else {
            std::env::set_var("PDFCRAFT_MODELS", &external);
        }
    }
    log::info!("settings directory: {}", config.display());
}

#[allow(dead_code)]
fn persist_read(dir: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join(name)).ok()
}

/// Write `bytes` to `dir/name` through a temporary file renamed over the old one, so a crash leaves
/// the old file or the new one whole, never a truncated one.
#[allow(dead_code)]
fn persist_write(dir: &std::path::Path, name: &str, bytes: &[u8]) {
    if let Err(error) = std::fs::create_dir_all(dir) {
        log::warn!("settings: {}: {error}", dir.display());
        return;
    }
    let path = dir.join(name);
    let temporary = path.with_extension("tmp");
    let written = std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, &path));
    if let Err(error) = written {
        log::warn!("settings: {}: {error}", path.display());
    }
}

/// Whether the high-density UI scale should be applied to this launch, and the sidecar that
/// remembers that it was.
///
/// The scale belongs to the *device*, but the settings file it lands in belongs to the *user*: it is
/// written on the first launch and never revisited, so
///   * a screen that later stops being high-density (a foldable, an external display, a display-size
///     change) kept the old scale, and
///   * an install that already had a settings file — an upgrade from a version before this
///     bootstrap existed — never got it at all, so an updated app and a freshly installed one
///     behaved differently.
///
/// The choice is remembered in its own sidecar (`device_class`), separate from the app's settings,
/// so "first launch" means "the scale has not been set for this device yet", not "first launch of
/// this file". A user who changes the scale by hand keeps it: the sidecar already says it was
/// applied, so nothing is re-applied.
#[allow(dead_code)]
fn device_scale_first_time(dir: &std::path::Path) -> bool {
    let marker = dir.join("device_class");
    if std::fs::read_to_string(&marker).ok().is_some_and(|saved| saved.trim() == "scaled") {
        return false;
    }
    persist_write(dir, "device_class", b"scaled");
    true
}
'''

PERSIST_METHODS = r'''
    /// Save settings, preferences and the recent-files list, at most every two seconds. Android
    /// can kill a backgrounded app without a clean exit, so waiting for `on_exit` alone would lose
    /// whatever changed since the last write.
    fn persist_tick(&mut self) {
        let Some(dir) = self.persist.clone() else { return };
        let now = std::time::Instant::now();
        if self.persist_last.is_some_and(|last| now.duration_since(last) < std::time::Duration::from_secs(2)) {
            return;
        }
        self.persist_last = Some(now);
        persist_save(&mut self.inner, &dir);
    }

    /// Save now: the window lost focus (the app is being backgrounded) or it is exiting.
    fn persist_now(&mut self) {
        if let Some(dir) = self.persist.clone() {
            persist_save(&mut self.inner, &dir);
        }
        self.saf_state.mirror.sync(&self.saf_bridge);
    }
'''

PERSIST_HOOK = '''        if raw.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))) {
            self.persist_now();
        }
'''

PERSIST_ON_EXIT = '''
    fn on_exit(&mut self) {
        self.persist_now();
    }
'''

PERSIST_ON_EXIT_GLOW = '''
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_now();
    }
'''

# Prune the staged working copies under `media/` (see SafActivity.newStageFile) once the app no
# longer refers to them. The copy is a cache of a document whose original lives at its SAF
# location, so deleting it loses nothing while keeping `Android/data/<pkg>/files` small — which is
# the whole point of the directory being user-visible. `__RECENT__`/`__OPEN__` are filled in per
# app: expressions yielding the current recent-file paths and the paths of open documents.
STAGED_SWEEP_METHODS = r'''
    /// The staged copy of a document, if `path` names one: a *direct* child of the app's own
    /// `media/` directory. Anything else on the device is never touched.
    fn staged_media_file(bridge: &saf::Bridge, path: &str) -> Option<std::path::PathBuf> {
        let files = bridge.files_dir().ok()?;
        let media = files.join("media");
        let file = std::path::PathBuf::from(path);
        (file.parent() == Some(media.as_path())).then_some(file)
    }

    /// Delete every staged copy under `media/` that neither the recent list nor an open document
    /// refers to, sparing files written in the last minute (a document may be mid-import). This
    /// clears out copies orphaned before this pruning existed, or by an app killed mid-session.
    fn staged_sweep_orphans(bridge: &saf::Bridge, recent: &[String], open: &[String]) {
        let Ok(files) = bridge.files_dir() else { return };
        let media = files.join("media");
        let Ok(entries) = std::fs::read_dir(&media) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if recent.iter().any(|keep| std::path::Path::new(keep) == path.as_path())
                || open.iter().any(|keep| std::path::Path::new(keep) == path.as_path())
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let fresh = meta
                .modified()
                .ok()
                .and_then(|at| at.elapsed().ok())
                .is_some_and(|age| age < std::time::Duration::from_secs(60));
            if fresh {
                continue;
            }
            let name = path.to_string_lossy().into_owned();
            match std::fs::remove_file(&path) {
                Ok(()) => log::info!("deleted an orphaned staged copy: {name}"),
                Err(error) => log::warn!("could not delete the staged copy {name}: {error}"),
            }
        }
    }

    /// When an entry leaves the recent-files list (the list is cleared, an entry is removed with
    /// its ×, or it ages out of the capped list) and no open document is showing that file, its
    /// staged copy under `media/` is deleted: the document's original still lives at its SAF
    /// location. The `uri:` mapping in SharedPreferences is deliberately kept, so re-opening the
    /// same document reuses the same path and a later save still reaches the original. A delayed
    /// sweep of the whole directory clears out copies this diff never saw (an older session, or a
    /// crash); it runs once, half a minute after start, to let session restore finish first.
    fn staged_prune_tick(&mut self) {
        let current: Vec<String> = __RECENT__;
        let open: Vec<String> = __OPEN__;
        for stale in self.staged_seen.iter().filter(|path| !current.contains(*path) && !open.contains(*path)) {
            let Some(file) = Self::staged_media_file(&self.saf_bridge, stale) else { continue };
            match std::fs::remove_file(&file) {
                Ok(()) => log::info!("deleted the staged copy of a recent file the app let go: {stale}"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => log::warn!("could not delete the staged copy {stale}: {error}"),
            }
        }
        if self.staged_sweep_at.is_some_and(|at| std::time::Instant::now() >= at) {
            self.staged_sweep_at = None;
            Self::staged_sweep_orphans(&self.saf_bridge, &current, &open);
        }
        self.staged_seen = current;
    }
'''

_persist = PERSIST_RECIPES.get(app)
if _persist:
    _load = _persist["load"].replace("@UI@", ui_ident).replace("@ENGINE@", engine_ident)
    _save = _persist["save"].replace("@UI@", ui_ident).replace("@ENGINE@", engine_ident)
    persist_support = PERSIST_SUPPORT + f'''
fn persist_load(inner: &mut {app_type}, dir: &std::path::Path) {{
{_load}
}}

fn persist_save(inner: &mut {app_type}, dir: &std::path::Path) {{
{_save}
}}
'''
    persist_fields = "\n    persist: Option<std::path::PathBuf>,\n    persist_last: Option<std::time::Instant>,"
    # Open a launch on the app's own homepage (see PERSIST_AFTER_LOAD); runs after `persist_load`.
    _after = PERSIST_AFTER_LOAD.get(app, "").replace("@UI@", ui_ident)
    _bootstrap = UI_SCALE_BOOTSTRAP.get(app)
    if _bootstrap:
        # Give the app its own UI-scale default on a high-density screen, once (`device_scale_first_time`,
        # whose `device_class` sidecar keeps this independent of the settings file — so an upgrade
        # gets it too, while a user's own later change is kept). Every other screen is left at the
        # app's own default — Auto, the display scale Android reports — which is already comfortable
        # there.
        #
        # The scale runs *after* `persist_load` (so nothing the loader applies can overwrite it) and
        # is written straight back by `persist_save`.
        _scale = _bootstrap.replace("@UI@", ui_ident).replace("@ENGINE@", engine_ident)
        _bootstrap_body = (
            "let persist = persist_dir(&app);\n"
            "        if let Some(dir) = &persist {\n"
            "            persist_load(&mut inner, dir);\n"
            + _after
            + "            if device_is_high_density(&app) == Some(true) && device_scale_first_time(dir) {\n"
            f"                {_scale}\n"
            "                persist_save(&mut inner, dir);\n"
            "            }\n"
            "        }\n"
            "        "
        )
    else:
        _bootstrap_body = (
            "let persist = persist_dir(&app);\n"
            "        if let Some(dir) = &persist {\n"
            "            persist_load(&mut inner, dir);\n"
            + _after
            + "        }\n"
            "        "
        )
    persist_new_body = _bootstrap_body
    persist_new_init = ", persist, persist_last: None"
    persist_methods = PERSIST_METHODS
    persist_logic = "        self.persist_tick();\n"
    persist_hook = PERSIST_HOOK
    if app == "wordcraft":
        persist_hook += """        if raw.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(true))) {
            load_wordcraft_user_fonts(&self.app);
        }
"""
    lock_path = Path("Cargo.lock")
    eframe_has_glow = False
    if lock_path.is_file():
        lock_text = lock_path.read_text(errors="replace")
        m = re.search(r'\[\[package\]\]\s+name = "eframe".*?dependencies = \[(.*?)\]', lock_text, re.DOTALL)
        if m and '"glow"' in m.group(1):
            eframe_has_glow = True
    if app == "pdfcraft":
        eframe_has_glow = True

    persist_on_exit = PERSIST_ON_EXIT_GLOW if eframe_has_glow else PERSIST_ON_EXIT
    persist_env_call = "    install_persist_env(&android_app);\n"
else:
    persist_support = ""
    persist_fields = ""
    persist_new_body = ""
    persist_new_init = ""
    persist_methods = ""
    persist_logic = ""
    persist_hook = ""
    persist_on_exit = ""
    persist_env_call = ""
# Staged-copy pruning (see STAGED_SWEEP_METHODS): only for apps whose recipe spells out how to
# read their recent list and open documents; the others keep whatever is under `media/`.
_staged = (saf or {}).get("recent_paths")
if _staged:
    staged_fields = "\n    staged_seen: Vec<String>,\n    staged_sweep_at: Option<std::time::Instant>,"
    staged_new_init = ", staged_seen: Vec::new(), staged_sweep_at: Some(std::time::Instant::now() + std::time::Duration::from_secs(30))"
    staged_methods = STAGED_SWEEP_METHODS.replace("__RECENT__", _staged).replace("__OPEN__", (saf or {}).get("open_paths") or "Vec::new()")
    staged_logic = "        self.staged_prune_tick();\n"
else:
    staged_fields = ""
    staged_new_init = ""
    staged_methods = ""
    staged_logic = ""
control_tx_def = ""
control_init = ""
ctx_store = ""
if app in ("cadcraft", "wordcraft", "deckcraft", "designcraft", "lightcraft", "soundcraft"):
    control_tx_def = f"""pub static CONTROL_TX: std::sync::Mutex<Option<std::sync::mpsc::Sender<{ui_ident}::ControlRequest>>> = std::sync::Mutex::new(None);
pub static EGUI_CTX: std::sync::Mutex<Option<egui::Context>> = std::sync::Mutex::new(None);

"""
    control_init = """        let (control_tx, control_rx) = std::sync::mpsc::channel();
        if let Ok(mut g) = CONTROL_TX.lock() {
            *g = Some(control_tx);
        }
        inner = inner.with_control(control_rx);
"""
    ctx_store = """        if let Ok(mut g) = EGUI_CTX.lock() {
            *g = Some(ctx.clone());
        }
"""
elif app == "gridcraft":
    control_tx_def = f"""pub static CONTROL_TX: std::sync::Mutex<Option<std::sync::mpsc::Sender<{ui_ident}::ControlRequest>>> = std::sync::Mutex::new(None);
pub static EGUI_CTX: std::sync::Mutex<Option<egui::Context>> = std::sync::Mutex::new(None);

"""
    control_init = """        let (control_tx, control_rx) = std::sync::mpsc::channel();
        if let Ok(mut g) = CONTROL_TX.lock() {
            *g = Some(control_tx);
        }
        inner.control_rx = Some(control_rx);
"""
    ctx_store = """        if let Ok(mut g) = EGUI_CTX.lock() {
            *g = Some(ctx.clone());
        }
"""
elif app == "figcraft":
    control_tx_def = f"""pub static CONTROL_QUEUE: std::sync::Mutex<Vec<{ui_ident}::ControlRequest>> = std::sync::Mutex::new(Vec::new());
pub static EGUI_CTX: std::sync::Mutex<Option<egui::Context>> = std::sync::Mutex::new(None);

"""
    control_init = ""
    ctx_store = """        if let Ok(mut g) = EGUI_CTX.lock() {
            *g = Some(ctx.clone());
        }
        if let Ok(mut q) = crate::CONTROL_QUEUE.lock() {
            self.inner.control_queue.extend(q.drain(..));
        }
"""
# A type that does not implement `eframe::App` is an inner app state; its desktop `main.rs` wraps it
# in a tiny struct that forwards `logic`/`ui`. Emit the same wrapper here.
has_eframe_app = bool(re.search(r"impl\s+eframe::App\s+for\s+" + re.escape(app_type) + r"\b", ui_text))
has_logic = bool(re.search(r"\b(?:pub\s+)?fn\s+logic\b", ui_text)) and app != "figcraft"
has_raw_hook = bool(re.search(r"pub fn raw_input_hook\b", ui_text))
# A close guard: the app's own "may I close?" check. The desktop runs it when its window closes,
# so a document with unsaved changes asks first ("Do you want to quit without saving unsaved
# changes?"); the Android back button runs the very same check, so the app's own prompt appears
# instead of the app vanishing with unsaved work. Only the apps that have one are asked.
has_close_guard = bool(re.search(r"pub fn close_requested\b", ui_text))
# The flag such an app sets once the user answered that prompt ("Don't save", or the save
# finished). The desktop closes the window when it is set; on Android the activity finishes.
has_quit_flag = bool(re.search(r"pub quit_requested\b", ui_text))
if not re.search(r"\bServices\b|\bNoServices\b", construct) and "Services" not in ui_root and not spec:
    raise SystemExit(f"The UI crate does not visibly export Services; a custom adapter is required for {app}.")

# figcraft's `Services::pick_*` methods return a future on newer source releases and a plain
# Option on older ones. The recipe is written in the sync form, so rewrite it — and import the
# future type — to whatever the checked-out trait declares; otherwise a regenerated launcher
# cannot compile against the source it was generated from.
if saf and app == "figcraft" and re.search(r"fn pick_open\([^)]*\)\s*->\s*DialogFuture", ui_root):
    saf["imports"] += f"\nuse {ui_ident}::DialogFuture;"
    saf["services"] = re.sub(
        r"(fn pick_\w+\([^)]*\)) -> Option<[^>]*> \{\n(\s+)(.*?)\n\s*\}",
        lambda m: (
            f"{m.group(1)} -> DialogFuture {{\n"
            f"{m.group(2)}let result = {m.group(3).strip()};\n"
            f"{m.group(2)}Box::pin(async move {{ result }})\n"
            f"{m.group(2)[:-4]}}}"
        ),
        saf["services"],
        flags=re.S,
    )
    saf["services"] = saf["services"].replace(
        "fn pick_open(&mut self, _title: &str, _exts: &[&str]) -> DialogFuture {",
        "fn pick_open(&mut self, _title: &str, _exts: &[&str]) -> DialogFuture {"
    )

# Launcher icons come from the one shared module: density mipmaps plus the API 26+ adaptive
# layers (background / foreground / monochrome — see lib/app_icons.py). It prefers an explicit
# -appicon, then the app's own vector source, then its largest own PNG, and never another
# ArtCraft app's branding.
import app_icons

# Stage in a temporary sibling and publish the Android crate only when every generated file is ready.
staging_dir = android_dir.with_name(android_dir.name + ".artcraft-staging")
if staging_dir.exists():
    shutil.rmtree(staging_dir)
staging_dir.mkdir(parents=True)
try:
    icon = app_icons.generate(src, app, staging_dir / "res", Path(appicon) if appicon else None)

    serde_dep = 'serde_json = { workspace = true }\n' if (saf or _persist) else ''
    # The SAF bridge for rfd-driven apps (pdfcraft) calls `rfd::artcraft_saf` from the launcher, so
    # the launcher needs rfd as a direct dependency. The app's workspace patches it to the vendored
    # Android copy (see below), which carries the artcraft_saf module.
    rfd_dep = 'rfd = "0.17"\n' if re.search(r"\brfd::", ui_text) else ''
    # Audio output on Android comes from the platform layer: the launcher compiles the app's own
    # desktop cpal module (copied in below, next to `mod audio;`) and injects it into the app, so
    # the launcher needs cpal as a direct dependency. Only these apps ship such a module; the third
    # entry is (module name, source file, extra launcher dependencies).
    AUDIO_MODULES = {
        "filmcraft": ("audio", "audio.rs", ()),
        "effectcraft": ("audio_out", "audio_out.rs", ()),
        "deckcraft": ("audio", "audio.rs", ("deckcraft-media = { workspace = true }",)),
        "soundcraft": ("audio", "audio.rs", ()),
    }
    cpal_ver = "0.18"
    if app in AUDIO_MODULES:
        lock_p = src / "Cargo.lock"
        if lock_p.is_file():
            m_cpal = re.search(r'name = "cpal"\s+version = "(\d+\.\d+)', lock_p.read_text())
            if m_cpal:
                cpal_ver = m_cpal.group(1)
        else:
            for toml_p in src.glob("**/Cargo.toml"):
                m_cpal = re.search(r'cpal\s*=\s*(?:\{[^}]*version\s*=\s*)?"(\d+\.\d+)', toml_p.read_text())
                if m_cpal:
                    cpal_ver = m_cpal.group(1)
                    break
    cpal_dep = "".join(f"{dep}\n" for dep in (f'cpal = "{cpal_ver}"', *AUDIO_MODULES[app][2])) if app in AUDIO_MODULES else ''
    # EffectCraft's Android launcher must build a fully wired session (`effectcraft-host`), not a
    # bare `Session::new()`: without the host's importer and exporter, every import and export fails
    # with "media import is not available in this build" / "export is not available in this build".
    host_dep = 'effectcraft-host = { path = "../../crates/host" }\n' if app == "effectcraft" else ''
    # CADCraft keeps its file formats out of the engine on purpose (it stays I/O-agnostic): the
    # host installs them as global hooks — the DXF/DWG/SVG reader and writer, the plotter and the
    # entity codec AutoLISP's entget/entmake/entmod use. The desktop app does that (see
    # apps/cadcraft/src/main.rs); the launcher did not, so the engine had no file formats at all and
    # every open failed with "file formats are not available in this build". The launcher needs the
    # `cadcraft-io` crate to install them.
    io_dep = 'cadcraft-io = { path = "../../crates/io" }\n' if app == "cadcraft" else ''
    wordcraft_dep = 'wordcraft-fonts = { workspace = true }\nwordcraft-control-key = { workspace = true }\n' if app == "wordcraft" else ''
    platform_dep = (
        'filmcraft-platform = { path = "../../crates/platform" }\n'
        'filmcraft-gpu = { path = "../../crates/gpu" }\n'
        'filmcraft-export = { path = "../../crates/export" }\n'
        'filmcraft-render = { path = "../../crates/render" }\n'
        'filmcraft-project = { path = "../../crates/project" }\n'
        'filmcraft-time = { path = "../../crates/time" }\n'
        if app == "filmcraft"
        else ''
    )
    mcp_crates = {
        "solidcraft": "solidcraft-mcp",
        "cadcraft": "cadcraft-mcp",
        "wordcraft": "wordcraft-mcp",
        "figcraft": "figcraft-mcp",
        "gridcraft": "gridcraft-mcp",
        "deckcraft": "deckcraft-mcp",
        "designcraft": "designcraft-mcp",
        "lightcraft": "lightcraft-mcp",
    }
    automation_crates = {
        "pdfcraft": "pdfcraft-automation",
        "soundcraft": "soundcraft-automation",
        "effectcraft": "effectcraft-automation",
        "materialcraft": "materialcraft-automation",
        "texturecraft": "texturecraft-automation",
        "filmcraft": "filmcraft-automation",
    }
    mcp_dep = ""
    if app in mcp_crates:
        mcp_dep = f'{mcp_crates[app]} = {{ path = "../../crates/mcp" }}\n'
    elif app in automation_crates:
        mcp_dep = f'{automation_crates[app]} = {{ path = "../../crates/automation" }}\n'
    if app in ("materialcraft", "texturecraft", "filmcraft"):
        mcp_dep += 'tokio = { version = "1", features = ["rt", "sync", "io-util"] }\n'
    if app in ("materialcraft", "texturecraft"):
        mcp_dep += 'rmcp = { workspace = true }\n'
    if saf or _persist or app in mcp_crates or app in automation_crates:
        serde_dep = 'serde_json = { workspace = true }\n'
    java_sources = 'java_sources = "java"\n' if saf else ''
    config_changes = "orientation|keyboard|keyboardHidden|navigation|screenSize|screenLayout|smallestScreenSize|density|layoutDirection|uiMode|fontScale|locale"
    # Android open/share associations for the app's document types (the MAIN/LAUNCHER entry is always
    # first). Shared with the adapter launchers; see lib/file_associations.py.
    intent_filter_value = file_associations.intent_filter_toml(app)
    if saf:
        activity_block = f'''[[package.metadata.android.application.activity]]
name = ".{saf['activity']}"
config_changes = "{config_changes}"
label = "{title}"
launch_mode = "singleTask"
orientation = "unspecified"
exported = true
# The MAIN/LAUNCHER filter that gives the APK a launcher icon; the remaining filters register
# the app for its document types so the system can open and share them with it. Keep it inline:
# lib/file_associations.py rewrites the single `intent_filter = ...` line in place.
intent_filter = {intent_filter_value}

[[package.metadata.android.application.activity.meta_data]]
name = "android.app.lib_name"
value = "{app}_android"
'''
    else:
        activity_block = f'''[package.metadata.android.application.activity]
config_changes = "{config_changes}"
label = "{title}"
launch_mode = "singleTask"
orientation = "unspecified"
exported = true
intent_filter = {intent_filter_value}
'''

    package_name = packagename if packagename else f"com.artcraft.{app}"
    # Read the app's version from the source workspace manifest so the APK is named photocraft-0.8.0.apk.
    try:
        app_version = ws["workspace"]["package"]["version"]
    except (KeyError, TypeError):
        app_version = "0.0.0"
    apk_basename = f"{app}-{app_version}"
    ws_manifest_text = root_manifest.read_text(errors="replace")
    is_eframe_36 = bool(
        re.search(r'eframe\s*=\s*\{[^}]*version\s*=\s*"=?0\.36', ws_manifest_text)
        or re.search(r'eframe\s*=\s*"=?0\.36', ws_manifest_text)
    )
    eframe_features = (
        '["wgpu", "persistence"]'
        if (app == "solidcraft" or is_eframe_36)
        else '["wgpu", "persistence", "clipboard"]'
    )
    soundcraft_permission = ""
    if app == "soundcraft":
        soundcraft_permission = """
[[package.metadata.android.uses_permission]]
name = "android.permission.RECORD_AUDIO"
"""

    manifest = f'''[package]
name = "{app}-android"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
crate-type = ["cdylib"]

[dependencies]
{engine_pkg} = {{ path = "../../crates/engine" }}
{ui_pkg} = {{ path = "../../crates/ui-egui" }}
eframe = {{ workspace = true, default-features = false, features = {eframe_features} }}
egui = {{ workspace = true }}
egui-winit = {{ workspace = true }}
log = {{ workspace = true }}
jni = "0.22"
{serde_dep}{rfd_dep}{cpal_dep}{host_dep}{io_dep}{wordcraft_dep}{platform_dep}{mcp_dep}
[target.'cfg(target_os = "android")'.dependencies]
android-activity = {{ version = "0.6", default-features = false, features = ["native-activity"] }}
# Rust `log` records go to logcat (tag = the crate name). Without this every `log::error!`
# in the launcher and the SAF bridge vanished silently on device.
android_logger = "0.15"

[package.metadata.android]
package = "{package_name}"
apk_name = "{apk_basename}"
build_targets = ["aarch64-linux-android"]
resources = "res"
{java_sources}
[package.metadata.android.sdk]
min_sdk_version = 28
target_sdk_version = {target_sdk}

# Let the app's own file browsers read and write files on the device (see
# templates/android/SafActivity.java: safRequestStoragePermission). Legacy permissions cover
# Android 12 and older; "All files access" covers Android 11+.
[[package.metadata.android.uses_permission]]
name = "android.permission.READ_EXTERNAL_STORAGE"
max_sdk_version = 32

[[package.metadata.android.uses_permission]]
name = "android.permission.WRITE_EXTERNAL_STORAGE"
max_sdk_version = 29

[[package.metadata.android.uses_permission]]
name = "android.permission.MANAGE_EXTERNAL_STORAGE"

# The update checker asks GitHub for a newer release of the builder (see
# templates/android/UpdateCheck.java). Without this the request fails and no update is offered.
[[package.metadata.android.uses_permission]]
name = "android.permission.INTERNET"

# Audio input for recording (SoundCraft only). Required at runtime;
# cpal's AAudio backend requests the input stream only if the app opens it, but Android rejects
# the stream without the manifest permission.
{soundcraft_permission}

[package.metadata.android.application]
label = "{title}"
# NativeActivity integrates with Android's system input method framework: the soft keyboard
# appears automatically for focused egui text fields, no EditText bridge or manual IME polling
# needed. The Gradle step below generates res/values/themes.xml with this style.
theme = "@style/Theme.Artcraft"
icon = "@mipmap/ic_launcher"

{activity_block}'''
    (staging_dir / "Cargo.toml").write_text(manifest)

    if saf:
        pkg = packagename if packagename else f"com.artcraft.{app}"
        java_src = (root / "templates" / "android" / "SafActivity.java").read_text()
        java_src = (
            java_src.replace("__PACKAGE__", pkg)
            .replace("__CLASS__", saf["activity"])
            .replace("__TAG__", saf["activity"])
        )
        java_dir = staging_dir / "java" / Path(*pkg.split("."))
        java_dir.mkdir(parents=True, exist_ok=True)
        (java_dir / f"{saf['activity']}.java").write_text(java_src)

    imports = []
    # Pull in the shared Android system-font module (written next to our rust source below) so the
    # launcher can fall back to the device's own fonts for scripts the app's fonts do not cover.
    imports.append("mod system_fonts;")
    imports.append("mod mcp_http;")
    imports.append("mod mobile_text_bar;")
    imports.append("mod touch_gestures;")
    # The custom IME bridge is removed: reference Android apps use egui's native IME via
    # android-activity + eframe, without an EditText bridge. The custom bridge conflicts with
    # egui's IMEOutput handling and breaks input.
    # imports.append("mod ime_bridge;")
    if re.search(r"\bSession\b", construct):
        imports.append(f"use {engine_ident}::Session;")
    services_syms = [s for s in ("NoServices", "Services") if re.search(r"\b" + s + r"\b", (SAF_RECIPES.get(app) or {}).get("construct_expr") or construct)]
    if services_syms:
        imports.append("use " + ui_ident + "::{" + ", ".join([app_type] + services_syms) + "};")
    else:
        imports.append(f"use {ui_ident}::{app_type};")
    if app in ("cadcraft", "wordcraft", "deckcraft", "designcraft", "gridcraft", "lightcraft", "soundcraft"):
        imports.append(f"use {ui_ident}::ControlRequest;")
    use_block = "\n".join(imports)
    saf_block = ""
    if saf:
        use_block = use_block + "\n" + saf["imports"]
        saf_block = (root / "templates" / "android" / "saf_bridge.rs").read_text()
        saf_block = saf_block + "\n" + saf["state"] + saf["services"] + saf["poll"]

    # The soft keyboard is put away as soon as the app is scrolling or drawing rather than
    # editing. The app's canvas decides that and sets `request_ime_hide` (see lib/scrolling.sh,
    # which patches WordCraft's canvas); the shell reads it here, after the frame ran, and tells
    # the Activity to hide the keyboard. Only WordCraft carries the flag, so only it gets the
    # block; the other apps keep the keyboard where the platform puts it.
    ime_hide_logic = ""
    if app == "wordcraft":
        ime_hide_logic = (
            "        // The canvas worked out that the user is scrolling or drawing, not editing:\n"
            "        // put the soft keyboard away at once. It is read here, after the frame ran,\n"
            "        // because the input hook and the frame itself run before the IME output is\n"
            "        // handed to the platform.\n"
            "        if self.inner.request_ime_hide {\n"
            "            self.inner.request_ime_hide = false;\n"
            "            self.app.hide_soft_input(false);\n"
            "        }\n"
        )

    # With a mouse over the window, every hovered interactive widget must show the pointing hand;
    # egui only sets the OS pointer icon when `visuals.interact_cursor` names one, and the apps'
    # own themes never do. `system_fonts::install_system_fonts` is the one hook every shell calls
    # every frame (generated and adapter-owned alike), so the fix lives there; see
    # `templates/android/system_fonts.rs`.
    interact_cursor_logic = ""

    if has_eframe_app:
        inner_logic = "self.inner.logic(ctx, frame);"
        inner_ui = "self.inner.ui(ui, frame);"
        inner_hook = "self.inner.raw_input_hook(ctx, raw);"
    else:
        if has_logic:
            inner_logic = "let _ = frame;\n        self.inner.logic(ctx);"
        else:
            inner_logic = "let _ = (ctx, frame);"
        if app == "lightcraft":
            inner_ui = (
                "self.inner.perf.host_frame_ms = frame.info().cpu_usage.map(|s| f64::from(s) * 1000.0);\n"
                "        self.inner.ui(ui);"
            )
        else:
            inner_ui = "let _ = frame;\n        self.inner.ui(ui);"
        if has_raw_hook:
            if re.search(r"pub\s+fn\s+raw_input_hook\s*\(\s*&mut\s+self\s*,\s*raw\s*:", ui_text):
                inner_hook = "let _ = ctx;\n        self.inner.raw_input_hook(raw);"
            else:
                inner_hook = "self.inner.raw_input_hook(ctx, raw);"
        else:
            inner_hook = "let _ = (ctx, raw);"

    # The Android back button asks the app before it leaves, so an app with unsaved work opens
    # its own "quit without saving?" prompt instead of the app disappearing. The check is the
    # desktop's own (`close_requested`), and the flag the app sets once the user answered it
    # (`quit_requested`) ends the app. Apps without either close as they are.
    if has_close_guard:
        close_guard_code = (
            "        if !self.inner.close_requested() {\n"
            "            // The app asked the user first (it opened its own prompt) and will finish\n"
            "            // the app itself once the user confirms.\n"
            "            self.close_asked = true;\n"
            "            return;\n"
            "        }\n"
        )
    else:
        close_guard_code = ""
    if has_quit_flag:
        quit_check = (
            "        if !self.closing && self.inner.quit_requested {\n"
            "            // The user confirmed the app's own prompt (\"Don't save\", or the save\n"
            "            // finished): the app may go now.\n"
            "            self.closing = true;\n"
            "            finish_activity(&self.app);\n"
            "        }\n"
        )
    else:
        quit_check = ""

    app_upper = re.sub(r"[^A-Za-z0-9]", "_", app).upper()
    # The SAF marker tracks the generator version so a reader can tell which template produced a
    # given APK. The non-SAF marker keeps its own feature version (fullscreen + modifier fix).
    shell_marker = (
        f"ARTCRAFT_{app_upper}_ANDROID_SHELL_V{GENERATOR_VERSION}_SAF_FILEACCESS"
        if saf
        else f"ARTCRAFT_{app_upper}_ANDROID_SHELL_V3_MODIFIERS_FULLSCREEN"
    )

    platform = r'''
use android_activity::AndroidApp;
use jni::{JavaVM, jni_sig, jni_str, objects::{JObject, JString, JValue}};

/// Emitted at startup so a device log (and the packaged `.so` strings) can confirm that this
/// shell — including the Android modifier-key fix and immersive fullscreen — is the one running.
const ARTCRAFT_ANDROID_SHELL_MARKER: &str = "MARKER_PLACEHOLDER";

/// Hide the Android status and navigation bars so the egui canvas owns the whole screen.
///
/// The theme's `windowFullscreen` flag only hides the status bar; the navigation bar needs sticky
/// immersive mode. `android_main` runs on its own Rust thread, so the Java main (UI) thread is
/// free and these window calls can run on it without a Java activity subclass.
fn apply_immersive_fullscreen(app: &AndroidApp) {
    let app_clone = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
        let jvm = unsafe { JavaVM::from_raw(app_clone.vm_as_ptr() as *mut _) };
        let result = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity_raw = app_clone.activity_as_ptr() as jni::sys::jobject;
            // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it.
            let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
            let window = env
                .call_method(&*activity, jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[])?
                .l()?;
            if window.is_null() {
                return Ok(());
            }
            let decor = env
                .call_method(&window, jni_str!("getDecorView"), jni_sig!("()Landroid/view/View;"), &[])?
                .l()?;
            if decor.is_null() {
                return Ok(());
            }
            // LAYOUT_STABLE | HIDE_NAVIGATION | FULLSCREEN | LAYOUT_HIDE_NAVIGATION
            // | LAYOUT_FULLSCREEN | IMMERSIVE_STICKY
            const IMMERSIVE: i32 =
                0x0000_0100 | 0x0000_0002 | 0x0000_0004 | 0x0000_0200 | 0x0000_0400 | 0x0000_1000;
            // SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR: this
            // replace-style set must not wipe the icon contrast that reflects the app's theme
            // (setBarIconAppearance) on Android 9/10, where the refresh can run repeatedly.
            const LIGHT_ICON_FLAGS: i32 = 0x2000 | 0x0010;
            let visibility = env
                .call_method(
                    &decor,
                    jni_str!("getSystemUiVisibility"),
                    jni_sig!("()I"),
                    &[],
                )?
                .i()?;
            env.call_method(
                &decor,
                jni_str!("setSystemUiVisibility"),
                jni_sig!("(I)V"),
                &[JValue::Int((visibility & LIGHT_ICON_FLAGS) | IMMERSIVE)],
            )?;
            Ok(())
        });
        if let Err(error) = result {
            log::error!("Could not apply immersive fullscreen: {error}");
        }
    }));
}

/// Keep the system-bar icon contrast on the app's active theme.
///
/// The bars are edge-to-edge (they float over the canvas when a swipe brings them in), so their
/// icon color must follow what egui is drawing: dark icons over a light theme, light icons over
/// a dark one. egui's choice arrives here as `light_theme` and the Activity applies
/// `APPEARANCE_LIGHT_*` on the Java main thread, just like the fullscreen calls — `android_main`
/// must not block waiting for it. Runs only when the theme actually changes.
fn apply_bar_appearance(app: &AndroidApp, light_theme: bool) {
    let app_clone = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
        let jvm = unsafe { JavaVM::from_raw(app_clone.vm_as_ptr() as *mut _) };
        let result = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity_raw = app_clone.activity_as_ptr() as jni::sys::jobject;
            // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it.
            let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
            env.call_method(
                &*activity,
                jni_str!("setBarIconAppearance"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(light_theme)],
            )?;
            Ok(())
        });
        if let Err(error) = result {
            log::error!("Could not style the Android system bars: {error}");
        }
    }));
}

/// Finish the Activity, ending the app. The Activity's own `onDestroy` ends the process once it
/// is finishing (see templates/android/SafActivity.java). Runs on the Java main thread, like the
/// fullscreen and bar-appearance calls, because `Activity.finish()` must be called there.
fn finish_activity(app: &AndroidApp) {
    let app_clone = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
        let jvm = unsafe { JavaVM::from_raw(app_clone.vm_as_ptr() as *mut _) };
        let result = jvm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity_raw = app_clone.activity_as_ptr() as jni::sys::jobject;
            // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it.
            let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
            env.call_method(&*activity, jni_str!("finish"), jni_sig!("()V"), &[])?;
            Ok(())
        });
        if let Err(error) = result {
            log::error!("Could not finish the activity: {error}");
        }
    }));
}

/// Ask the Activity whether the screen is dense enough (>600 dpi) to give the app its own UI scale
/// instead of the system's. Returns `None` when the answer cannot be read, in which case the caller
/// leaves the UI scale alone.
///
/// The Activity reads the screen's `densityDpi`; the test and its rationale live in
/// `SafActivity.isHighDensityDevice`. Called once, on the Java main thread, using the same
/// activity-via-JNI pattern as the fullscreen and bar-appearance helpers above.
fn device_is_high_density(app: &AndroidApp) -> Option<bool> {
    // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
    let jvm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = jvm.attach_current_thread(|env| -> jni::errors::Result<bool> {
        // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it.
        let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
        Ok(env
            .call_method(&*activity, jni_str!("isHighDensityDevice"), jni_sig!("()Z"), &[])?
            .z()?)
    });
    match result {
        Ok(is_dense) => Some(is_dense),
        Err(error) => {
            log::warn!("Could not determine the screen density: {error}");
            None
        }
    }
}

/// Reconstruct egui's modifier state on Android.
///
/// winit's Android backend delivers `KeyboardInput` events but never emits `ModifiersChanged`
/// (it discards the `KeyEvent`'s meta state), so egui's modifier state stays empty and
/// `Ctrl`/`Alt` shortcuts are ignored while plain keys continue to work. Physical modifier keys
/// do arrive as `Event::Key` events (`Key::ControlLeft`, `Key::AltLeft`, ...), so we track them
/// and stamp the resulting state onto this frame's events, suppressing the text character that
/// would otherwise be emitted alongside a Ctrl/Command shortcut.
fn apply_android_modifiers(state: &mut egui::Modifiers, raw: &mut egui::RawInput) {
    let before = *state;
    let mut kept: Vec<egui::Event> = Vec::with_capacity(raw.events.len());
    for mut event in raw.events.drain(..) {
        match &mut event {
            egui::Event::ModifiersChanged(m) => {
                state.shift = m.shift;
                state.ctrl = m.ctrl;
                state.alt = m.alt;
                state.mac_cmd = m.mac_cmd;
                state.command = m.command || m.ctrl;
                kept.push(event);
            }
            egui::Event::Key { key, physical_key, pressed, modifiers, .. } => {
                let tracked = physical_key.unwrap_or(*key);
                match tracked {
                    egui::Key::ShiftLeft | egui::Key::ShiftRight => state.shift = *pressed,
                    egui::Key::ControlLeft | egui::Key::ControlRight => state.ctrl = *pressed,
                    egui::Key::AltLeft | egui::Key::AltRight => state.alt = *pressed,
                    _ => {}
                }
                state.mac_cmd = false;
                state.command = state.ctrl;
                if matches!(*key, egui::Key::Backspace | egui::Key::Delete) {
                    *modifiers = egui::Modifiers::default();
                } else {
                    modifiers.shift |= state.shift;
                    modifiers.ctrl |= state.ctrl;
                    modifiers.alt |= state.alt;
                    modifiers.command |= state.command;
                }
                kept.push(event);
            }
            egui::Event::PointerButton { modifiers, .. } => {
                modifiers.shift |= state.shift;
                modifiers.ctrl |= state.ctrl;
                modifiers.alt |= state.alt;
                modifiers.command |= state.command;
                kept.push(event);
            }
            egui::Event::MouseWheel { modifiers, .. } => {
                modifiers.shift |= state.shift;
                modifiers.ctrl |= state.ctrl;
                modifiers.alt |= state.alt;
                modifiers.command |= state.command;
                kept.push(event);
            }
            egui::Event::Text(_) if state.ctrl || state.command || state.mac_cmd => {}
            egui::Event::WindowFocused(false) => {
                *state = egui::Modifiers::default();
                kept.push(event);
            }
            _ => kept.push(event),
        }
    }
    raw.events = kept;
    if *state != before {
        raw.events.push(egui::Event::ModifiersChanged(*state));
    }
}

/// Read the Android system clipboard's text (empty when there is none). Reaches
/// `ClipboardManager` through JNI using the live VM and Activity held by `AndroidApp`,
/// so it works from `android_main`'s own thread without a Java activity subclass.
fn android_clipboard_get(app: &AndroidApp) -> String {
    let app = app.clone();
    // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<String> {
        // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
        // without taking ownership or attempting to delete the reference.
        let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
        let service = JObject::from(env.new_string("clipboard")?);
        let manager = env
            .call_method(
                &*activity,
                jni_str!("getSystemService"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                &[JValue::Object(&service)],
            )?
            .l()?;
        if manager.is_null() {
            return Ok(String::new());
        }
        let clip = env
            .call_method(
                &manager,
                jni_str!("getPrimaryClip"),
                jni_sig!("()Landroid/content/ClipData;"),
                &[],
            )?
            .l()?;
        if clip.is_null() {
            return Ok(String::new());
        }
        let count = env
            .call_method(&clip, jni_str!("getItemCount"), jni_sig!("()I"), &[])?
            .i()?;
        if count <= 0 {
            return Ok(String::new());
        }
        let item = env
            .call_method(
                &clip,
                jni_str!("getItemAt"),
                jni_sig!("(I)Landroid/content/ClipData$Item;"),
                &[JValue::Int(0)],
            )?
            .l()?;
        if item.is_null() {
            return Ok(String::new());
        }
        let text = env
            .call_method(
                &item,
                jni_str!("getText"),
                jni_sig!("()Ljava/lang/CharSequence;"),
                &[],
            )?
            .l()?;
        let text = if !text.is_null() {
            text
        } else {
            env.call_method(
                &item,
                jni_str!("coerceToText"),
                jni_sig!("(Landroid/content/Context;)Ljava/lang/CharSequence;"),
                &[JValue::Object(&*activity)],
            )?
            .l()?
        };
        if text.is_null() {
            return Ok(String::new());
        }
        let text = env
            .call_method(
                &text,
                jni_str!("toString"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        if text.is_null() {
            return Ok(String::new());
        }
        let text = JString::cast_local(env, text)?;
        Ok(text.try_to_string(env)?)
    });
    result.unwrap_or_else(|error| {
        log::error!("Could not read the Android clipboard: {error}");
        String::new()
    })
}

/// Write text to the Android system clipboard (`ClipboardManager`). Best-effort:
/// a refusal is logged, never raised into the frame loop.
fn android_clipboard_set(app: &AndroidApp, text: &str) {
    let app = app.clone();
    let text = text.to_owned();
    // SAFETY: AndroidApp owns the live VM pointer for the lifetime of the activity.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
        // without taking ownership or attempting to delete the reference.
        let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
        let service = JObject::from(env.new_string("clipboard")?);
        let manager = env
            .call_method(
                &*activity,
                jni_str!("getSystemService"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                &[JValue::Object(&service)],
            )?
            .l()?;
        if manager.is_null() {
            return Ok(());
        }
        let text_obj = JObject::from(env.new_string(&text)?);
        let label = JObject::from(env.new_string("egui")?);
        let clip = env
            .call_static_method(
                jni_str!("android/content/ClipData"),
                jni_str!("newPlainText"),
                jni_sig!("(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;"),
                &[JValue::Object(&label), JValue::Object(&text_obj)],
            )?
            .l()?;
        if clip.is_null() {
            return Ok(());
        }
        env.call_method(
            &manager,
            jni_str!("setPrimaryClip"),
            jni_sig!("(Landroid/content/ClipData;)V"),
            &[JValue::Object(&clip)],
        )?;
        Ok(())
    });
    if let Err(error) = result {
        log::error!("Could not write the Android clipboard: {error}");
    }
}

/// Read HTML text from the Android system clipboard (None when there is none).
fn android_clipboard_get_html(app: &AndroidApp) -> Option<String> {
    let app = app.clone();
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
        let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
        let service = JObject::from(env.new_string("clipboard")?);
        let manager = env
            .call_method(
                &*activity,
                jni_str!("getSystemService"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                &[JValue::Object(&service)],
            )?
            .l()?;
        if manager.is_null() {
            return Ok(None);
        }
        let clip = env
            .call_method(
                &manager,
                jni_str!("getPrimaryClip"),
                jni_sig!("()Landroid/content/ClipData;"),
                &[],
            )?
            .l()?;
        if clip.is_null() {
            return Ok(None);
        }
        let count = env
            .call_method(&clip, jni_str!("getItemCount"), jni_sig!("()I"), &[])?
            .i()?;
        if count <= 0 {
            return Ok(None);
        }
        let item = env
            .call_method(
                &clip,
                jni_str!("getItemAt"),
                jni_sig!("(I)Landroid/content/ClipData$Item;"),
                &[JValue::Int(0)],
            )?
            .l()?;
        if item.is_null() {
            return Ok(None);
        }
        let html = env
            .call_method(
                &item,
                jni_str!("getHtmlText"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        if html.is_null() {
            return Ok(None);
        }
        let html = JString::cast_local(env, html)?;
        let s = html.try_to_string(env)?;
        if s.is_empty() {
            Ok(None)
        } else {
            Ok(Some(s))
        }
    });
    result.unwrap_or_else(|error| {
        log::error!("Could not read HTML from Android clipboard: {error}");
        None
    })
}

/// Set HTML and plain text to the Android clipboard.
fn android_clipboard_set_html(app: &AndroidApp, text: &str, html: &str) -> bool {
    let app = app.clone();
    let text = text.to_owned();
    let html = html.to_owned();
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<bool> {
        let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
        let service = JObject::from(env.new_string("clipboard")?);
        let manager = env
            .call_method(
                &*activity,
                jni_str!("getSystemService"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                &[JValue::Object(&service)],
            )?
            .l()?;
        if manager.is_null() {
            return Ok(false);
        }
        let label = JObject::from(env.new_string("egui")?);
        let text_obj = JObject::from(env.new_string(&text)?);
        let html_obj = JObject::from(env.new_string(&html)?);
        let clip = env.call_static_method(
            jni_str!("android/content/ClipData"),
            jni_str!("newHtmlText"),
            jni_sig!("(Ljava/lang/CharSequence;Ljava/lang/CharSequence;Ljava/lang/String;)Landroid/content/ClipData;"),
            &[JValue::Object(&label), JValue::Object(&text_obj), JValue::Object(&html_obj)],
        )?.l()?;
        if clip.is_null() {
            return Ok(false);
        }
        env.call_method(
            &manager,
            jni_str!("setPrimaryClip"),
            jni_sig!("(Landroid/content/ClipData;)V"),
            &[JValue::Object(&clip)],
        )?;
        Ok(true)
    });
    result.unwrap_or_else(|error| {
        log::error!("Could not write HTML to Android clipboard: {error}");
        false
    })
}
'''.replace("MARKER_PLACEHOLDER", shell_marker)

    # CADCraft's file-format hooks. The engine stays I/O-agnostic on purpose: the *host* installs
    # them — the desktop app does (see apps/cadcraft/src/main.rs), and the launcher now does too.
    # Without them the engine has no file formats at all, and every open fails with "file formats
    # are not available in this build".
    if app == "cadcraft":
        platform += r'''
/// CADCraft's file formats: the DXF/DWG/SVG reader and writer, the plotter, and the entity
/// codec AutoLISP's `entget`/`entmake`/`entmod` use. The engine keeps its I/O out on purpose —
/// the host installs these (the desktop app does, see apps/cadcraft/src/main.rs).
fn install_cadcraft_io() {
    __ENGINE__::cmd::file::set_io(__ENGINE__::cmd::file::IoHooks {
        read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
        write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|e| e.to_string())),
    });
    __ENGINE__::cmd::file::set_write_as(|d, name, o| {
        cadcraft_io::write_with(d, name, &cadcraft_io::SaveOptions { version: o.version, binary: o.binary })
            .map_err(|e| e.to_string())
    });
    // AutoLISP's entget, entmake and entmod see entities as DXF groups.
    __ENGINE__::cmd::file::set_entity_codec(__ENGINE__::cmd::file::EntityCodec {
        groups: cadcraft_io::entity_groups,
        entity: cadcraft_io::entity_from_groups,
    });
}
'''.replace("__ENGINE__", engine_ident)

    if saf:
        saf_fields = "\n    saf_bridge: saf::Bridge,\n    saf_state: std::sync::Arc<SafState>,\n    saf_inbox: SafInbox,"
        saf_new_params = ", saf_bridge: saf::Bridge, saf_state: std::sync::Arc<SafState>, saf_inbox: SafInbox"
        saf_new_init = ", saf_bridge, saf_state, saf_inbox"
        saf_poll_method = '''
    /// Drain any finished Android file-picker requests and hand the results to the app.
    fn poll_file_picker(&mut self) {
        saf_poll(&self.saf_bridge, &self.saf_state, &self.saf_inbox);
    }
'''
        saf_logic_poll = "        self.poll_file_picker();\n"
        # Open documents the system handed us (ACTION_VIEW/ACTION_SEND). The Activity stages each
        # file off the UI thread and queues its private path; wait a couple of frames so the app has
        # finished its own start-up, then open one per frame exactly like a picked file.
        intent_body = INTENT_OPEN.get(app, "")
        if intent_body:
            intent_method = f'''
    fn open_incoming(&mut self, path: &str) {{
        log::info!("Opening the file the system handed the app: {{path}}");
        self.saf_state.mirror.track(0, path);
{intent_body}    }}

    /// Watch for files the system (or another app's drop) handed the app while it was idle.
    ///
    /// egui runs a frame only when something asks it to, so a file that arrives while the app
    /// sits untouched would wait in the Activity's queue until the user happened to touch the
    /// screen. The watcher asks the Activity how many files are waiting (without taking any of
    /// them) and asks egui for a repaint when there is one, so the file opens on its own. The
    /// file itself is still opened on the frame thread, in `poll_incoming_files`.
    fn watch_incoming(&mut self, ctx: &egui::Context) {{
        static WATCHER: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        if WATCHER.set(()).is_err() {{
            return;
        }}
        let bridge = self.saf_bridge.clone_ref();
        let ctx = ctx.clone();
        std::thread::spawn(move || loop {{
            std::thread::sleep(std::time::Duration::from_millis(200));
            if !ALIVE.load(std::sync::atomic::Ordering::Relaxed) {{
                break;
            }}
            match bridge.intent_pending() {{
                Ok(pending) if pending > 0 => ctx.request_repaint(),
                Ok(_) => {{}}
                Err(error) => {{
                    log::info!("Launcher does not report incoming files: {{error}}");
                    break;
                }}
            }}
        }});
    }}

    /// Open any file the system handed the app (an intent it is registered for). Runs once per
    /// staged file and is a no-op afterwards; a launcher without the intent host stops the poll.
    fn poll_incoming_files(&mut self) {{
        if self.intent_warmup > 0 {{
            self.intent_warmup -= 1;
            return;
        }}
        if self.intent_warmup < 0 {{
            return;
        }}
        for _ in 0..16 {{
            match self.saf_bridge.intent_poll() {{
                Ok(Some(path)) => self.open_incoming(&path),
                Ok(None) => break,
                Err(error) => {{
                    log::info!("Launcher does not report incoming files: {{error}}");
                    self.intent_warmup = -1;
                    break;
                }}
            }}
        }}
    }}
'''
            intent_logic = "        self.watch_incoming(ctx);\n        self.poll_incoming_files();\n"
            intent_field = "\n    intent_warmup: i64,"
            intent_init = ", intent_warmup: 2"
        else:
            intent_method = ""
            intent_logic = ""
            intent_field = ""
            intent_init = ""
        saf_setup = saf.get("setup", "")
        post_construct = saf.get("post_construct", "")
        if "construct_expr" in saf:
            construct_expr = saf["construct_expr"]
            if app == "lightcraft" and ".with_fs()" not in construct_expr:
                construct_expr = construct_expr.replace("Session::new()", "Session::new().with_fs()")
        else:
            if "Services::default()" not in construct:
                raise SystemExit(f"The SAF recipe for {app} expects a `Services::default()` constructor, found: {construct}")
            construct_expr = construct.replace("Services::default()", "platform_services")
            # Ensure filesystem hooks are installed for apps that need to read staged files (e.g., lightcraft with SAF)
            if app == "lightcraft" and ".with_fs()" not in construct_expr:
                construct_expr = construct_expr.replace("Session::new()", "Session::new().with_fs()")
            if not saf_setup:
                saf_setup = "    let platform_services = saf_services(saf_bridge.clone_ref(), std::sync::Arc::clone(&saf_state), saf_inbox.clone());\n"
        inner_decl = "let mut inner" if post_construct else "let inner"
    else:
        saf_fields = ""
        saf_new_params = ""
        saf_new_init = ""
        saf_poll_method = ""
        saf_logic_poll = ""
        construct_expr = construct
        saf_setup = ""
        post_construct = ""
        inner_decl = "let inner"
        intent_method = ""
        intent_logic = ""
        intent_field = ""
        intent_init = ""

    topbar_modifier_hook = ""
    if app in ("designcraft", "wordcraft", "gridcraft", "deckcraft"):
        # The Shift/Ctrl/Alt buttons stand in for the soft keyboard: with a hardware keyboard
        # attached they are in the way, so the launcher tells the app's top bar about it.
        topbar_modifier_hook = (
            f"        {ui_ident}::topbar_modifiers::apply_to_raw_input(&self.modifiers, raw);\n"
            f"        {ui_ident}::topbar_modifiers::set_hardware_keyboard_connected(\n"
            f"            android_activity::hardware_keyboard_connected(),\n"
            f"        );\n"
        )

    app_clipboard_init = ""
    if app == "wordcraft":
        app_clipboard_init = """        let app_clip_text = app.clone();
        inner.services.clipboard_text = Some(Box::new({
            let app = app_clip_text.clone();
            move || {
                let t = android_clipboard_get(&app);
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            }
        }));
        let app_clip_html = app.clone();
        inner.services.clipboard_html = Some(Box::new({
            let app = app_clip_html.clone();
            move || android_clipboard_get_html(&app)
        }));
        let app_clip_set_rich = app.clone();
        inner.services.set_clipboard_rich = Some(Box::new({
            let app = app_clip_set_rich.clone();
            move |text, html, _rtf, _mathml| {
                if !html.is_empty() {
                    android_clipboard_set_html(&app, text, html)
                } else {
                    android_clipboard_set(&app, text);
                    true
                }
            }
        }));
"""
    elif app == "designcraft":
        app_clipboard_init = """        let app_clip_text = app.clone();
        inner.services.clipboard_text = Some(Box::new({
            let app = app_clip_text.clone();
            move || {
                let t = android_clipboard_get(&app);
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            }
        }));
"""
    elif app == "effectcraft":
        app_clipboard_init = """        let app_clip_text = app.clone();
        inner.hooks.clipboard_text = Some(Box::new({
            let app = app_clip_text.clone();
            move || {
                let t = android_clipboard_get(&app);
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            }
        }));
"""
    elif app == "gridcraft":
        app_clipboard_init = """        let app_clip_text = app.clone();
        inner.services.paste_text = Some(Box::new({
            let app = app_clip_text.clone();
            move || {
                let t = android_clipboard_get(&app);
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            }
        }));
        let app_clip_html = app.clone();
        inner.services.paste_html = Some(Box::new({
            let app = app_clip_html.clone();
            move || android_clipboard_get_html(&app)
        }));
"""
    elif app == "deckcraft":
        app_clipboard_init = """        let app_clip_html = app.clone();
        inner.services.clipboard_html = Some(Box::new({
            let app = app_clip_html.clone();
            move || android_clipboard_get_html(&app)
        }));
        let app_clip_set_html = app.clone();
        inner.services.clipboard_set_html = Some(Box::new({
            let app = app_clip_set_html.clone();
            move |html, text| {
                android_clipboard_set_html(&app, text, html)
            }
        }));
"""

    shell = f'''
/// On-device input diagnostics (reported through `adb logcat`).
///
/// Counts the egui events the platform backend produced since the last report: pointer
/// motion, mouse buttons, wheel scrolls and touches. The stock touch-only winit backend
/// turned a connected mouse into a finger: every mouse move/click arrived wrapped in
/// `Event::Touch`, so `touch` grew while hovering. With the builder-patched backend the
/// same mouse activity grows `pointer`/`buttons`/`wheel` and leaves `touch` at zero
/// (only real fingers grow it). Reports at most once per second, and only when a counter
/// actually changed.
#[derive(Default)]
struct InputProbe {{
    pointer_moved: u64,
    buttons: u64,
    wheels: u64,
    touches: u64,
    reported: (u64, u64, u64, u64),
    last_report: Option<std::time::Instant>,
}}

impl InputProbe {{
    fn observe(&mut self, raw: &egui::RawInput) {{
        for event in &raw.events {{
            match event {{
                egui::Event::PointerMoved(_) => self.pointer_moved += 1,
                egui::Event::PointerButton {{ .. }} => self.buttons += 1,
                egui::Event::MouseWheel {{ .. }} => self.wheels += 1,
                egui::Event::Touch {{ .. }} => self.touches += 1,
                _ => {{}}
            }}
        }}
        let totals = (self.pointer_moved, self.buttons, self.wheels, self.touches);
        if totals == self.reported {{
            return;
        }}
        let now = std::time::Instant::now();
        if self
            .last_report
            .is_some_and(|last| now.duration_since(last) < std::time::Duration::from_secs(1))
        {{
            return;
        }}
        log::info!(
            "input-probe: pointer={{}} buttons={{}} wheel={{}} touch={{}} (event totals)",
            totals.0, totals.1, totals.2, totals.3
        );
        self.reported = totals;
        self.last_report = Some(now);
    }}
}}

{persist_support}
/// Cleared when the shell shuts down, so a background watcher stops talking to the Activity.
///
/// Android releases the Activity's Java object as soon as the native thread returns, so a
/// watcher that kept asking it for files after that would call into a dead object. The flag is
/// cleared when the shell is dropped, which happens before the native thread returns.
static ALIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

{control_tx_def}struct AndroidShell {{
    inner: {app_type},
    app: AndroidApp,
    modifiers: egui::Modifiers,
    fullscreen_applied: bool,
    probe: InputProbe,
    bars_theme: Option<bool>,
    back_pending: bool,
    closing: bool,
    close_asked: bool,
    _mcp: Option<mcp_http::Endpoint>,{saf_fields}{intent_field}{persist_fields}{staged_fields}
}}

impl AndroidShell {{
    fn new(mut inner: {app_type}, app: AndroidApp{saf_new_params}) -> Self {{
{control_init}        let _mcp = mcp_http::start_mcp_service(
            "{title}".to_string(),
            "{app_version}".to_string(),
            std::sync::Arc::new(|| ALIVE.load(std::sync::atomic::Ordering::Relaxed)),
        );
        let app_clip = app.clone();
        mobile_text_bar::set_clipboard_getter(move || android_clipboard_get(&app_clip));
        let app_clip_set = app.clone();
        mobile_text_bar::set_clipboard_setter(move |text| android_clipboard_set(&app_clip_set, text));
{app_clipboard_init}        {persist_new_body}Self {{ inner, app, modifiers: egui::Modifiers::default(), fullscreen_applied: false, probe: InputProbe::default(), bars_theme: None, back_pending: false, closing: false, close_asked: false, _mcp{saf_new_init}{intent_init}{persist_new_init}{staged_new_init} }}
    }}
{saf_poll_method}{intent_method}{persist_methods}{staged_methods}
    /// Keep the system-bar icon contrast on the app's active theme: egui's light/dark choice is
    /// compared every frame and only a flip reaches Java (`apply_bar_appearance`), where the
    /// edge-to-edge bars get dark icons over a light canvas and light icons over a dark one.
    fn sync_bar_appearance(&mut self, ctx: &egui::Context) {{
        // `Context::style()` is gone in egui 0.36; `theme()` is what selects the active
        // light/dark style, so it is exactly the choice the bars must mirror.
        let light_theme = ctx.theme() == egui::Theme::Light;
        if self.bars_theme == Some(light_theme) {{
            return;
        }}
        self.bars_theme = Some(light_theme);
        apply_bar_appearance(&self.app, light_theme);
    }}

    /// Re-apply sticky immersive mode on the first frame and whenever the window regains focus
    /// (the system restores the bars after a gesture or when another activity closes).
    fn refresh_fullscreen(&mut self, raw: &egui::RawInput) {{
        let refocused = raw
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::WindowFocused(true)));
        if !self.fullscreen_applied || refocused {{
            self.fullscreen_applied = true;
            apply_immersive_fullscreen(&self.app);
        }}
    }}

    /// Map Android's system back button onto Escape.
    ///
    /// winit's Android backend consumes `KEYCODE_BACK` (it reports the key as handled), so the
    /// framework's own back handling never runs and the button only reaches egui as
    /// `Key::BrowserBack` — which almost no app listens for. Every app already treats Escape as
    /// "close the dialog / cancel / go back", so the back press is re-labelled as Escape, giving
    /// the button its usual meaning with no app change. The press is also remembered, so
    /// [`AndroidShell::ui`] can finish the app when nothing consumed it (the normal Android
    /// "back exits" behaviour).
    fn map_android_back(&mut self, raw: &mut egui::RawInput) {{
        let mut pressed = false;
        for event in &mut raw.events {{
            if let egui::Event::Key {{ key, pressed: is_pressed, .. }} = event
                && *key == egui::Key::BrowserBack
            {{
                *key = egui::Key::Escape;
                pressed |= *is_pressed;
            }}
        }}
        if pressed {{
            self.back_pending = true;
            log::info!("Android: back press mapped to Escape");
        }}
    }}

    /// The user asked to leave (the Android back button). Let the app have the first word: the
    /// apps that guard against losing work ask first, and only a close they agree to ends the
    /// app. `close_requested` is the desktop's own check — the desktop runs it when its window
    /// closes — so the app opens the same "Do you want to quit without saving unsaved changes?"
    /// prompt it shows on the desktop instead of vanishing with unsaved work.
    fn request_close(&mut self) {{
        if self.closing {{
            return;
        }}
        log::info!("Android: the app asked to close");
{close_guard_code}        self.closing = true;
        finish_activity(&self.app);
    }}

    /// Emit egui `Copy`/`Cut`/`Paste` events for Ctrl/Cmd+C/X/V key presses.
    ///
    /// winit's Android backend never emits `ModifiersChanged`, so egui-winit's own
    /// modifier-based clipboard-shortcut detection never fires there. The shell detects the
    /// shortcuts after [`apply_android_modifiers`] has reconstructed the modifier state and
    /// synthesizes the events egui's widgets (text edits, label selection) expect.
    fn handle_clipboard_shortcuts(&mut self, raw: &mut egui::RawInput) {{
        let mut synthesized: Vec<egui::Event> = Vec::new();
        for event in &raw.events {{
            let egui::Event::Key {{ key, pressed: true, modifiers, .. }} = event else {{ continue }};
            if !modifiers.command && !modifiers.ctrl {{
                continue;
            }}
            match key {{
                egui::Key::C => synthesized.push(egui::Event::Copy),
                egui::Key::X => synthesized.push(egui::Event::Cut),
                egui::Key::V => {{
                    let text = android_clipboard_get(&self.app);
                    if !text.is_empty() {{
                        synthesized.push(egui::Event::Paste(text));
                    }}
                }}
                _ => {{}}
            }}
        }}
        raw.events.extend(synthesized);
    }}
}}

impl Drop for AndroidShell {{
    fn drop(&mut self) {{
        // The Activity is about to go: stop the watcher before the native thread returns.
        ALIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    }}
}}

impl eframe::App for AndroidShell {{
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {{
{ctx_store}{intent_logic}{saf_logic_poll}{persist_logic}{staged_logic}        {inner_logic}
{ime_hide_logic}        self.sync_bar_appearance(ctx);
{interact_cursor_logic}{quit_check}    }}

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {{
        system_fonts::install_system_fonts(ui.ctx());
        {inner_ui}
        mobile_text_bar::show_text_bar(ui.ctx());
        if self.back_pending {{
            self.back_pending = false;
            // A back press was mapped onto Escape. If the app consumed that Escape (a dialog
            // closed, a panel went back), the button did its job; otherwise back means "leave",
            // as it does on Android — which the app may still refuse, by asking first.
            if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {{
                if self.close_asked {{
                    // The app had its say last time (it opened a prompt, which this press just
                    // dismissed); don't ask it again straight away.
                    self.close_asked = false;
                }} else {{
                    self.request_close();
                }}
            }}
        }}
    }}

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {{
        mobile_text_bar::process_raw_input(ctx, raw);
        touch_gestures::process_raw_input(ctx, raw);
        self.probe.observe(raw);
        self.refresh_fullscreen(raw);
        apply_android_modifiers(&mut self.modifiers, raw);
{topbar_modifier_hook}        self.map_android_back(raw);
        self.handle_clipboard_shortcuts(raw);
{persist_hook}        {inner_hook}
    }}
{persist_on_exit}}}
'''

    # Install fonts before the first pass. Some apps (figcraft) lay out text with a named font family
    # ("semibold") that only exists after their theme installs the fonts, and egui applies `set_fonts`
    # at the *next* pass: installing from the app's own first frame is one pass too late, so the frame
    # that first draws with that family panics ("FontFamily::Name(\"semibold\") is not bound to any
    # fonts"). The eframe creator runs before the first pass, so installing there makes the family
    # available from frame one.
    create_hook = {
        "figcraft": "            figcraft_ui_egui::theme::install_fonts(&cc.egui_ctx, false, 0);\n",
        # CADCraft's file formats have to be in place before the app opens anything.
        "cadcraft": "            install_cadcraft_io();\n",
        "materialcraft": "            materialcraft_ui_egui::theme::install(&cc.egui_ctx);\n",
        "solidcraft": """            solidcraft_ui_egui::fonts::setup_bundled_fonts(&cc.egui_ctx);
            if let Some(rs) = &cc.wgpu_render_state {
                let supported = eframe::egui_wgpu::supported_msaa_samples(&rs.adapter, &rs.device, rs.target_format, None);
                let samples = if supported.contains(&4) { 4 } else if supported.contains(&2) { 2 } else { 1 };
                shell.inner.set_wgpu_split(rs, 24, 8, samples);
                solidcraft_ui_egui::gpu::install_picture_renderer(rs);
            }
""",
    }.get(app, "")
    theme_line = (
        "            cc.egui_ctx.set_theme(egui::ThemePreference::System);\n"
    )
    needs_cc = bool(re.search(r"\bcc\b", construct_expr) or re.search(r"\bcc\b", post_construct))
    if needs_cc:
        creator = (
            "Box::new(move |cc| {\n"
            + theme_line
            + create_hook
            + f"            {inner_decl} = {construct_expr};\n"
            + f"{post_construct}"
            + "            let shell = AndroidShell::new(inner, android_app.clone(), saf_bridge.clone_ref(), saf_state.clone(), saf_inbox.clone());\n"
            + "            Ok(Box::new(shell))\n        })"
        )
        outer_construct = f"{saf_setup}    // inner and shell constructed inside eframe creator closure below (requires cc)\n"
    else:
        creator = (
            "Box::new(move |cc| {\n"
            + theme_line
            + create_hook
            + "            Ok(Box::new(shell))\n        })"
        )
        shell_decl = "let mut shell" if (app == "solidcraft" or "shell." in create_hook) else "let shell"
        outer_construct = f"{saf_setup}    {inner_decl} = {construct_expr};\n{post_construct}    {shell_decl} = AndroidShell::new(inner, android_app.clone(), saf_bridge, saf_state, saf_inbox);\n"

    soundcraft_mic = "    saf_bridge.request_microphone_permission();\n" if app == "soundcraft" else ""

    if saf:
        entry = f'''//! Generated Android launcher for {title}.
//! Starts the app's egui UI with Android Storage Access Framework (SAF) file services plus the
//! shared Android platform shell (immersive fullscreen and the Ctrl/Alt modifier-key fix).

{use_block}{platform}{saf_block}{shell}
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: android_activity::AndroidApp) {{
    // A panic's backtrace, to see where the Vulkan surface is created.
    #[allow(unused_unsafe)]
    unsafe {{
        std::env::set_var("RUST_BACKTRACE", "1");
        if let Some(path) = android_app.internal_data_path().or_else(|| android_app.external_data_path()) {{
            let tmp = path.join("tmp");
            let _ = std::fs::create_dir_all(&tmp);
            std::env::set_var("TMPDIR", &tmp);
        }}
    }}
    // Route Rust `log` records to logcat; without this a failed picker/save was invisible.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("{package_name}"),
    );
    log::info!("{{ARTCRAFT_ANDROID_SHELL_MARKER}} ready ({{}})", saf::SAF_MARKER);
    apply_immersive_fullscreen(&android_app);
    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
{persist_env_call}    system_fonts::install_font_env(&android_app);
    let saf_bridge = saf::Bridge::new(&android_app);
    saf_bridge.request_storage_permission();
{soundcraft_mic}    let saf_state = saf_new_state();
    let saf_inbox = SafInbox::default();
{outer_construct}    let mut options = eframe::NativeOptions {{
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("{title}")
            .with_drag_and_drop(true),
        android_app: Some(android_app.clone()),
        ..Default::default()
    }};
    // Initialize egui theme from Android system UI mode (light/dark preference)
    let dark_mode = {{
        use jni::{{JavaVM, jni_sig, jni_str, objects::JObject}};
        let app_clone = android_app.clone();
        let vm = unsafe {{ JavaVM::from_raw(app_clone.vm_as_ptr() as *mut _) }};
        vm.attach_current_thread(|env| -> jni::errors::Result<bool> {{
            let activity_raw = app_clone.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe {{ env.as_cast_raw::<JObject>(&activity_raw)? }};
            let resources = env.call_method(activity, jni_str!("getResources"), jni_sig!("()Landroid/content/res/Resources;"), &[])?
                .l()?;
            let config = env.call_method(&resources, jni_str!("getConfiguration"), jni_sig!("()Landroid/content/res/Configuration;"), &[])?
                .l()?;
            let ui_mode = env.get_field(&config, jni_str!("uiMode"), jni_sig!("I"))?.i()?;
            // UI_MODE_NIGHT_YES = 0x20, UI_MODE_NIGHT_MASK = 0x30
            let is_dark = (ui_mode & 0x30) == 0x20;
            Ok(is_dark)
        }}).unwrap_or(false)
    }};

    // Vulkan first with a GLES fallback for devices without a usable Vulkan driver, and the
    // adapter's own texture/buffer sizes: egui's 8192 px default cap sends documents wider or
    // taller than that to the CPU compositor (the desktop apps do this in their gpu_canvas).
    options.wgpu_options.surface.desired_maximum_frame_latency = Some(2);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {{
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
        create.instance_descriptor.flags = eframe::wgpu::InstanceFlags::empty();
        create.power_preference = eframe::wgpu::PowerPreference::HighPerformance;
        create.device_descriptor = std::sync::Arc::new(|adapter: &eframe::wgpu::Adapter| {{
            let supported = adapter.limits();
            let base = if adapter.get_info().backend == eframe::wgpu::Backend::Gl {{
                eframe::wgpu::Limits::downlevel_webgl2_defaults()
            }} else {{
                eframe::wgpu::Limits::default()
            }};
            let base = base.or_worse_values_from(&supported);
            eframe::wgpu::DeviceDescriptor {{
                label: Some("{title} wgpu device"),
                required_limits: eframe::wgpu::Limits {{
                    max_texture_dimension_2d: supported.max_texture_dimension_2d.min(8192),
                    max_buffer_size: supported.max_buffer_size,
                    ..base
                }},
                ..Default::default()
            }}
        }});
    }}
    let result = eframe::run_native(
        "{title}",
        options,
        {creator},
    );
    if let Err(error) = result {{
        log::error!("{title} Android exited with an error: {{error}}");
    }}
}}
'''
    else:
        entry = f'''//! Generated Android launcher for {title}.
//! Starts the app's egui UI with default services plus the shared Android platform shell
//! (immersive fullscreen and the Ctrl/Alt modifier-key fix).

{use_block}{platform}{saf_block}{shell}
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: android_activity::AndroidApp) {{
    // A panic's backtrace, to see where the Vulkan surface is created.
    #[allow(unused_unsafe)]
    unsafe {{
        std::env::set_var("RUST_BACKTRACE", "1");
        if let Some(path) = android_app.internal_data_path().or_else(|| android_app.external_data_path()) {{
            let tmp = path.join("tmp");
            let _ = std::fs::create_dir_all(&tmp);
            std::env::set_var("TMPDIR", &tmp);
        }}
    }}
    // Route Rust `log` records to logcat; without this a failed picker/save was invisible.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("{package_name}"),
    );
    log::info!("{{ARTCRAFT_ANDROID_SHELL_MARKER}} ready ({{}})", saf::SAF_MARKER);
    apply_immersive_fullscreen(&android_app);
    egui_winit::android_activity::set_activity(android_app.activity_as_ptr());
    egui_winit::android_activity::set_vm(android_app.vm_as_ptr());
{persist_env_call}    system_fonts::install_font_env(&android_app);
    let shell_app = android_app.clone();
    let mut options = eframe::NativeOptions {{
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("{title}")
            .with_drag_and_drop(true),
        android_app: Some(android_app.clone()),
        ..Default::default()
    }};
    // Initialize egui theme from Android system UI mode (light/dark preference)
    let dark_mode = {{
        use jni::{{JavaVM, jni_sig, jni_str, objects::JObject}};
        let app_clone = android_app.clone();
        let vm = unsafe {{ JavaVM::from_raw(app_clone.vm_as_ptr() as *mut _) }};
        vm.attach_current_thread(|env| -> jni::errors::Result<bool> {{
            let activity_raw = app_clone.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe {{ env.as_cast_raw::<JObject>(&activity_raw)? }};
            let resources = env.call_method(activity, jni_str!("getResources"), jni_sig!("()Landroid/content/res/Resources;"), &[])?
                .l()?;
            let config = env.call_method(&resources, jni_str!("getConfiguration"), jni_sig!("()Landroid/content/res/Configuration;"), &[])?
                .l()?;
            let ui_mode = env.get_field(&config, jni_str!("uiMode"), jni_sig!("I"))?.i()?;
            let is_dark = (ui_mode & 0x30) == 0x20;
            Ok(is_dark)
        }}).unwrap_or(false)
    }};

    // Vulkan first with a GLES fallback for devices without a usable Vulkan driver, and the
    // adapter's own texture/buffer sizes: egui's 8192 px default cap sends documents wider or
    // taller than that to the CPU compositor (the desktop apps do this in their gpu_canvas).
    options.wgpu_options.surface.desired_maximum_frame_latency = Some(2);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {{
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
        create.instance_descriptor.flags = eframe::wgpu::InstanceFlags::empty();
        create.power_preference = eframe::wgpu::PowerPreference::HighPerformance;
        create.device_descriptor = std::sync::Arc::new(|adapter: &eframe::wgpu::Adapter| {{
            let supported = adapter.limits();
            let base = if adapter.get_info().backend == eframe::wgpu::Backend::Gl {{
                eframe::wgpu::Limits::downlevel_webgl2_defaults()
            }} else {{
                eframe::wgpu::Limits::default()
            }};
            let base = base.or_worse_values_from(&supported);
            eframe::wgpu::DeviceDescriptor {{
                label: Some("{title} wgpu device"),
                required_limits: eframe::wgpu::Limits {{
                    max_texture_dimension_2d: supported.max_texture_dimension_2d.min(8192),
                    max_buffer_size: supported.max_buffer_size,
                    ..base
                }},
                ..Default::default()
            }}
        }});
    }}
    let result = eframe::run_native(
        "{title}",
        options,
        Box::new(move |cc| {{
            cc.egui_ctx.set_theme(egui::ThemePreference::System);
            Ok(Box::new(AndroidShell::new({construct_expr}, shell_app.clone())))
        }}),
    );
    if let Err(error) = result {{
        log::error!("{title} Android exited with an error: {{error}}");
    }}
}}
'''
    (staging_dir / "src").mkdir()

    # The shared Android system-font module, referenced by `mod system_fonts;` in the entry above.
    shutil.copy2(root / "templates" / "android" / "system_fonts.rs", staging_dir / "src" / "system_fonts.rs")
    shutil.copy2(root / "templates" / "android" / text_bar_file, staging_dir / "src" / "mobile_text_bar.rs")
    shutil.copy2(root / "templates" / "android" / "touch_gestures.rs", staging_dir / "src" / "touch_gestures.rs")
    if app in ("subcraft", "tracedraw", "picassosuite", "bimcraft", "earthcraft", "mapcraft", "transit", "anicraft", "papercraft", "ferrite", "pixelcraft", "clothcraft"):
        tg = staging_dir / "src" / "touch_gestures.rs"
        if tg.is_file():
            tg.write_text(re.sub(r'(?m)^\s*source:\s*egui::MouseWheelSource::.*?\n', '', tg.read_text()))
    mcp_code = (root / "templates" / "android" / "mcp_http.rs").read_text()
    suites = {
        "solidcraft": """
struct SuiteMcpServer(solidcraft_mcp::Server);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}
let make: MakeServer = Arc::new(move || {
    let session = solidcraft_engine::Session::default();
    let backend = Box::new(solidcraft_mcp::Headless::new(session));
    Box::new(SuiteMcpServer(solidcraft_mcp::Server::new(backend)))
});
""",
        "cadcraft": """
struct UiControlBackend;
impl cadcraft_mcp::Backend for UiControlBackend {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let tx = {
            let g = crate::CONTROL_TX.lock().map_err(|e| e.to_string())?;
            g.clone().ok_or_else(|| "CadApp UI not initialized".to_string())?
        };
        let (req, pending) = cadcraft_ui_egui::ControlRequest::new(method, params);
        tx.send(req).map_err(|e| format!("failed to send to UI: {e}"))?;
        if let Ok(g) = crate::EGUI_CTX.lock() {
            if let Some(ctx) = g.as_ref() {
                ctx.request_repaint();
            }
        }
        let res = pending.wait(std::time::Duration::from_secs(30));
        if let Some(ok) = res.get("ok").and_then(|v| v.as_bool()) {
            if ok {
                Ok(res.get("result").cloned().unwrap_or(res))
            } else {
                let err = res.get("error").and_then(|v| v.as_str()).unwrap_or("command failed");
                Err(err.to_string())
            }
        } else if let Some(err) = res.get("error").and_then(|v| v.as_str()) {
            Err(err.to_string())
        } else {
            Ok(res)
        }
    }
    fn has_ui(&self) -> bool { true }
    fn describe(&self) -> String { "CADCraft Android Live UI".to_string() }
}
struct SuiteMcpServer(cadcraft_mcp::Server);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}
let make: MakeServer = Arc::new(move || {
    let backend = Box::new(UiControlBackend);
    Box::new(SuiteMcpServer(cadcraft_mcp::Server::new(backend)))
});
""",
        "wordcraft": """
struct UiControlBackend;
impl wordcraft_mcp::Backend for UiControlBackend {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let tx = {
            let g = crate::CONTROL_TX.lock().map_err(|e| e.to_string())?;
            g.clone().ok_or_else(|| "WordCraft UI not initialized".to_string())?
        };
        let (req, rx) = wordcraft_ui_egui::ControlRequest::new(method, params);
        tx.send(req).map_err(|e| format!("failed to send to UI: {e}"))?;
        if let Ok(g) = crate::EGUI_CTX.lock() {
            if let Some(ctx) = g.as_ref() {
                ctx.request_repaint();
            }
        }
        let res = rx.recv_timeout(std::time::Duration::from_secs(30)).map_err(|e| format!("UI timeout: {e}"))?;
        if let Some(ok) = res.get("ok").and_then(|v| v.as_bool()) {
            if ok {
                Ok(res.get("result").cloned().unwrap_or(res))
            } else {
                let err = res.get("error").and_then(|v| v.as_str()).unwrap_or("command failed");
                Err(err.to_string())
            }
        } else if let Some(err) = res.get("error").and_then(|v| v.as_str()) {
            Err(err.to_string())
        } else {
            Ok(res)
        }
    }
    fn has_ui(&self) -> bool { true }
    fn describe(&self) -> String { "WordCraft Android Live UI".to_string() }
}
struct SuiteMcpServer(wordcraft_mcp::Server);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}
let make: MakeServer = Arc::new(move || {
    let backend = Box::new(UiControlBackend);
    Box::new(SuiteMcpServer(wordcraft_mcp::Server::new(backend)))
});
""",
        "figcraft": """
struct UiControlBackend;
impl figcraft_mcp::Backend for UiControlBackend {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let (reply, rx) = std::sync::mpsc::channel();
        let req = figcraft_ui_egui::ControlRequest {
            id: serde_json::json!(1),
            method: method.to_string(),
            params,
            reply,
            state: Default::default(),
        };
        if let Ok(mut q) = crate::CONTROL_QUEUE.lock() {
            q.push(req);
        } else {
            return Err("FigCraft UI queue lock failed".to_string());
        }
        if let Ok(g) = crate::EGUI_CTX.lock() {
            if let Some(ctx) = g.as_ref() {
                ctx.request_repaint();
            }
        }
        let res = rx.recv_timeout(std::time::Duration::from_secs(30)).map_err(|e| format!("UI timeout: {e}"))?;
        res.result
    }
    fn has_ui(&self) -> bool { true }
    fn describe(&self) -> String { "FigCraft Android Live UI".to_string() }
}
struct SuiteMcpServer(figcraft_mcp::Server);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}
let make: MakeServer = Arc::new(move || {
    let backend = Box::new(UiControlBackend);
    Box::new(SuiteMcpServer(figcraft_mcp::Server::new(backend)))
});
""",
        "gridcraft": """
struct UiControlBackend;
impl gridcraft_mcp::Backend for UiControlBackend {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let tx = {
            let g = crate::CONTROL_TX.lock().map_err(|e| e.to_string())?;
            g.clone().ok_or_else(|| "GridCraft UI not initialized".to_string())?
        };
        let (req, rx) = gridcraft_ui_egui::ControlRequest::new(method, params);
        tx.send(req).map_err(|e| format!("failed to send to UI: {e}"))?;
        if let Ok(g) = crate::EGUI_CTX.lock() {
            if let Some(ctx) = g.as_ref() {
                ctx.request_repaint();
            }
        }
        let res = rx.recv_timeout(std::time::Duration::from_secs(30)).map_err(|e| format!("UI timeout: {e}"))?;
        if let Some(ok) = res.get("ok").and_then(|v| v.as_bool()) {
            if ok {
                Ok(res.get("result").cloned().unwrap_or(res))
            } else {
                let err = res.get("error").and_then(|v| v.as_str()).unwrap_or("command failed");
                Err(err.to_string())
            }
        } else if let Some(err) = res.get("error").and_then(|v| v.as_str()) {
            Err(err.to_string())
        } else {
            Ok(res)
        }
    }
    fn has_ui(&self) -> bool { true }
    fn describe(&self) -> String { "GridCraft Android Live UI".to_string() }
}
struct SuiteMcpServer(gridcraft_mcp::Server);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}
let make: MakeServer = Arc::new(move || {
    let backend = Box::new(UiControlBackend);
    Box::new(SuiteMcpServer(gridcraft_mcp::Server::new(backend)))
});
""",
        "deckcraft": """
struct UiControlBackend;
impl deckcraft_mcp::Backend for UiControlBackend {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let tx = {
            let g = crate::CONTROL_TX.lock().map_err(|e| e.to_string())?;
            g.clone().ok_or_else(|| "DeckCraft UI not initialized".to_string())?
        };
        let (req, rx) = deckcraft_ui_egui::ControlRequest::new(method, params);
        tx.send(req).map_err(|e| format!("failed to send to UI: {e}"))?;
        if let Ok(g) = crate::EGUI_CTX.lock() {
            if let Some(ctx) = g.as_ref() {
                ctx.request_repaint();
            }
        }
        let res = rx.recv_timeout(std::time::Duration::from_secs(30)).map_err(|e| format!("UI timeout: {e}"))?;
        if let Some(ok) = res.get("ok").and_then(|v| v.as_bool()) {
            if ok {
                Ok(res.get("result").cloned().unwrap_or(res))
            } else {
                let err = res.get("error").and_then(|v| v.as_str()).unwrap_or("command failed");
                Err(err.to_string())
            }
        } else if let Some(err) = res.get("error").and_then(|v| v.as_str()) {
            Err(err.to_string())
        } else {
            Ok(res)
        }
    }
    fn has_ui(&self) -> bool { true }
    fn describe(&self) -> String { "DeckCraft Android Live UI".to_string() }
}
struct SuiteMcpServer(deckcraft_mcp::Server);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}
let make: MakeServer = Arc::new(move || {
    let backend = Box::new(UiControlBackend);
    Box::new(SuiteMcpServer(deckcraft_mcp::Server::new(backend)))
});
""",
        "designcraft": """
struct UiControlBackend;
impl designcraft_mcp::Backend for UiControlBackend {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let tx = {
            let g = crate::CONTROL_TX.lock().map_err(|e| e.to_string())?;
            g.clone().ok_or_else(|| "DesignCraft UI not initialized".to_string())?
        };
        let (req, rx) = designcraft_ui_egui::ControlRequest::new(method, params);
        tx.send(req).map_err(|e| format!("failed to send to UI: {e}"))?;
        if let Ok(g) = crate::EGUI_CTX.lock() {
            if let Some(ctx) = g.as_ref() {
                ctx.request_repaint();
            }
        }
        let res = rx.recv_timeout(std::time::Duration::from_secs(30)).map_err(|e| format!("UI timeout: {e}"))?;
        if let Some(ok) = res.get("ok").and_then(|v| v.as_bool()) {
            if ok {
                Ok(res.get("result").cloned().unwrap_or(res))
            } else {
                let err = res.get("error").and_then(|v| v.as_str()).unwrap_or("command failed");
                Err(err.to_string())
            }
        } else if let Some(err) = res.get("error").and_then(|v| v.as_str()) {
            Err(err.to_string())
        } else {
            Ok(res)
        }
    }
    fn has_ui(&self) -> bool { true }
    fn describe(&self) -> String { "DesignCraft Android Live UI".to_string() }
}
struct SuiteMcpServer(designcraft_mcp::Server);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}
let make: MakeServer = Arc::new(move || {
    let backend = Box::new(UiControlBackend);
    Box::new(SuiteMcpServer(designcraft_mcp::Server::new(backend)))
});
""",
        "lightcraft": """
struct UiControlBackend;
impl lightcraft_mcp::Backend for UiControlBackend {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let tx = {
            let g = crate::CONTROL_TX.lock().map_err(|e| e.to_string())?;
            g.clone().ok_or_else(|| "LightCraft UI not initialized".to_string())?
        };
        let (req, rx) = lightcraft_ui_egui::ControlRequest::new(method, params);
        tx.send(req).map_err(|e| format!("failed to send to UI: {e}"))?;
        if let Ok(g) = crate::EGUI_CTX.lock() {
            if let Some(ctx) = g.as_ref() {
                ctx.request_repaint();
            }
        }
        let res = rx.recv_timeout(std::time::Duration::from_secs(30)).map_err(|e| format!("UI timeout: {e}"))?;
        if let Some(ok) = res.get("ok").and_then(|v| v.as_bool()) {
            if ok {
                Ok(res.get("result").cloned().unwrap_or(res))
            } else {
                let err = res.get("error").and_then(|v| v.as_str()).unwrap_or("command failed");
                Err(err.to_string())
            }
        } else if let Some(err) = res.get("error").and_then(|v| v.as_str()) {
            Err(err.to_string())
        } else {
            Ok(res)
        }
    }
    fn has_ui(&self) -> bool { true }
    fn describe(&self) -> String { "LightCraft Android Live UI".to_string() }
}
struct SuiteMcpServer(lightcraft_mcp::Server);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}
let make: MakeServer = Arc::new(move || {
    let backend = Box::new(UiControlBackend);
    Box::new(SuiteMcpServer(lightcraft_mcp::Server::new(backend)))
});
""",
        "pdfcraft": """
struct SuiteMcpServer(pdfcraft_automation::mcp::McpServer);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}
let make: MakeServer = Arc::new(move || {
    let auto = pdfcraft_automation::Automation::new();
    Box::new(SuiteMcpServer(pdfcraft_automation::mcp::McpServer::new(auto)))
});
""",
        "soundcraft": """
struct UiControlBackend;
impl soundcraft_automation::Backend for UiControlBackend {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let tx = {
            let g = crate::CONTROL_TX.lock().map_err(|e| e.to_string())?;
            g.clone().ok_or_else(|| "SoundCraft UI not initialized".to_string())?
        };
        let (req, rx) = soundcraft_ui_egui::ControlRequest::new(method, params);
        tx.send(req).map_err(|e| format!("failed to send to UI: {e}"))?;
        if let Ok(g) = crate::EGUI_CTX.lock() {
            if let Some(ctx) = g.as_ref() {
                ctx.request_repaint();
            }
        }
        let res = rx.recv_timeout(std::time::Duration::from_secs(30)).map_err(|e| format!("UI timeout: {e}"))?;
        if let Some(ok) = res.get("ok").and_then(|v| v.as_bool()) {
            if ok {
                Ok(res.get("result").cloned().unwrap_or(res))
            } else {
                let err = res.get("error").and_then(|v| v.as_str()).unwrap_or("command failed");
                Err(err.to_string())
            }
        } else if let Some(err) = res.get("error").and_then(|v| v.as_str()) {
            Err(err.to_string())
        } else {
            Ok(res)
        }
    }
    fn has_ui(&self) -> bool { true }
    fn describe(&self) -> String { "SoundCraft Android Live UI".to_string() }
}
struct SuiteMcpServer(soundcraft_automation::mcp::Server<UiControlBackend>);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        let msg: serde_json::Value = serde_json::from_str(line).ok()?;
        let res = self.0.handle(&msg)?;
        serde_json::to_string(&res).ok()
    }
}
let make: MakeServer = Arc::new(move || {
    let backend = UiControlBackend;
    Box::new(SuiteMcpServer(soundcraft_automation::mcp::Server { backend }))
});
""",
        "effectcraft": """
struct SuiteMcpServer(effectcraft_automation::McpServer);
unsafe impl Send for SuiteMcpServer {}
impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}
let make: MakeServer = Arc::new(move || {
    let session = effectcraft_engine::Session::new();
    let backend = effectcraft_automation::Backend::headless(session);
    Box::new(SuiteMcpServer(effectcraft_automation::McpServer::new(backend)))
});
""",
        "filmcraft": """
use filmcraft_automation::FilmcraftMcp;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};

struct SuiteMcpServer {
    rt: tokio::runtime::Runtime,
    client_in: tokio::io::DuplexStream,
    lines: tokio::io::Lines<TokioBufReader<tokio::io::DuplexStream>>,
}

impl SuiteMcpServer {
    fn new() -> Option<Self> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
        let (client_in, server_in) = tokio::io::duplex(65536);
        let (server_out, client_out) = tokio::io::duplex(65536);
        let lines = TokioBufReader::new(client_out).lines();
        let session = filmcraft_engine::Session::default();
        let mcp = FilmcraftMcp::headless(session);
        rt.spawn(async move {
            let _ = mcp.serve_io(server_in, server_out).await;
        });
        Some(Self { rt, client_in, lines })
    }
}

impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        let input = format!("{line}\\n");
        self.rt.block_on(async {
            self.client_in.write_all(input.as_bytes()).await.ok()?;
            self.client_in.flush().await.ok()?;
            self.lines.next_line().await.ok()?
        })
    }
}

let make: MakeServer = Arc::new(move || {
    SuiteMcpServer::new().map(|s| Box::new(s) as Box<dyn McpServer>).unwrap_or_else(|| {
        Box::new(DefaultMcpServer::new(app_name.clone(), app_version.clone()))
    })
});
""",
        "materialcraft": """
use materialcraft_automation::MaterialCraftMcp;
use rmcp::ServiceExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};

struct SuiteMcpServer {
    rt: tokio::runtime::Runtime,
    client_in: tokio::io::DuplexStream,
    lines: tokio::io::Lines<TokioBufReader<tokio::io::DuplexStream>>,
}

impl SuiteMcpServer {
    fn new() -> Option<Self> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
        let (client_in, server_in) = tokio::io::duplex(65536);
        let (server_out, client_out) = tokio::io::duplex(65536);
        let lines = TokioBufReader::new(client_out).lines();
        let session = materialcraft_automation::Headless::default_session();
        let headless = materialcraft_automation::Headless::new(session, materialcraft_automation::Filesystem::TrustedLocal);
        let mcp = MaterialCraftMcp::headless(headless);
        rt.spawn(async move {
            let _ = mcp.serve((server_in, server_out)).await;
        });
        Some(Self { rt, client_in, lines })
    }
}

impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        let input = format!("{line}\\n");
        self.rt.block_on(async {
            self.client_in.write_all(input.as_bytes()).await.ok()?;
            self.client_in.flush().await.ok()?;
            self.lines.next_line().await.ok()?
        })
    }
}

let make: MakeServer = Arc::new(move || {
    SuiteMcpServer::new().map(|s| Box::new(s) as Box<dyn McpServer>).unwrap_or_else(|| {
        Box::new(DefaultMcpServer::new(app_name.clone(), app_version.clone()))
    })
});
""",
        "texturecraft": """
use texturecraft_automation::TexturecraftMcp;
use rmcp::ServiceExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};

struct SuiteMcpServer {
    rt: tokio::runtime::Runtime,
    client_in: tokio::io::DuplexStream,
    lines: tokio::io::Lines<TokioBufReader<tokio::io::DuplexStream>>,
}

impl SuiteMcpServer {
    fn new() -> Option<Self> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
        let (client_in, server_in) = tokio::io::duplex(65536);
        let (server_out, client_out) = tokio::io::duplex(65536);
        let lines = TokioBufReader::new(client_out).lines();
        let session = texturecraft_automation::Headless::default_session().ok()?;
        let headless = texturecraft_automation::Headless::new(session, texturecraft_automation::Filesystem::TrustedLocal);
        let mcp = TexturecraftMcp::headless(headless);
        rt.spawn(async move {
            let _ = mcp.serve((server_in, server_out)).await;
        });
        Some(Self { rt, client_in, lines })
    }
}

impl McpServer for SuiteMcpServer {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        let input = format!("{line}\\n");
        self.rt.block_on(async {
            self.client_in.write_all(input.as_bytes()).await.ok()?;
            self.client_in.flush().await.ok()?;
            self.lines.next_line().await.ok()?
        })
    }
}

let make: MakeServer = Arc::new(move || {
    SuiteMcpServer::new().map(|s| Box::new(s) as Box<dyn McpServer>).unwrap_or_else(|| {
        Box::new(DefaultMcpServer::new(app_name.clone(), app_version.clone()))
    })
});
"""
    }
    default_make = '''    let make: MakeServer = Arc::new(move || {
        Box::new(DefaultMcpServer::new(app_name.clone(), app_version.clone()))
    });'''
    if app in suites:
        mcp_code = mcp_code.replace(default_make, suites[app].strip())
    (staging_dir / "src" / "mcp_http.rs").write_text(mcp_code)
    # The custom IME bridge is removed to allow egui native IME. Do not copy ime_bridge.rs

    # Audio output on Android: the apps get their audio backend from the platform layer (cpal), and
    # their desktop modules are plain cpal code compiled straight from the app's own source into the
    # launcher (no app edit). `mod audio;` / `mod audio_out;` in the entry picks them up.
    # (AUDIO_MODULES is defined above, beside the launcher dependencies it implies.)
    if app in AUDIO_MODULES:
        _, filename, _ = AUDIO_MODULES[app]
        src_audio = src / "apps" / app / "src" / filename
        if src_audio.is_file():
            shutil.copy2(src_audio, staging_dir / "src" / filename)
    (staging_dir / "src" / "lib.rs").write_text(entry)
    (staging_dir / ".artcraft-android-builder-generated").write_text(
        GENERATOR_VERSION + "\n"
        "Generated by artcraft-android-builder. Edit adapters/" + app + "/ for persistent customizations; "
        "do not hand-edit generated source if you want regeneration.\n"
    )
    # Validate the generated manifest before publishing the directory into Cargo's apps/* workspace glob.
    tomllib.loads((staging_dir / "Cargo.toml").read_text())
    staging_dir.rename(android_dir)
except BaseException:
    shutil.rmtree(staging_dir, ignore_errors=True)
    raise
print(f"Generated NativeActivity wrapper for {title}.")
print(f"  Engine crate: {engine_pkg}")
print(f"  UI crate: {ui_pkg}")
print(f"  UI app type: {app_type}")
print(f"  Icon: {icon.relative_to(src) if icon.is_relative_to(src) else icon}")
print("  Orientation: sensor (allows portrait and landscape rotation)")
