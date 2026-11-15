#!/usr/bin/env bash
# Generate catalog.rs from app-catalog.json.
#
# Run this before building to update the app catalog from the JSON config, or just run
# ./build-launcher.sh from the project root: it regenerates the catalog automatically.
#
# Apps without a "release" block install from the ArtCraft builder's shared release
# (MostafaXS1/artcraft-android, asset named <app-id>-<version>.apk).
#
# Apps from another repository add a release block and (optionally) their Android package:
#
#   "release": {
#     "repo": "bramble-apps/bramble-office",
#     "asset": "Keydeck*apk"
#   },
#   "package": "app.bramble.keydeck"
#
# "asset" is a glob matched against every file in that repository's latest GitHub release;
# "*" matches anything. "package" is the Android application id, used for detecting,
# opening and uninstalling the installed app (defaults to com.artcraft.<id>).
#
# Android only lets an app see packages listed in its manifest <queries>. This script rebuilds
# that list from every app's package plus the optional top-level "query_packages" array, so add
# any extra application ids the launcher should detect there:
#
#   "query_packages": ["com.example.alternate.id"]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CATALOG_JSON="$SCRIPT_DIR/app-catalog.json"
CATALOG_RS="$SCRIPT_DIR/src/catalog.rs"
MANIFEST="$SCRIPT_DIR/android/app/src/main/AndroidManifest.xml"
SHARED_REPO="MostafaXS1/artcraft-android"

if [[ ! -f "$CATALOG_JSON" ]]; then
    echo "error: $CATALOG_JSON not found" >&2
    exit 1
fi
if ! command -v jq >/dev/null; then
    echo "error: jq is required to generate the catalog" >&2
    exit 1
fi
if ! jq -e . "$CATALOG_JSON" >/dev/null; then
    echo "error: $CATALOG_JSON is not valid JSON" >&2
    exit 1
fi

TOTAL_APPS=$(jq '(.official_apps | length) + (.community_apps | length)' "$CATALOG_JSON")

cat > "$CATALOG_RS" << EOF
//! The app catalog the Android launcher installs from.
//!
//! Generated from app-catalog.json by generate-catalog.sh. Do not edit by hand.
//!
//! Every Craft App that has an Android build, plus the community apps the builder ports,
//! in one static list. Text here is English; the UI translates each tagline by app id and
//! falls back to this when it has none.
//!
//! Android APKs come from GitHub releases. Most apps come from the ArtCraft builder's
//! shared release (\`$SHARED_REPO\`) as \`<app-id>-<version>.apk\`. An app can instead name
//! its own repository and the file to look for in its latest release (see
//! \`app-catalog.json\` and \`release_source_for\`).

use artcraft_launcher_core::catalog::{AppKind, CraftApp, ARTCRAFT_TEAM_ID};
use artcraft_launcher_core::release::{Asset, Release};
use artcraft_launcher_core::version::Version;

/// The community-app suite: who publishes it and its own website.
pub const COMMUNITY: AppKind = AppKind::Community;

/// Where one app's Android APK comes from.
///
/// \`asset_pattern\` is \`None\` for the shared builder release, whose files are named
/// \`<app-id>-<version>.apk\`; otherwise it is a glob against the external release's assets.
#[derive(Clone, Copy, Debug)]
pub struct ReleaseSource {
    pub repo: &'static str,
    pub asset_pattern: Option<&'static str>,
}

/// One community app. \`repo\` is the app's own repository for the card's links; the
/// release lookup may come from a different repository (\`release_source_for\`).
#[allow(clippy::too_many_arguments)] // one flat, declarative table row
const fn community(
    id: &'static str,
    name: &'static str,
    tagline: &'static str,
    color: [u8; 3],
    repo: &'static str,
    publisher: &'static str,
    site: Option<&'static str>,
    bundle: Option<&'static str>,
) -> CraftApp {
    CraftApp {
        id,
        name,
        tagline,
        color,
        ink_light: [color[0] / 2, color[1] / 2, color[2] / 2],
        ink_dark: [
            color[0].saturating_add((255 - color[0]) / 2),
            color[1].saturating_add((255 - color[1]) / 2),
            color[2].saturating_add((255 - color[2]) / 2),
        ],
        repo,
        former_ids: &[],
        former_names: &[],
        kind: COMMUNITY,
        publisher,
        msi_publisher: None,
        bundle,
        former_bundles: &[],
        site,
        mac_team: None,
    }
}

