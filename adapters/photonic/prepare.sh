#!/usr/bin/env bash
set -Eeuo pipefail
APP="${1:-photonic}"
TITLE="${2:-Photonic}"
SRC="${3:-source/photonic}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_ABS="$ROOT/$SRC"
ADAPTER="$ROOT/adapters/photonic"
DEST="$SRC_ABS/apps/$APP-android"

# Update root Cargo.toml to add workspace member and vendor patches
python3 - "$SRC_ABS/Cargo.toml" <<'PY'
from pathlib import Path
import re, sys
p = Path(sys.argv[1])
s = p.read_text()
m = re.search(r'(?ms)^members\s*=\s*\[(.*?)\]', s)
if m:
    inner = m.group(1)
    if '"apps/photonic-android"' not in inner:
        new_inner = inner.rstrip()
        if not new_inner.endswith(','):
            new_inner += ','
        new_inner += ' "apps/photonic-android"'
        s = s[:m.start(1)] + new_inner + s[m.end(1):]
p.write_text(s)
PY

# Patch crates/photonic-gui/Cargo.toml to point rfd to vendor/rfd
python3 - "$SRC_ABS/crates/photonic-gui/Cargo.toml" <<'PY'
from pathlib import Path
import sys, re
p = Path(sys.argv[1])
s = p.read_text()
s = re.sub(r'rfd\s*=\s*"0\.15"', 'rfd = { path = "../../vendor/rfd", default-features = false }', s)
p.write_text(s)
PY

# Stub fastembed in crates/photonic-embed (ort has no Android prebuilt binaries)
python3 - "$SRC_ABS/crates/photonic-embed" <<'PY'
from pathlib import Path
import sys
d = Path(sys.argv[1])
(d / "Cargo.toml").write_text('[package]\nname = "photonic-embed"\nversion.workspace = true\nedition = "2021"\nlicense.workspace = true\n\n[dependencies]\nanyhow = { workspace = true }\n')
(d / "src" / "lib.rs").write_text('''use anyhow::{anyhow, Result};

pub struct Embedder;

impl Embedder {
    pub fn new() -> Result<Self> {
        Err(anyhow!("Semantic search embedding is not supported on Android"))
    }

    pub fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Err(anyhow!("Semantic search embedding is not supported on Android"))
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len().min(b.len()) {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}
''')
PY

# Stub ort in crates/photonic-matte (ort has no Android prebuilt binaries)
python3 - "$SRC_ABS/crates/photonic-matte" <<'PY'
from pathlib import Path
import sys
d = Path(sys.argv[1])
(d / "Cargo.toml").write_text('[package]\nname = "photonic-matte"\nversion.workspace = true\nedition = "2021"\nlicense.workspace = true\n\n[dependencies]\nanyhow = { workspace = true }\nphotonic-core = { path = "../photonic-core" }\n')
(d / "src" / "lib.rs").write_text('''use anyhow::{anyhow, Result};
use photonic_core::raster::{image::RasterImage, mask::Mask};

pub fn model_is_cached() -> bool {
    false
}

pub fn remove_background(img: &RasterImage) -> Result<Mask> {
    if img.width == 0 || img.height == 0 {
        return Err(anyhow!("cannot matte a zero-size image"));
    }
    Err(anyhow!("Background removal is not supported on Android"))
}
''')
PY

# Add [lib] to crates/photonic-app/Cargo.toml and patch arboard and reqwest
python3 - "$SRC_ABS/crates/photonic-app/Cargo.toml" <<'PY'
from pathlib import Path
import sys, re
p = Path(sys.argv[1])
s = p.read_text()
if "[lib]" not in s:
    s += '\n\n[lib]\nname = "photonic_app"\npath = "src/lib.rs"\ncrate-type = ["rlib"]\n'
s = re.sub(r'arboard\s*=\s*\{[^}]*\}', 'arboard = { path = "../../vendor/arboard" }', s)
s = re.sub(r'reqwest\s*=\s*\{[^}]*\}', 'reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "blocking", "json"] }', s)
if "android-activity" not in s:
    s += '\n[target.\'cfg(target_os = "android")\'.dependencies]\nandroid-activity = { version = "0.6", features = ["native-activity"] }\nopenssl = { version = "0.10", features = ["vendored"] }\nopenssl-sys = { version = "0.9", features = ["vendored"] }\n'
