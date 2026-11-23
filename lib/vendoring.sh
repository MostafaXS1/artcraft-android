#!/usr/bin/env bash
# Vendoring and crate patching orchestration for ArtCraft Android.
# Extracted from lib/build-common.sh as part of modularization (issue #73).

vendor_dependencies() {
    local src_dir="$ROOT/$SRC"

    # egui-winit
    echo "Vendoring the Android-compatible egui-winit for $TITLE (system clipboard via JNI)..."
    mkdir -p "$src_dir/vendor"
    if [[ "$APP" == "subcraft" || "$APP" == "tracedraw" || "$APP" == "picassosuite" || "$APP" == "bimcraft" || "$APP" == "earthcraft" || "$APP" == "mapcraft" || "$APP" == "transit" || "$APP" == "anicraft" || "$APP" == "papercraft" || "$APP" == "pixelcraft" ]]; then
        rm -rf "$src_dir/vendor/egui-winit"
        cp -a "$ROOT/vendor/egui-winit-0.36" "$src_dir/vendor/egui-winit"
    elif [[ "$APP" == "ferrite" ]]; then
        rm -rf "$src_dir/vendor/egui-winit"
        cp -a "$ROOT/vendor/egui-winit-0.34" "$src_dir/vendor/egui-winit"
    elif [[ "$APP" == "solidcraft" || "$APP" == "pcbcraft" || "$APP" == "fieldcraft" ]]; then
        echo "Skipping builder egui-winit copy for $APP (handled by prepare.sh)"
    else
        rm -rf "$src_dir/vendor/egui-winit"
        cp -a "$ROOT/vendor/egui-winit" "$src_dir/vendor/egui-winit"
    fi
    if [[ "$APP" == "solidcraft" || "$APP" == "pcbcraft" || "$APP" == "fieldcraft" ]]; then
        echo "Skipping egui patching for $APP (handled by prepare.sh)"
    else
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$src_dir/Cargo.toml" egui-winit auto "https://github.com/storytold/egui.git"
    fi

    # rfd
    if grep -rqE '\brfd::' "$src_dir"/crates/*/src "$src_dir"/apps/*/src 2>/dev/null || grep -qE '^\s*rfd\s*=' "$src_dir/Cargo.toml" 2>/dev/null || grep -rqE '\brfd\b' "$ROOT/adapters/$APP" 2>/dev/null; then
        echo "Vendoring an Android-compatible rfd for $TITLE (native file dialogs become no-ops)..."
        mkdir -p "$src_dir/vendor"
        rm -rf "$src_dir/vendor/rfd"
        cp -a "$ROOT/vendor/rfd" "$src_dir/vendor/rfd"
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$src_dir/Cargo.toml" rfd "vendor/rfd"
    fi

    # arboard
    if grep -rqE '\barboard\b' "$src_dir"/crates "$src_dir"/apps "$src_dir"/src "$src_dir"/Cargo.lock "$src_dir"/Cargo.toml 2>/dev/null; then
        echo "Vendoring an Android-compatible arboard for $TITLE (clipboard access becomes a no-op)..."
        mkdir -p "$src_dir/vendor"
        rm -rf "$src_dir/vendor/arboard"
        cp -a "$ROOT/vendor/arboard" "$src_dir/vendor/arboard"
        python3 "$ROOT/lib/cargo_patches.py" patch-arboard "$src_dir/Cargo.toml" "$src_dir"
    fi

    # midir
    if [[ -d "$ROOT/vendor/midir" ]] && (grep -rqE '\bmidir\b' "$src_dir"/crates "$src_dir"/apps "$src_dir"/src 2>/dev/null || grep -qE '^\s*midir\s*=' "$src_dir/Cargo.toml" 2>/dev/null); then
        echo "Vendoring an Android-compatible midir for $TITLE (dummy MIDI backend)..."
        mkdir -p "$src_dir/vendor"
        rm -rf "$src_dir/vendor/midir"
        cp -a "$ROOT/vendor/midir" "$src_dir/vendor/midir"
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$src_dir/Cargo.toml" midir "vendor/midir"
    fi

    # rustls-platform-verifier
    if [[ -d "$ROOT/vendor/rustls-platform-verifier" ]] && (grep -rqE '\brustls-platform-verifier\b' "$src_dir/Cargo.lock" "$src_dir"/Cargo.toml 2>/dev/null); then
        echo "Vendoring an Android-compatible rustls-platform-verifier for $TITLE (WebPKI Mozilla root certs)..."
        mkdir -p "$src_dir/vendor"
        rm -rf "$src_dir/vendor/rustls-platform-verifier"
        cp -a "$ROOT/vendor/rustls-platform-verifier" "$src_dir/vendor/rustls-platform-verifier"
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$src_dir/Cargo.toml" rustls-platform-verifier "vendor/rustls-platform-verifier"
    fi

    # winit
    echo "Vendoring the mouse/scroll-aware winit for $TITLE..."
    mkdir -p "$src_dir/vendor"
    if [[ -d "$src_dir/vendor/winit" ]]; then
        rm -rf "$src_dir/vendor/winit/src/platform_impl/android"
        cp -a "$ROOT/vendor/winit/src/platform_impl/android" "$src_dir/vendor/winit/src/platform_impl/android"
    else
        rm -rf "$src_dir/vendor/winit"
        cp -a "$ROOT/vendor/winit" "$src_dir/vendor/winit"
    fi

    python3 "$ROOT/lib/cargo_patches.py" patch-winit-ime "$src_dir"

    # egui_dock
    if grep -rqE '\begui_dock\b' "$src_dir"/crates/*/src "$src_dir"/apps/*/src 2>/dev/null; then
        echo "Vendoring egui_dock against the vendored egui for $TITLE..."
        mkdir -p "$src_dir/vendor"
        rm -rf "$src_dir/vendor/egui_dock"
        cp -a "$ROOT/vendor/egui_dock" "$src_dir/vendor/egui_dock"
        python3 "$ROOT/lib/cargo_patches.py" patch-egui-dock "$src_dir"
    fi

    # accesskit_winit in subcraft
    python3 "$ROOT/lib/cargo_patches.py" patch-subcraft-accesskit "$src_dir"
    python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$src_dir/Cargo.toml" winit "vendor/winit"

    # egui-wgpu
    echo "Vendoring the surface-fixing egui-wgpu for $TITLE..."
    if [[ ("$APP" == "solidcraft" || "$APP" == "fieldcraft" || "$APP" == "pcbcraft") && (-d "$src_dir/vendor/egui/crates/egui-wgpu" || -d "$src_dir/vendor/egui-wgpu") ]]; then
        local wgpu_target="$src_dir/vendor/egui/crates/egui-wgpu"
        [[ -d "$wgpu_target" ]] || wgpu_target="$src_dir/vendor/egui-wgpu"
        python3 "$ROOT/lib/cargo_patches.py" patch-egui-wgpu-surface "$wgpu_target"
    elif [[ "$APP" == "solidcraft" || "$APP" == "fieldcraft" || "$APP" == "pcbcraft" || "$APP" == "ferrite" ]]; then
        echo "Skipping builder egui-wgpu copy for $APP (handled by prepare.sh)"
    elif [[ "$APP" == "subcraft" || "$APP" == "tracedraw" || "$APP" == "picassosuite" || "$APP" == "bimcraft" || "$APP" == "earthcraft" || "$APP" == "mapcraft" || "$APP" == "transit" || "$APP" == "anicraft" || "$APP" == "papercraft" || "$APP" == "pixelcraft" ]]; then
        rm -rf "$src_dir/vendor/egui-wgpu"
        cp -a "$ROOT/vendor/egui-wgpu-0.36" "$src_dir/vendor/egui-wgpu"
    else
        rm -rf "$src_dir/vendor/egui-wgpu"
        cp -a "$ROOT/vendor/egui-wgpu" "$src_dir/vendor/egui-wgpu"
    fi

    if [[ "$APP" != "solidcraft" && "$APP" != "fieldcraft" && "$APP" != "pcbcraft" && "$APP" != "ferrite" ]]; then
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$src_dir/Cargo.toml" egui-wgpu "vendor/egui-wgpu" "https://github.com/storytold/egui.git"
    fi

    # android-activity
    echo "Vendoring the ANR-safe android-activity glue for $TITLE..."
    rm -rf "$src_dir/vendor/android-activity"
    cp -a "$ROOT/vendor/android-activity" "$src_dir/vendor/android-activity"
    python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$src_dir/Cargo.toml" android-activity "vendor/android-activity"

    # sys-locale
    if grep -rqE '\bsys_locale\b' "$src_dir"/crates/*/src "$src_dir"/apps/*/src 2>/dev/null || grep -qE '^\s*sys-locale\s*=' "$src_dir/Cargo.toml" 2>/dev/null; then
        echo "Vendoring an Android SELinux-safe sys-locale for $TITLE..."
        mkdir -p "$src_dir/vendor"
        rm -rf "$src_dir/vendor/sys-locale"
        cp -a "$ROOT/vendor/sys-locale" "$src_dir/vendor/sys-locale"
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$src_dir/Cargo.toml" sys-locale "vendor/sys-locale"
    fi

    # cleanup nested target/
    rm -rf "$src_dir/vendor"/*/target

    # ensure workspace dependencies
    python3 "$ROOT/lib/cargo_patches.py" ensure-workspace-deps "$src_dir" "$APP"

    # strip winit wayland features
    python3 "$ROOT/lib/cargo_patches.py" strip-winit-wayland "$src_dir"
}