/// One official ArtCraft app.
const fn official(
    id: &'static str,
    name: &'static str,
    tagline: &'static str,
    color: [u8; 3],
    repo: &'static str,
) -> CraftApp {
    CraftApp {
        id,
        name,
        tagline,
        color,
        ink_light: [color[0] / 2, color[1] / 2, color[2] / 2],
        ink_dark: [
            color[0].saturating_add((255 - color[0]) / 2),
            color[1].saturating_add((255 - color[1]) / 2),
            color[2].saturating_add((255 - color[2]) / 2),
        ],
        repo,
        former_ids: &[],
        former_names: &[],
        kind: AppKind::Official,
        publisher: "ArtCraft",
        msi_publisher: None,
        bundle: None,
        former_bundles: &[],
        site: None,
        mac_team: Some(ARTCRAFT_TEAM_ID),
    }
}

/// Apps whose APK is not in the shared builder release: app id -> release source.
static RELEASE_OVERRIDES: &[(&str, ReleaseSource)] = &[
EOF

jq -r '
    (.official_apps[], .community_apps[])
    | select(.release != null)
    | "    (\(.id | @json), ReleaseSource { repo: \(.release.repo | @json), asset_pattern: Some(\(.release.asset | @json)) }),"
' "$CATALOG_JSON" >> "$CATALOG_RS"

cat >> "$CATALOG_RS" << EOF
];

/// The Android suite, in the order getartcraft.com lists the official apps, then the
/// community apps. Every entry must have an icon in \`ui-egui\`'s \`app_icon_png\`.
pub static SUITE_ANDROID: [CraftApp; $TOTAL_APPS] = [
EOF

jq -r '
    .official_apps[]
    | "    official(\(.id | @json), \(.name | @json), \(.tagline | @json), [\(.color | join(", "))], \(.repo | @json)),"
' "$CATALOG_JSON" >> "$CATALOG_RS"

jq -r '
    .community_apps[]
    | "    community(\n        \(.id | @json),\n        \(.name | @json),\n        \(.tagline | @json),\n        [\(.color | join(", "))],\n        \(.repo | @json),\n        \(.publisher | @json),\n        \(if .site then "Some(\(.site | @json))" else "None" end),\n        \(if .bundle then "Some(\(.bundle | @json))" else "None" end),\n    ),"
' "$CATALOG_JSON" >> "$CATALOG_RS"

cat >> "$CATALOG_RS" << EOF
];

/// An app by id, from the Android suite.
pub fn app_by_id(id: &str) -> Option<&'static CraftApp> {
    SUITE_ANDROID.iter().find(|a| a.id == id)
}

/// The shared repository most Android APKs come from.
pub const ANDROID_REPO: &str = "$SHARED_REPO";

/// Where this app's APK comes from. Apps without an override use the shared release.
pub fn release_source_for(app: &CraftApp) -> ReleaseSource {
    RELEASE_OVERRIDES
        .iter()
        .find(|(id, _)| *id == app.id)
        .map_or(ReleaseSource { repo: ANDROID_REPO, asset_pattern: None }, |(_, source)| *source)
}

/// The Android package the app's APK installs as. Defaults to \`com.artcraft.<id>\`;
/// \`app-catalog.json\` can override it for apps that already ship under another id.
pub fn package_for(app_id: &str) -> String {
    match app_id {
EOF

jq -r '
    (.official_apps[], .community_apps[])
    | select(.package != null)
    | "        \(.id | @json) => \(.package | @json).to_string(),"
' "$CATALOG_JSON" >> "$CATALOG_RS"

cat >> "$CATALOG_RS" << 'EOF'
        _ => format!("com.artcraft.{app_id}"),
    }
}

/// Matches a release asset name against a simple glob (`*` matches any run of characters,
/// `?` one character). Used by `app-catalog.json`'s `release.asset`.
pub fn asset_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let name: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut resume) = (None, 0);
    while ni < name.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == name[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star = Some(pi);
            pi += 1;
            resume = ni;
        } else if let Some(s) = star {
            pi = s + 1;
            resume += 1;
            ni = resume;
        } else {
            return false;
        }
    }
    pattern[pi..].iter().all(|c| *c == '*')
}

/// Strips trailing packaging tokens that are not part of the version:
/// `0.0.6-beta-android-arm64` → `0.0.6-beta`. Without this, a file name whose suffix happens to
/// be valid pre-release (`beta-android-arm64`) compares newer than the same version installed
/// from the release, so the launcher keeps offering the update that is already in place.
fn strip_packaging_suffix(rest: &str) -> &str {
    let mut s = rest;
    loop {
        let Some((prefix, token)) = s.rsplit_once('-') else { break };
        if is_packaging_token(token) {
            s = prefix;
            continue;
        }
        break;
    }
    s
}

