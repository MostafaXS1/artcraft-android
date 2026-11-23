#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-clothcraft}"
TITLE="${2:-ClothCraft}"
SRC="${3:-source/clothcraft}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/clothcraft"
DEST="$SRC_ABS/apps/$APP-android"

# 1. Update root Cargo.toml to add workspace members and pin rfd to 0.17
python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys
p = Path(sys.argv[1])
s = p.read_text()
s = re.sub(r'(?m)^(\s*rfd\s*=\s*\{.*?version\s*=\s*)"[^"]+"', r'\1"0.17"', s)
s = re.sub(r'(?m)^(\s*rfd\s*=\s*)"[^"]+"', r'\1"0.17"', s)
if "[workspace]" not in s:
    s = '[workspace]\nmembers = [".", "apps/clothcraft-android"]\n\n' + s
else:
    m = re.search(r'(?ms)^members\s*=\s*\[(.*?)\]', s)
    if m:
        inner = m.group(1)
        if '"apps/clothcraft-android"' not in inner:
            new_inner = inner.rstrip()
            if not new_inner.endswith(','):
                new_inner += ','
            new_inner += ' "apps/clothcraft-android"'
            s = s[:m.start(1)] + new_inner + s[m.end(1):]
p.write_text(s)
PY

# 2. Expose atelier module in src/lib.rs
python3 - "$SRC_ABS/src/lib.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
lines = p.read_text().splitlines(keepends=True)
modified = False
if not any("extern crate self as marvelous_cloth;" in l for l in lines):
    # Insert after the module doc comment
    idx = 1 if lines and lines[0].startswith("//!") else 0
    lines.insert(idx, "extern crate self as marvelous_cloth;\n")
    modified = True
if not any("pub mod atelier;" in l for l in lines):
    lines.append("\n#[cfg(feature = \"desktop\")]\npub mod atelier;\n")
    modified = True
if modified:
    p.write_text("".join(lines))
PY

# 3. Add create_atelier helper in src/atelier.rs
python3 - "$SRC_ABS/src/atelier.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
if "pub fn create_atelier(" not in s:
    helper = """
pub fn create_atelier(
    cc: &eframe::CreationContext<'_>,
    project: FittingProject,
    scene: FittingScene,
    output: PathBuf,
    source: Option<PathBuf>,
    modified: bool,
) -> Result<Box<dyn eframe::App>, String> {
    let render = cc.wgpu_render_state.clone().ok_or("wgpu unavailable")?;
    let prefs = settings::load(&output.join(settings::FILE_NAME));
    settings::apply(&cc.egui_ctx, &prefs);
    let (wire, seam_links) = (prefs.wireframe, prefs.seam_links);
    let (dt, iterations) = (prefs.dt, prefs.iterations);
    let simulation = project.simulation.clone().unwrap_or(VbdSettings {
        dt,
        iterations,
        ..Default::default()
    });
    let mut authoring = AuthoringUi {
        tool: Tool::Edit,
        ..Default::default()
    };
    authoring.board.snap_mm = prefs.snap_mm;
    let (tx, receive) = worker::start(render.device.clone(), render.queue.clone()).map_err(|e| e.to_string())?;
    let m = project
        .template
        .panels
        .iter()
        .find_map(|p| p.polygon.as_ref())
        .map(|p| p.meshing_settings())
        .unwrap_or_default();
    Ok(Box::new(Atelier {
        document: documents_ui::DocumentUi::new(&output, source, modified),
        issue: Default::default(),
        quick: Default::default(),
        authoring,
        prefs,
        settings_open: false,
        prefs_dirty: false,
        inspector_tab: chrome::InspectorTab::Piece,
        authoring_smoke: None,
        mesh_mm: m.target_edge_m * 1000.,
        anisotropy: m.anisotropy,
        mesh_axis: m.axis_degrees,
        project,
        smoke_initial: Vec::new(),
        scene,
        output,
        viewport: Viewport::new(render),
        palette: Default::default(),
        profiles: Default::default(),
        tools_visible: true,
        camera: camera(),
        send: tx,
        receive,
        generation: 0,
        pending: false,
        playing: false,
        wire,
        heat: false,
        seams: false,
        seam_links,
        clean_view: false,
        gpu: true,
        settings: simulation,
        metrics: None,
        cadence: Default::default(),
        simulation_error: None,
        status: "Ready · Space to start the simulation".into(),
        grab: None,
        grab_radius_px: 30.,
        grab_strength: 1.,
        drag_plane: None,
        smoke: false,
        perf: None,
        drag_test: false,
        frames: 0,
        smoke_point: None,
        smoke_tick: 0,
        smoke_started: std::time::Instant::now(),
        smoke_grabbed: false,
        smoke_applied: false,
        smoke_captured: false,
    }))
}
"""
    p.write_text(s + "\n" + helper)
PY

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"
sed -i '/source: egui::MouseWheelSource::/d' "$DEST/src/touch_gestures.rs" 2>/dev/null || true
[[ -f "$ADAPTER/src/mcp_http.rs" ]] || cp -f "$ROOT/templates/android/mcp_http.rs" "$DEST/src/mcp_http.rs"

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/clothcraft.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed ClothCraft adapter in $DEST"
