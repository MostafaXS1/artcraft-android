#!/usr/bin/env bash
# Asset embedding, native compilation, and Gradle packaging for ArtCraft Android.
# Extracted from lib/build-common.sh as part of modularization (issue #73).

embed_craft_assets() {
    # Craft fonts
    local craft_fonts_dir="$ROOT/source/craft-fonts"
    if [[ -f "$craft_fonts_dir/fonts/manifest.txt" ]]; then
        echo "Reusing the craft-fonts checkout in $craft_fonts_dir"
    elif [[ -e "$craft_fonts_dir" ]]; then
        if [[ -d "$craft_fonts_dir" && -z "$(ls -A "$craft_fonts_dir" 2>/dev/null)" ]]; then
            rmdir "$craft_fonts_dir" 2>/dev/null || true
        fi
        if [[ -e "$craft_fonts_dir" ]]; then
            echo "The craft-fonts checkout at $craft_fonts_dir has no fonts/manifest.txt; leaving it alone." >&2
        fi
    fi
    if [[ ! -e "$craft_fonts_dir" ]]; then
        echo "Fetching craft-fonts (the shared faces the apps embed)..."
        git clone --depth 1 https://github.com/storytold/craft-fonts.git "$craft_fonts_dir" || {
            echo "Could not fetch craft-fonts; the apps will build without the shared faces." >&2
        }
    fi
    if [[ -f "$craft_fonts_dir/fonts/manifest.txt" ]]; then
        export CRAFT_FONTS_DIR="$craft_fonts_dir"
        echo "  -> embedding the craft-fonts faces ($(grep -cve '^#' -e '^$' "$craft_fonts_dir/fonts/manifest.txt") of them)"
    fi

    # Craft i18n
    local craft_i18n_dir="$ROOT/source/craft-i18n"
    if [[ -d "$craft_i18n_dir/locales" ]]; then
        echo "Reusing the craft-i18n checkout in $craft_i18n_dir"
    elif [[ -e "$craft_i18n_dir" ]]; then
        if [[ -d "$craft_i18n_dir" && -z "$(ls -A "$craft_i18n_dir" 2>/dev/null)" ]]; then
            rmdir "$craft_i18n_dir" 2>/dev/null || true
        fi
        if [[ -e "$craft_i18n_dir" ]]; then
            echo "The craft-i18n checkout at $craft_i18n_dir has no locales/; leaving it alone." >&2
        fi
    fi
    if [[ ! -e "$craft_i18n_dir" ]]; then
        echo "Fetching craft-i18n (the shared translations the apps embed)..."
        git clone --depth 1 https://github.com/storytold/craft-i18n.git "$craft_i18n_dir" || {
            echo "Could not fetch craft-i18n; the apps will build without the shared translations." >&2
        }
    fi
    if [[ -d "$craft_i18n_dir/locales" ]]; then
        export CRAFT_I18N_DIR="$craft_i18n_dir"
        echo "  -> embedding the craft-i18n catalogs"
    fi

    # PdfCraft OCR models
    if [[ "$APP" == "pdfcraft" ]]; then
        local models_dir="$ROOT/$SRC/assets/models"
        if [[ ! -f "$models_dir/text-detection.onnx" || ! -f "$models_dir/text-recognition.onnx" ]]; then
            echo "Fetching OCR models for PdfCraft..."
            mkdir -p "$models_dir"
            curl -sSL "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx" -o "$models_dir/text-detection.onnx" || true
            curl -sSL "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx" -o "$models_dir/text-recognition.onnx" || true
        fi
        if [[ -f "$models_dir/text-detection.onnx" && -f "$models_dir/text-recognition.onnx" ]]; then
            echo "  -> packaging OCR models for PdfCraft"
        fi
    fi

    # Contributor credits
    if [[ -f "$ROOT/data/contributors/$APP.json" && -d "$ROOT/$SRC/contributors" ]]; then
        echo "  -> embedding refreshed contributor snapshot from data/contributors/$APP.json"
        cp "$ROOT/data/contributors/$APP.json" "$ROOT/$SRC/contributors/contributors.json"
    fi
}