fn is_packaging_token(token: &str) -> bool {
    let t = token.to_ascii_lowercase();
    matches!(
        t.as_str(),
        "android" | "arm64" | "arm32" | "armv6" | "armv7" | "armv8a" | "armeabi" | "aarch64"
            | "x86" | "x86_64" | "x64" | "amd64" | "i386" | "i686"
            | "v7a" | "v8a" | "v9a" | "mips" | "riscv64"
            | "universal" | "binary"
    )
}

/// Reads `<app-id>-<version>.apk` (the builder's shared-release naming): the app it belongs
/// to and its version. `None` for anything else (checksums, other suffixes, unknown apps).
pub fn parse_apk_name(name: &str) -> Option<(&'static str, Version)> {
    let base = name.strip_suffix(".apk")?;
    // Longest id first, so an id that is a prefix of another can never win.
    let mut ids: Vec<&'static str> = SUITE_ANDROID.iter().map(|a| a.id).collect();
    ids.sort_by_key(|id| std::cmp::Reverse(id.len()));
    for id in ids {
        let Some(rest) = base.strip_prefix(id).and_then(|rest| rest.strip_prefix('-')) else { continue };
        if !rest.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        // The version may be followed by architecture tokens (`0.20.2-android-arm64-v8a`): those
        // are stripped from the end first, so the remainder keeps any pre-release suffix whole
        // (`0.0.6-beta`). `0.0.6-beta-android-arm64` must compare equal to the installed
        // `0.0.6-beta`, or the launcher would offer the same update forever.
        let parts: Vec<&str> = strip_packaging_suffix(rest).split('-').collect();
        for n in (1..=parts.len()).rev() {
            if let Some(version) = artcraft_launcher_core::version::parse(&parts[..n].join("-")) {
                return Some((id, version));
            }
        }
    }
    None
}