p.write_text(s)
PY

# Patch alpha_mode in crates/photonic-render/src/renderer/mod.rs
python3 - "$SRC_ABS/crates/photonic-render/src/renderer/mod.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
old_alpha = '''    let Some(alpha_mode) = caps.alpha_modes.first().copied() else {
        return Err(anyhow!("surface has no compatible alpha modes"));
    };'''
new_alpha = '''    let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Auto) {
        wgpu::CompositeAlphaMode::Auto
    } else if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
        wgpu::CompositeAlphaMode::Opaque
    } else {
        caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto)
    };'''
if old_alpha in s:
    s = s.replace(old_alpha, new_alpha, 1)
    p.write_text(s)
PY

# Add run_android to crates/photonic-app/src/main.rs and create src/lib.rs
python3 - "$SRC_ABS/crates/photonic-app" <<'PY'
from pathlib import Path
import sys, re
app_dir = Path(sys.argv[1])
main_rs = app_dir / "src" / "main.rs"
s = main_rs.read_text()

# Handle suspended and safe capture_rx
if "fn suspended(&mut self" not in s:
    s = s.replace(
        "impl ApplicationHandler for PhotonicWinitApp {",
        "impl ApplicationHandler for PhotonicWinitApp {\n    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {\n        tracing::info!(\"Window suspended — clearing render state\");\n        self.state = None;\n    }"
    )

s = s.replace(
    'let capture_rx = self.capture_rx.take().expect("capture_rx already consumed");',
    'let capture_rx = self.capture_rx.take().unwrap_or_else(|| std::sync::mpsc::channel().1);'
)

# Do not exit on transient surface errors
s = s.replace(
    'error!(%error, "GPU surface failed; closing Photonic cleanly");\n                        event_loop.exit();\n                        return;',
    'error!(%error, "GPU surface error during frame");\n                        return;'
)

# Inject mobile scaling in render_frame
if "ctx.screen_rect().width() < 600.0" not in s:
    s = s.replace(
        "let full_output = egui_ctx.run(raw_input, |ctx| {",
        "let full_output = egui_ctx.run(raw_input, |ctx| {\n            if ctx.screen_rect().width() < 600.0 {\n                ctx.set_zoom_factor(0.85);\n            }"
    )

# Inject run_android
if "#[cfg(target_os = \"android\")]\npub fn run_android" in s:
    s = s.split("#[cfg(target_os = \"android\")]\npub fn run_android")[0].rstrip() + "\n"

run_android_code = '''
#[cfg(target_os = "android")]
pub fn run_android(android_app: android_activity::AndroidApp) -> anyhow::Result<()> {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    let document = Document::default_artboard();
    let document_arc = Arc::new(Mutex::new(document));
    let preferences = photonic_gui::preferences::AppPreferences::load();
    let (hist_steps, hist_size) = preferences.history_limits();
    let mut initial_history = CommandHistory::new(hist_steps);
    initial_history.set_limits(hist_steps, hist_size);
    let history_arc: Arc<Mutex<CommandHistory>> = Arc::new(Mutex::new(initial_history));
    let (capture_tx, capture_rx) = std::sync::mpsc::channel::<oneshot::Sender<Vec<u8>>>();

    let audit_log = Arc::new(std::sync::Mutex::new(AuditLog::new()));
    let mcp_document_path = Arc::new(std::sync::Mutex::new(None));
    let mcp_config = McpServerConfig {
        port: 0,
        secret: None,
        protocol_mode: photonic_mcp::protocol::ProtocolMode::Dual,
    };
    let mcp_running = Arc::new(AtomicBool::new(false));
    let mcp_restart_requested = Arc::new(AtomicBool::new(false));
    let mcp_state = spawn_mcp_server(
        Arc::clone(&document_arc),
        Arc::clone(&history_arc),
        capture_tx.clone(),
        mcp_config.clone(),
        Arc::clone(&mcp_running),
        Arc::clone(&audit_log),
        Arc::clone(&mcp_document_path),
    );

    let mut event_loop_builder = EventLoop::builder();
    event_loop_builder.with_android_app(android_app);
    let event_loop = event_loop_builder.build()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = PhotonicWinitApp {
        document: document_arc,
        history: history_arc,
        mcp_running,
        mcp_restart_requested,
        mcp_capture_tx: capture_tx,
        mcp_config,
        mcp_document_path,
        mcp_state,
        mcp_result_rx: None,
        capture_rx: Some(capture_rx),
        state: None,
        show_welcome: false,
        initial_file: None,
        audit_log,
        window_state: WindowState::default(),
        startup_error: None,
    };

    event_loop.run_app(&mut app)?;
    if let Some(error) = app.startup_error {
        return Err(anyhow::anyhow!(error));
    }
    Ok(())
}
'''
s += "\n" + run_android_code
main_rs.write_text(s)

