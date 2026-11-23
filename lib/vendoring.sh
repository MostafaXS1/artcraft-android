#!/usr/bin/env bash
# Vendoring and crate patching orchestration for ArtCraft Android.
# Extracted from lib/build-common.sh as part of modularization (issue #73).

vendor_dependencies() {
    # egui-winit
    echo "Vendoring the Android-compatible egui-winit for $TITLE (system clipboard via JNI)..."
    mkdir -p "$SRC/vendor"
    if [[ "$APP" == "subcraft" || "$APP" == "tracedraw" || "$APP" == "picassosuite" || "$APP" == "bimcraft" || "$APP" == "earthcraft" || "$APP" == "mapcraft" || "$APP" == "transit" || "$APP" == "anicraft" || "$APP" == "papercraft" || "$APP" == "pixelcraft" ]]; then
        rm -rf "$SRC/vendor/egui-winit"
        cp -a "$ROOT/vendor/egui-winit-0.36" "$SRC/vendor/egui-winit"
    elif [[ "$APP" == "ferrite" ]]; then
        rm -rf "$SRC/vendor/egui-winit"
        cp -a "$ROOT/vendor/egui-winit-0.34" "$SRC/vendor/egui-winit"
    elif [[ "$APP" == "solidcraft" || "$APP" == "pcbcraft" || "$APP" == "fieldcraft" ]]; then
        echo "Skipping builder egui-winit copy for $APP (handled by prepare.sh)"
    else
        rm -rf "$SRC/vendor/egui-winit"
        cp -a "$ROOT/vendor/egui-winit" "$SRC/vendor/egui-winit"
    fi
    if [[ "$APP" == "solidcraft" || "$APP" == "pcbcraft" || "$APP" == "fieldcraft" ]]; then
        echo "Skipping egui patching for $APP (handled by prepare.sh)"
    else
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" egui-winit auto "https://github.com/storytold/egui.git"
    fi

    # rfd
    if grep -rqE '\brfd::' "$SRC"/crates/*/src "$SRC"/apps/*/src 2>/dev/null || grep -qE '^\s*rfd\s*=' "$SRC/Cargo.toml" 2>/dev/null || grep -rqE '\brfd\b' "$ROOT/adapters/$APP" 2>/dev/null; then
        echo "Vendoring an Android-compatible rfd for $TITLE (native file dialogs become no-ops)..."
        mkdir -p "$SRC/vendor"
        rm -rf "$SRC/vendor/rfd"
        cp -a "$ROOT/vendor/rfd" "$SRC/vendor/rfd"
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" rfd "vendor/rfd"
    fi

    # arboard
    if grep -rqE '\barboard\b' "$SRC"/crates "$SRC"/apps "$SRC"/src "$SRC"/Cargo.lock "$SRC"/Cargo.toml 2>/dev/null; then
        echo "Vendoring an Android-compatible arboard for $TITLE (clipboard access becomes a no-op)..."
        mkdir -p "$SRC/vendor"
        rm -rf "$SRC/vendor/arboard"
        cp -a "$ROOT/vendor/arboard" "$SRC/vendor/arboard"
        python3 "$ROOT/lib/cargo_patches.py" patch-arboard "$SRC/Cargo.toml" "$SRC"
    fi

    # midir
    if [[ -d "$ROOT/vendor/midir" ]] && (grep -rqE '\bmidir\b' "$SRC"/crates "$SRC"/apps "$SRC"/src 2>/dev/null || grep -qE '^\s*midir\s*=' "$SRC/Cargo.toml" 2>/dev/null); then
        echo "Vendoring an Android-compatible midir for $TITLE (dummy MIDI backend)..."
        mkdir -p "$SRC/vendor"
        rm -rf "$SRC/vendor/midir"
        cp -a "$ROOT/vendor/midir" "$SRC/vendor/midir"
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" midir "vendor/midir"
    fi

    # rustls-platform-verifier
    if [[ -d "$ROOT/vendor/rustls-platform-verifier" ]] && (grep -rqE '\brustls-platform-verifier\b' "$SRC/Cargo.lock" "$SRC"/Cargo.toml 2>/dev/null); then
        echo "Vendoring an Android-compatible rustls-platform-verifier for $TITLE (WebPKI Mozilla root certs)..."
        mkdir -p "$SRC/vendor"
        rm -rf "$SRC/vendor/rustls-platform-verifier"
        cp -a "$ROOT/vendor/rustls-platform-verifier" "$SRC/vendor/rustls-platform-verifier"
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" rustls-platform-verifier "vendor/rustls-platform-verifier"
    fi

    # winit
    echo "Vendoring the mouse/scroll-aware winit for $TITLE..."
    mkdir -p "$SRC/vendor"
    if [[ -d "$SRC/vendor/winit" ]]; then
        rm -rf "$SRC/vendor/winit/src/platform_impl/android"
        cp -a "$ROOT/vendor/winit/src/platform_impl/android" "$SRC/vendor/winit/src/platform_impl/android"
    else
        rm -rf "$SRC/vendor/winit"
        cp -a "$ROOT/vendor/winit" "$SRC/vendor/winit"
    fi

    python3 "$ROOT/lib/cargo_patches.py" patch-winit-ime "$SRC"

    # egui_dock
    if grep -rqE '\begui_dock\b' "$SRC"/crates/*/src "$SRC"/apps/*/src 2>/dev/null; then
        echo "Vendoring egui_dock against the vendored egui for $TITLE..."
        mkdir -p "$SRC/vendor"
        rm -rf "$SRC/vendor/egui_dock"
        cp -a "$ROOT/vendor/egui_dock" "$SRC/vendor/egui_dock"
        python3 "$ROOT/lib/cargo_patches.py" patch-egui-dock "$SRC"
    fi

    # accesskit_winit in subcraft
    python3 "$ROOT/lib/cargo_patches.py" patch-subcraft-accesskit "$SRC"
    python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" winit "vendor/winit"

    # egui-wgpu
    echo "Vendoring the surface-fixing egui-wgpu for $TITLE..."
    if [[ ("$APP" == "solidcraft" || "$APP" == "fieldcraft" || "$APP" == "pcbcraft") && (-d "$SRC/vendor/egui/crates/egui-wgpu" || -d "$SRC/vendor/egui-wgpu") ]]; then
        local wgpu_target="$SRC/vendor/egui/crates/egui-wgpu"
        [[ -d "$wgpu_target" ]] || wgpu_target="$SRC/vendor/egui-wgpu"
        python3 "$ROOT/lib/cargo_patches.py" patch-egui-wgpu-surface "$wgpu_target"
    elif [[ "$APP" == "solidcraft" || "$APP" == "fieldcraft" || "$APP" == "pcbcraft" || "$APP" == "ferrite" ]]; then
        echo "Skipping builder egui-wgpu copy for $APP (handled by prepare.sh)"
    elif [[ "$APP" == "subcraft" || "$APP" == "tracedraw" || "$APP" == "picassosuite" || "$APP" == "bimcraft" || "$APP" == "earthcraft" || "$APP" == "mapcraft" || "$APP" == "transit" || "$APP" == "anicraft" || "$APP" == "papercraft" || "$APP" == "pixelcraft" ]]; then
        rm -rf "$SRC/vendor/egui-wgpu"
        cp -a "$ROOT/vendor/egui-wgpu-0.36" "$SRC/vendor/egui-wgpu"
    else
        rm -rf "$SRC/vendor/egui-wgpu"
        cp -a "$ROOT/vendor/egui-wgpu" "$SRC/vendor/egui-wgpu"
    fi

    if [[ "$APP" != "solidcraft" && "$APP" != "fieldcraft" && "$APP" != "pcbcraft" && "$APP" != "ferrite" ]]; then
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" egui-wgpu "vendor/egui-wgpu" "https://github.com/storytold/egui.git"
    fi

    # android-activity
    echo "Vendoring the ANR-safe android-activity glue for $TITLE..."
    rm -rf "$SRC/vendor/android-activity"
    cp -a "$ROOT/vendor/android-activity" "$SRC/vendor/android-activity"
    python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" android-activity "vendor/android-activity"

    # sys-locale
    if grep -rqE '\bsys_locale\b' "$SRC"/crates/*/src "$SRC"/apps/*/src 2>/dev/null || grep -qE '^\s*sys-locale\s*=' "$SRC/Cargo.toml" 2>/dev/null; then
        echo "Vendoring an Android SELinux-safe sys-locale for $TITLE..."
        mkdir -p "$SRC/vendor"
        rm -rf "$SRC/vendor/sys-locale"
        cp -a "$ROOT/vendor/sys-locale" "$SRC/vendor/sys-locale"
        python3 "$ROOT/lib/cargo_patches.py" patch-crates-io "$SRC/Cargo.toml" sys-locale "vendor/sys-locale"
    fi

    # cleanup nested target/
    rm -rf "$SRC/vendor"/*/target

    # ensure workspace dependencies
    python3 "$ROOT/lib/cargo_patches.py" ensure-workspace-deps "$SRC" "$APP"

    # strip winit wayland features
    python3 "$ROOT/lib/cargo_patches.py" strip-winit-wayland "$SRC"
}