/// Builds the per-app `Release` the UI shows: the asset found in the GitHub release, with a
/// normalized name `<app>-<version>-android-aarch64.apk` so the download path is stable.
///
/// Shared-release assets carry their version in the file name; external releases (Keydeck,
/// Values, ...) don't, so the release tag's version is the fallback.
pub fn android_release(
    app: &CraftApp,
    tag: &str,
    version: Option<Version>,
    published_at: Option<i64>,
    html_url: String,
    notes: String,
    asset: Asset,
) -> Release {
    let version = asset
        .name
        .strip_suffix(".apk")
        .and_then(|s| s.strip_prefix(&format!("{}-", app.id)))
        .and_then(|s| artcraft_launcher_core::version::parse(strip_packaging_suffix(s)))
        .or(version);
    let version_str = version.as_ref().map(ToString::to_string).unwrap_or_else(|| tag.trim_start_matches('v').to_string());
    Release {
        tag: tag.to_string(),
        version,
        name: format!("{} {}", app.name, version_str),
        published_at,
        html_url,
        notes,
        prerelease: false,
        assets: vec![Asset {
            name: format!("{}-{}-android-aarch64.apk", app.id, version_str),
            url: asset.url,
            size: asset.size,
            sha256: asset.sha256,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suite_has_apps_with_unique_ids() {
        let mut ids: Vec<_> = SUITE_ANDROID.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SUITE_ANDROID.len());
    }

    #[test]
    fn packages_use_their_catalog_override() {
        assert_eq!(package_for("photocraft"), "com.artcraft.photocraft");
        assert_eq!(package_for("tracedraw"), "dev.tracedraw.app");
        assert_eq!(package_for("keydeck"), "app.bramble.keydeck");
        assert_eq!(package_for("values"), "app.bramble.values");
        assert_eq!(package_for("letters"), "app.bramble.letters");
    }

    #[test]
    fn asset_patterns_match_the_release_files() {
        assert!(asset_matches("Keydeck*apk", "Keydeck-android.apk"));
        assert!(asset_matches("Keydeck*apk", "keydeck-ANDROID.APK"));
        assert!(asset_matches("Bramble*apk", "Bramble-Notes-android.apk"));
        assert!(asset_matches("Values*apk", "Values-android.apk"));
        assert!(!asset_matches("Values*apk", "Keydeck-android.apk"));
        assert!(!asset_matches("Keydeck*apk", "Keydeck-x86_64.AppImage"));
        assert!(asset_matches("*-android.apk", "Keydeck-android.apk"));
    }

    #[test]
    fn packaging_suffixes_are_not_part_of_the_version() {
        let procraft = app_by_id("procraft").unwrap();
        let asset = Asset {
            name: "procraft-0.0.6-beta-android-arm64.apk".to_string(),
            url: "https://example.com/procraft-0.0.6-beta-android-arm64.apk".to_string(),
            size: 100,
            sha256: Some("sha256:deadbeef".to_string()),
        };
        let release = android_release(procraft, "v0.0.6-beta", None, None, "https://github.com/...".to_string(), "".to_string(), asset);
        assert_eq!(release.version.as_ref().map(ToString::to_string).as_deref(), Some("0.0.6-beta"));

        assert_eq!(parse_apk_name("procraft-0.0.6-beta-android-arm64.apk"), Some(("procraft", artcraft_launcher_core::version::parse("0.0.6-beta").unwrap())));
        assert_eq!(parse_apk_name("anicraft-0.2.1-android-arm64-v8a.apk"), Some(("anicraft", artcraft_launcher_core::version::parse("0.2.1").unwrap())));
        assert_eq!(parse_apk_name("anicraft-0.2.1.apk"), Some(("anicraft", artcraft_launcher_core::version::parse("0.2.1").unwrap())));
    }

    #[test]
    fn android_release_normalizes_the_asset_name() {
        let app = app_by_id("photocraft").unwrap();
        let asset = Asset {
            name: "photocraft-0.20.2.apk".to_string(),
            url: "https://example.com/photocraft-0.20.2.apk".to_string(),
            size: 1000,
            sha256: Some("sha256:deadbeef".to_string()),
        };
        let version = artcraft_launcher_core::version::parse("0.20.2");
        let release = android_release(app, "v0.20.2", version, Some(1234567890), "https://github.com/...".to_string(), "".to_string(), asset);
        assert_eq!(release.assets[0].name, "photocraft-0.20.2-android-aarch64.apk");
        assert_eq!(release.version.unwrap().to_string(), "0.20.2");
    }

    #[test]
    fn external_release_uses_the_tag_version() {
        let app = app_by_id("keydeck").unwrap();
        let asset = Asset {
            name: "Keydeck-android.apk".to_string(),
            url: "https://example.com/Keydeck-android.apk".to_string(),
            size: 1000,
            sha256: Some("sha256:deadbeef".to_string()),
        };
        let version = artcraft_launcher_core::version::parse("v0.5.1");
        let release = android_release(app, "v0.5.1", version, None, "https://github.com/...".to_string(), "".to_string(), asset);
        assert_eq!(release.assets[0].name, "keydeck-0.5.1-android-aarch64.apk");
        assert_eq!(release.version.unwrap().to_string(), "0.5.1");
    }

    #[test]
    fn release_sources_are_read_from_the_catalog() {
        let keydeck = release_source_for(app_by_id("keydeck").unwrap());
        assert_eq!(keydeck.repo, "bramble-apps/bramble-office");
        assert_eq!(keydeck.asset_pattern, Some("Keydeck*apk"));
        let photocraft = release_source_for(app_by_id("photocraft").unwrap());
        assert_eq!(photocraft.repo, ANDROID_REPO);
        assert_eq!(photocraft.asset_pattern, None);
    }

    #[test]
    fn apk_names_match_the_suite() {
        for app in &SUITE_ANDROID {
            let asset_name = format!("{}-1.0.0.apk", app.id);
            let parsed = parse_apk_name(&asset_name);
            assert!(parsed.is_some(), "failed to parse {asset_name}");
            assert_eq!(parsed.unwrap().0, app.id);
        }
    }
}
EOF

# Android 11+ only lets the launcher see packages named in its manifest's <queries>. Generate that
# list from the catalog so a newly added app or package id is detected without editing the XML.
if [[ -f "$MANIFEST" ]]; then
    QUERY_LIST="$(mktemp)"
    jq -r '
        [(.official_apps[], .community_apps[]) | (.package // ("com.artcraft." + .id))]
        + (.query_packages // [])
        | unique[]' "$CATALOG_JSON" > "$QUERY_LIST"
    UPDATED_MANIFEST="$(mktemp)"
    awk -v list="$QUERY_LIST" '
        /<!-- BEGIN GENERATED PACKAGE QUERIES -->/ {
            print
            print "    <queries>"
            while ((getline pkg < list) > 0) print "        <package android:name=\"" pkg "\" />"
            close(list)
            print "    </queries>"
            skip = 1
            next
        }
        /<!-- END GENERATED PACKAGE QUERIES -->/ { skip = 0; print; next }
        !skip { print }
    ' "$MANIFEST" > "$UPDATED_MANIFEST"
    mv "$UPDATED_MANIFEST" "$MANIFEST"
    rm -f "$QUERY_LIST"
fi

echo "Generated $CATALOG_RS from $CATALOG_JSON ($TOTAL_APPS apps)"