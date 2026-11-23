#!/usr/bin/env bash
# Keystore and signing configuration for ArtCraft Android builder.
# Extracted from lib/build-common.sh as part of modularization (issue #73).

prepare_signing_key() {
    say 5 "Preparing a builder-owned signing key..."
    KEY="$ROOT/keys/artcraft-builder.jks"
    KEY_PASSWORD_FILE="$ROOT/keys/.keystore-password"
    mkdir -p "$ROOT/keys"
    chmod 700 "$ROOT/keys"

    if [[ -n "${ARTCRAFT_KEYSTORE_BASE64:-}" && ! -f "$KEY" ]]; then
        echo "$ARTCRAFT_KEYSTORE_BASE64" | base64 -d > "$KEY"
        chmod 600 "$KEY"
    fi

    if [[ ! -f "$KEY_PASSWORD_FILE" ]]; then
        if [[ -n "${ARTCRAFT_KEYSTORE_PASSWORD:-}" ]]; then
            printf '%s\n' "$ARTCRAFT_KEYSTORE_PASSWORD" > "$KEY_PASSWORD_FILE"
        elif [[ -f "$KEY" ]]; then
            # Compatibility with earlier builder versions, which used this local-only password.
            printf 'android\n' > "$KEY_PASSWORD_FILE"
        else
            python3 -c 'import secrets; print(secrets.token_hex(24))' > "$KEY_PASSWORD_FILE"
        fi
        chmod 600 "$KEY_PASSWORD_FILE"
    fi
    KEY_PASSWORD="$(<"$KEY_PASSWORD_FILE")"
    if [[ ! -f "$KEY" ]]; then
        if [[ -n "${CI:-}" && "$MODE" == "release" && "${GITHUB_REF:-}" =~ ^refs/tags/ ]]; then
            fail "Release tag builds in CI require a persistent release keystore (set ARTCRAFT_KEYSTORE_BASE64 secret) to maintain Android update signature continuity."
        fi
        if [[ -n "${CI:-}" ]]; then
            echo "WARNING: Generating an ephemeral signing key for CI non-tag build. Generated APKs will not be update-compatible with official releases." >&2
        fi
        command -v keytool >/dev/null 2>&1 || fail "keytool is required. Install a JDK (on Fedora: sudo dnf install java-25-openjdk-devel)."
        keytool -genkeypair -noprompt -v \
            -keystore "$KEY" -storetype JKS \
            -storepass "$KEY_PASSWORD" -keypass "$KEY_PASSWORD" \
            -alias artcraftbuilder -keyalg RSA -keysize 2048 -validity 10000 \
            -dname "CN=ArtCraft Android Builder, OU=Local Builds, O=ArtCraft, C=US"
        chmod 600 "$KEY"
    fi
}
