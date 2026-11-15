# Native clipboard image probe

`clipboard_image_probe.rs` is an opt-in Linux Wayland integration probe for the real
`egui_winit::State` clipboard path. It needs a private compositor; do not run it
against a user session.

## Required example dependencies

The manifest includes this Linux development dependency:

```toml
[target.'cfg(target_os = "linux")'.dev-dependencies]
softbuffer = { version = "0.4.8", default-features = false, features = [
  "wayland",
  "wayland-dlopen",
] }
```

The example also uses the existing optional `image` dependency (`clipboard-image`
feature, PNG only).

## Build

```sh
cargo build -p egui-winit --example clipboard_image_probe --no-default-features \
  --features wayland,clipboard-image,winit/rwh_06
```

## Run against the pinned private Sway tools

```sh
CARGO_TARGET_DIR=/path/to/managed/target \
python3 crates/egui-winit/examples/clipboard_image_smoke.py \
  --evidence /absolute/durable/evidence/dir
```

The controller creates a fresh `XDG_RUNTIME_DIR`, starts headless Sway from
`/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/native-tools/root`,
and drives the probe with `wtype`, `wl-copy`, and `wl-paste`. It never reads or
writes the user's clipboard. `result.json`, logs, the offered bytes, the actually
copied PNG, verification output, source hashes, and the executable hash are kept in
a fresh run directory under `--evidence`. The parent may set `SOURCE_IDENTITY` or
`EXPECTED_SOURCE_SHA`; the controller records it verbatim alongside its own file
hashes and does not run Git.

`clipboard_image_probe --verify-png FILE` decodes a copied PNG and verifies its
exact 2x2 straight-alpha RGBA, alpha channel, and seed match.

`clipboard_image_probe --verify-window FILE` checks that a compositor screenshot
contains all three painted color swatches. The controller waits for this check
before accepting the visible window, and requires clipboard worker repaint wakes.
