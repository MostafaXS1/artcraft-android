#!/usr/bin/env bash
# Source tree validation, stable tag resolution, and checkout manager for ArtCraft Android.
# Extracted from lib/build-common.sh as part of modularization (issue #73).

# A checkout is only usable when every tracked file of the workspace is actually on disk.
source_tree_is_complete() {
    local dir="$1" missing
    [[ -d "$dir/.git" ]] || return 1
    git -C "$dir" rev-parse --verify HEAD >/dev/null 2>&1 || return 1
    missing="$(git -c core.quotePath=false -C "$dir" ls-tree -r --name-only HEAD -- crates apps 2>/dev/null | while IFS= read -r f; do
        [[ -e "$dir/$f" ]] || printf '%s\n' "$f"
    done | head -5)"
    [[ -z "$missing" ]]
}

# Query the latest published release tag for a repository.
resolve_latest_stable_tag() {
    local origin="$1" repo="$2"
    local tag=""

    # 1. Check GitHub's /releases/latest redirect
    if [[ "$origin" =~ ^https://github\.com/([^/]+)/([^/]+)/?$ ]]; then
        local gh_org="${BASH_REMATCH[1]}"
        local gh_repo="${BASH_REMATCH[2]%.git}"
        local latest_url="https://github.com/$gh_org/$gh_repo/releases/latest"
        local effective_url
        effective_url="$(curl -sIL -o /dev/null -w '%{url_effective}' --connect-timeout 5 --max-time 10 "$latest_url" 2>/dev/null || true)"
        if [[ "$effective_url" =~ /releases/tag/([^/?#]+) ]]; then
            tag="${BASH_REMATCH[1]}"
        fi
    fi

    # 2. Fallback: query git remote tags, excluding pre-releases (-rc, -alpha, -beta, -nightly, etc.)
    # and matching standard semver tags
    if [[ -z "$tag" ]]; then
        tag="$(git ls-remote --tags --sort=-v:refname "$repo" 2>/dev/null | \
            grep -v '\^{}' | \
            awk '{print $2}' | \
            sed 's|refs/tags/||' | \
            grep -E '^v?[0-9]+(\.[0-9]+)+$' | \
            head -n 1 || true)"
    fi

    # 3. Final fallback: traditional v* glob
    if [[ -z "$tag" ]]; then
        tag="$(git ls-remote --tags --sort=-v:refname "$repo" 'v*' 2>/dev/null | \
            grep -v '\^{}' | \
            head -n 1 | \
            awk '{print $2}' | \
            sed 's|refs/tags/||' || true)"
    fi

    printf '%s' "$tag"
}

checkout_app_source() {
    say 3 "Fetching official source for $TITLE..."

    if [[ -e "$SRC" ]]; then
        if [[ -d "$SRC" && ! -d "$SRC/.git" && -z "$(ls -A "$SRC" 2>/dev/null)" ]]; then
            rmdir "$SRC" 2>/dev/null || true
        fi
    fi

    if [[ -e "$SRC" ]]; then
        [[ -d "$SRC/.git" ]] || fail "$SRC exists but is not a Git checkout. Move it aside manually before retrying."
        local origin
        origin="$(git -C "$SRC" remote get-url origin 2>/dev/null || true)"
        case "${origin%.git}" in
            "$APP_ORIGIN") ;;
            *) fail "$SRC has unexpected origin '$origin'; refusing to reuse or overwrite it." ;;
        esac
        if ! source_tree_is_complete "$SRC"; then
            fail "$SRC is an incomplete checkout: some files of the workspace are missing on disk. That happens when a clone is interrupted (a killed build, or a full disk). Move $SRC aside and run the build again to re-clone it."
        fi
        if [[ "$UPDATE_SOURCE" == "true" ]]; then
            echo "Checking for upstream source updates for $TITLE ($REPO)..."
            local latest_ref=""
            if [[ "$BUILD_SOURCE" == "stable" ]]; then
                local latest_tag
                latest_tag="$(resolve_latest_stable_tag "$APP_ORIGIN" "$REPO")"
                if [[ -n "$latest_tag" ]]; then
                    latest_ref="$latest_tag"
                fi
            elif [[ "$BUILD_SOURCE" != "main" && "$BUILD_SOURCE" != "master" && "$BUILD_SOURCE" != "latest" && -n "$BUILD_SOURCE" ]]; then
                latest_ref="$BUILD_SOURCE"
            fi

            local current_tag current_commit is_outdated
            current_tag="$(git -C "$SRC" describe --tags --exact-match 2>/dev/null || true)"
            current_commit="$(git -C "$SRC" rev-parse HEAD 2>/dev/null || true)"
            is_outdated=false

            if [[ -n "$latest_ref" ]]; then
                if [[ "$current_tag" != "$latest_ref" ]]; then
                    is_outdated=true
                fi
            else
                local remote_head
                remote_head="$(git ls-remote "$REPO" HEAD 2>/dev/null | awk '{print $1}')"
                if [[ -n "$remote_head" && "$current_commit" != "$remote_head" ]]; then
                    is_outdated=true
                    latest_ref="$remote_head"
                fi
            fi

            if [[ "$is_outdated" == "true" ]]; then
                echo "Current source (${current_tag:-${current_commit:0:7}}) is outdated compared to upstream ($latest_ref). Updating..."
                git -c filter.lfs.smudge= -c filter.lfs.required=false -c filter.lfs.clean= -C "$SRC" fetch --tags origin >/dev/null 2>&1 || git -c filter.lfs.smudge= -c filter.lfs.required=false -c filter.lfs.clean= -C "$SRC" fetch origin >/dev/null 2>&1
                if git -C "$SRC" checkout -f "$latest_ref" >/dev/null 2>&1; then
                    rm -rf "$SRC/apps/$APP-android"
                    echo "Successfully updated $TITLE source to $latest_ref ($(git -C "$SRC" rev-parse --short HEAD 2>/dev/null || true))."
                else
                    echo "Warning: Could not checkout $latest_ref; continuing with existing source."
                fi
            else
                echo "Current source is up to date (${current_tag:-${current_commit:0:7}})."
            fi
        else
            echo "Skipping update check (--noupdate passed)."
            echo "Reusing existing source checkout."
        fi
        echo "Ref/Commit: $(git -C "$SRC" describe --tags --always 2>/dev/null || git -C "$SRC" rev-parse --short HEAD 2>/dev/null || echo unknown)"
    else
        local src_tmp="$ROOT/source/.clone-$APP.$$"
        rm -rf "$src_tmp"
        local free_bytes
        free_bytes="$(df -k "$ROOT/source" 2>/dev/null | awk 'NR==2 {print $4 * 1024}')"
        if [[ "$free_bytes" =~ ^[0-9]+$ ]] && (( free_bytes < 1048576000 )); then
            rm -rf "$src_tmp"
            fail "Only $((free_bytes / 1048576)) MiB are free on the disk holding $ROOT/source, but cloning $TITLE needs about 1 GiB. Free some space (build/ holds the Cargo output of every app) and run the build again."
        fi

        local target_ref=""
        if [[ "$BUILD_SOURCE" == "stable" ]]; then
            echo "Looking up latest stable release tag for $TITLE ($REPO)..."
            local latest_tag
            latest_tag="$(resolve_latest_stable_tag "$APP_ORIGIN" "$REPO")"
            if [[ -n "$latest_tag" ]]; then
                target_ref="$latest_tag"
                echo "  -> Found latest stable release tag: $target_ref"
            else
                echo "  -> No stable release tag found; falling back to default branch."
            fi
        elif [[ "$BUILD_SOURCE" != "main" && "$BUILD_SOURCE" != "master" && "$BUILD_SOURCE" != "latest" && -n "$BUILD_SOURCE" ]]; then
            target_ref="$BUILD_SOURCE"
            echo "Using target ref: $target_ref"
        else
            echo "Building against upstream main branch."
        fi

        local clone_args=(--depth 1)
        if [[ -n "$target_ref" ]]; then
            clone_args+=(--branch "$target_ref")
        fi

        if ! git -c filter.lfs.smudge= -c filter.lfs.required=false -c filter.lfs.clean= clone "${clone_args[@]}" "$REPO" "$src_tmp"; then
            rm -rf "$src_tmp"
            fail "Cloning $REPO (ref: ${target_ref:-default}) failed. Check the network connection and the free disk space, then run the build again."
        fi
        if ! source_tree_is_complete "$src_tmp"; then
            rm -rf "$src_tmp"
            fail "The clone of $REPO came back incomplete: some files of the workspace are missing on disk. That usually means the disk filled up during the clone. Free some space and run the build again."
        fi
        mv "$src_tmp" "$SRC"
    fi

    # Export upstream release commit SHA and build date for the app being built.
    APP_COMMIT_SHA="$(git -C "$SRC" rev-parse HEAD 2>/dev/null || true)"
    BUILD_DATE="$(date -u +%Y-%m-%d)"
    APP_UPPER="$(echo "$APP" | tr '[:lower:]' '[:upper:]')"
    if [[ "$MODE" == "release" && -n "$APP_COMMIT_SHA" ]]; then
        export "${APP_UPPER}_BUILD_SHA"="$APP_COMMIT_SHA"
        export "${APP_UPPER}_BUILD_DATE"="$BUILD_DATE"
        export BUILD_SHA="$APP_COMMIT_SHA"
        export BUILD_DATE="$BUILD_DATE"
        export GIT_HASH="$APP_COMMIT_SHA"
        export GIT_COMMIT="$APP_COMMIT_SHA"
        echo "  -> Baked release build commit: ${APP_COMMIT_SHA:0:8} (${APP_UPPER}_BUILD_SHA)"
    else
        echo "  -> Development/debug build mode: leaving ${APP_UPPER}_BUILD_SHA unset (will show development build)"
    fi

    # Remove pinned upstream toolchains that might lack Android targets in CI runners
    rm -f "$SRC/rust-toolchain" "$SRC/rust-toolchain.toml"
}