lib_rs = app_dir / "src" / "lib.rs"
lib_rs.write_text(s)
PY

mkdir -p "$DEST"
cp -a "$ADAPTER/." "$DEST/"
rm -f "$DEST/prepare.sh" "$DEST/README.md"
rm -rf "$DEST/assets"
cp -f "$ROOT/templates/android/system_fonts.rs" "$DEST/src/system_fonts.rs"
python3 - "$DEST/src/system_fonts.rs" <<'PY'
from pathlib import Path
import sys, re
p = Path(sys.argv[1])
s = p.read_text()
s = re.sub(r'(?ms)pub fn install_system_fonts\(ctx: &egui::Context\) \{.*?^\}', 'pub fn install_system_fonts(_ctx: &egui::Context) {}', s)
p.write_text(s)
PY

cp -f "$ROOT/templates/android/mobile_text_bar.rs" "$DEST/src/mobile_text_bar.rs"
python3 - "$DEST/src/mobile_text_bar.rs" <<'PY'
from pathlib import Path
import sys, re
p = Path(sys.argv[1])
s = p.read_text()
s = s.replace("CornerRadius", "Rounding")
s = s.replace("Rounding::same(8)", "Rounding::same(8.0)")
s = s.replace("Rounding::same(5)", "Rounding::same(5.0)")
s = s.replace("ctx.content_rect()", "ctx.input(|i| i.raw.screen_rect).unwrap_or(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 1000.0)))")
s = re.sub(r'ui\.painter\(\)\.rect\(\s*bar_rect,\s*Rounding::same\(8\.0\),\s*bg_color,\s*stroke,\s*egui::StrokeKind::Inside,\s*\);',
           'ui.painter().rect_filled(bar_rect, Rounding::same(8.0), bg_color); ui.painter().rect_stroke(bar_rect, Rounding::same(8.0), stroke);', s)
s = re.sub(r'ui\.painter\(\)\.rect\(\s*r,\s*Rounding::same\(5\.0\),\s*fill,\s*btn_stroke,\s*egui::StrokeKind::Inside,\s*\);',
           'ui.painter().rect_filled(r, Rounding::same(5.0), fill); ui.painter().rect_stroke(r, Rounding::same(5.0), btn_stroke);', s)
p.write_text(s)
PY

cp -f "$ROOT/templates/android/touch_gestures.rs" "$DEST/src/touch_gestures.rs"
sed -i -e '/source: egui::MouseWheelSource::/d' -e '/phase: TouchPhase::/d' "$DEST/src/touch_gestures.rs" 2>/dev/null || true
[[ -f "$ADAPTER/src/mcp_http.rs" ]] || cp -f "$ROOT/templates/android/mcp_http.rs" "$DEST/src/mcp_http.rs"

python3 "$ROOT/lib/app_icons.py" --src "$SRC_ABS" --app "$APP" --dest "$DEST/res" || true
printf 'Generated by artcraft-android-builder; custom adapter sourced from adapters/photonic.\n' > "$DEST/.artcraft-android-builder-generated"

python3 - "$DEST/Cargo.toml" "$ROOT" "$APP" <<'PY'
from pathlib import Path
import sys, tomllib
path, root, app = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sys.path.insert(0, str(root / "lib"))
import file_associations
file_associations.patch_cargo_toml(path, app)
tomllib.loads(path.read_text())
PY
echo "Installed Photonic adapter in $DEST"