build_native_library() {
    # Generate AndroidManifest.xml, themes.xml and Gradle project
    echo "  -> generating AndroidManifest.xml, the theme resource and the Gradle project"
    python3 "$ROOT/lib/android_project.py" "$ROOT" "$ANDROID_DIR" "$APP" "$APP_VERSION"

    # shellcheck source=/dev/null
    source "$ANDROID_DIR/.artcraft-gradle.properties"

    echo "  -> pre-fetching Cargo dependencies"
    cargo fetch --manifest-path "$ANDROID_DIR/Cargo.toml" || true

    echo "  -> patching wgpu for Adreno Vulkan swapchain support across the suite"
    python3 "$ROOT/lib/cargo_patches.py" patch-wgpu-registry

    if [[ "$MODE" == "release" ]]; then
        local var_sha="${APP_UPPER}_BUILD_SHA"
        if [[ -z "${!var_sha:-}" && -d "$ROOT/$SRC/.git" ]]; then
            local app_commit_sha
            app_commit_sha="$(git -C "$ROOT/$SRC" rev-parse HEAD 2>/dev/null || true)"
            if [[ -n "$app_commit_sha" ]]; then
                export "${APP_UPPER}_BUILD_SHA"="$app_commit_sha"
                export "${APP_UPPER}_BUILD_DATE"="$(date -u +%Y-%m-%d)"
                export BUILD_SHA="$app_commit_sha"
                export BUILD_DATE="$(date -u +%Y-%m-%d)"
            fi
        fi
    fi

    export RUST_FONTCONFIG_DLOPEN=1
    echo "  -> building the native library with cargo-ndk (arm64-v8a, API $MIN_SDK)"
    rm -rf "$ANDROID_DIR/jniLibs"

    # For apps using path dependencies from workspace root
    if [[ "$APP" == "solidcraft" || "$APP" == "pcbcraft" || "$APP" == "fieldcraft" || "$APP" == "ferrite" || "$APP" == "papercraft" || "$APP" == "pixelcraft" ]]; then
        local ndk_package="$APP-android"
        local ndk_manifest="$ROOT/$SRC/Cargo.toml"
        if [[ "$MODE" == "release" ]]; then
            if ! cargo ndk -t arm64-v8a -P "$MIN_SDK" -o "$ANDROID_DIR/jniLibs" --manifest-path "$ndk_manifest" build --release -p "$ndk_package" 2>&1 | tee "$ROOT/$LOG"; then
                fail "Release native build failed. Log: $ROOT/$LOG"
            fi
        else
            if ! cargo ndk -t arm64-v8a -P "$MIN_SDK" -o "$ANDROID_DIR/jniLibs" --manifest-path "$ndk_manifest" build -p "$ndk_package" 2>&1 | tee "$ROOT/$LOG"; then
                fail "Debug native build failed. Log: $ROOT/$LOG"
            fi
        fi
    else
        if [[ "$MODE" == "release" ]]; then
            if ! cargo ndk -t arm64-v8a -P "$MIN_SDK" -o "$ANDROID_DIR/jniLibs" build --release 2>&1 | tee "$ROOT/$LOG"; then
                fail "Release native build failed. Log: $ROOT/$LOG"
            fi
        else
            if ! cargo ndk -t arm64-v8a -P "$MIN_SDK" -o "$ANDROID_DIR/jniLibs" build 2>&1 | tee "$ROOT/$LOG"; then
                fail "Debug native build failed. Log: $ROOT/$LOG"
            fi
        fi
    fi

    # Bundle libc++_shared.so if needed
    local needs_libcxx
    needs_libcxx="$(find "$ANDROID_DIR/jniLibs" -name "*.so" -exec readelf -d {} + 2>/dev/null | grep 'Shared library: \[libc++_shared\.so\]' || true)"
    if [[ -n "$needs_libcxx" ]]; then
        echo "  -> bundling libc++_shared.so from NDK..."
        local libcxx="$NDK/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so"
        if [[ -f "$libcxx" ]]; then
            mkdir -p "$ANDROID_DIR/jniLibs/arm64-v8a"
            cp -f "$libcxx" "$ANDROID_DIR/jniLibs/arm64-v8a/libc++_shared.so"
        else
            echo "  -> WARNING: the NDK has no libc++_shared.so; the app will not start" >&2
        fi
    fi
}

package_and_sign_apk() {
    local gradle_jdk
    gradle_jdk="$(find_gradle_jdk || true)"
    if [[ -z "$gradle_jdk" ]]; then
        echo "  -> no JDK 17-23 found; downloading Temurin JDK 21 into .cache/jdk"
        gradle_jdk="$(bootstrap_gradle_jdk)"
    fi
    echo "  -> Gradle JDK: $gradle_jdk"

    local gradle_task gradle_variant final_apk
    if [[ "$MODE" == "release" ]]; then
        gradle_task="assembleRelease"
        gradle_variant="release"
        final_apk="$ROOT/output/$APP-$APP_VERSION.apk"
    else
        gradle_task="assembleDebug"
        gradle_variant="debug"
        final_apk="$ROOT/output/$APP-$APP_VERSION-debug.apk"
    fi

    echo "  -> packaging and signing with Gradle ($gradle_task)"
    (
        cd "$ANDROID_DIR/android" && \
        JAVA_HOME="$gradle_jdk" \
        ARTCRAFT_KEYSTORE="$KEY" \
        ARTCRAFT_KEYSTORE_PASSWORD="$KEY_PASSWORD" \
        ARTCRAFT_KEY_ALIAS="artcraftbuilder" \
        ARTCRAFT_KEY_PASSWORD="$KEY_PASSWORD" \
        ./gradlew --console=plain "$gradle_task"
    ) 2>&1 | tee -a "$ROOT/$LOG"
    local gradle_status="${PIPESTATUS[0]}"
    if [[ "$gradle_status" -ne 0 ]]; then
        fail "Gradle $gradle_task failed (exit $gradle_status). Log: $ROOT/$LOG"
    fi

    local apk_dir="$ANDROID_DIR/android/build/outputs/apk/$gradle_variant"
    local built_apk
    built_apk="$(find "$apk_dir" -maxdepth 1 -type f -name '*.apk' ! -name '*-unsigned.apk' 2>/dev/null | head -n1)"
    [[ -n "$built_apk" && -f "$built_apk" ]] || {
        echo "Expected a Gradle APK under $apk_dir" >&2
        echo "Searching generated artifacts:" >&2
        find "$ANDROID_DIR/android/build/outputs" -type f -name '*.apk' -print >&2 || true
        fail "Gradle reported success but the APK artifact was not found. Log: $ROOT/$LOG"
    }
    cp -f "$built_apk" "${final_apk}.tmp" && mv -f "${final_apk}.tmp" "$final_apk"

    say 7 "Verifying the Android file bridge in the built APK..."
    verify_android_bridge "$APP" "$built_apk"

    say 8 "Build succeeded."
    echo "App: $TITLE"
    echo "Mode: $MODE"
    echo "APK: $final_apk"
    echo "Size: $(du -h "$final_apk" | awk '{print $1}')"
    echo "SHA-256: $(sha256sum "$final_apk" | awk '{print $1}')"
    echo "Log: $ROOT/$LOG"
    echo
    echo "This is a locally signed build. Keep keys/artcraft-builder.jks for compatible future updates."
    echo "Public distribution should use a separately managed signing key."
}
