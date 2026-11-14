#!/usr/bin/env bash
# Generate about.rs from about.json.
#
# Run this before building to update the app pages from the JSON config, or just run
# ./build-launcher.sh from the project root: it regenerates the pages automatically.
#
# The official Craft Apps' product pages are hand-written in artcraft-launcher-core::details.
# The apps the Android catalog carries that the core suite has no page for get theirs here
# instead, from about.json: the same AppDetails shape (id, headline, about, features, shots,
# platforms), so the details view treats them exactly like the official pages.
#
# Each page needs non-empty headline, about, features, shots (empty is fine: no screenshot
# repository is published for these apps) and platforms, and an id that appears in
# app-catalog.json, exactly once per app.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ABOUT_JSON="$SCRIPT_DIR/about.json"
ABOUT_RS="$SCRIPT_DIR/src/about.rs"
CATALOG_JSON="$SCRIPT_DIR/app-catalog.json"

if [[ ! -f "$ABOUT_JSON" ]]; then
    echo "error: $ABOUT_JSON not found" >&2
    exit 1
fi
if ! command -v jq >/dev/null; then
    echo "error: jq is required to generate the pages" >&2
    exit 1
fi
if ! jq -e . "$ABOUT_JSON" >/dev/null; then
    echo "error: $ABOUT_JSON is not valid JSON" >&2
    exit 1
fi

TOTAL=$(jq '.about | length' "$ABOUT_JSON")

cat > "$ABOUT_RS" << EOF
//! The product pages for Android-catalog apps the core suite does not carry one for.
//!
//! Generated from about.json by generate-about.sh. Do not edit by hand.
//!
//! The core suite's pages live in artcraft-launcher-core::details; anything in the Android
//! catalog without one gets its page here: every community app, and AniCraft (an official
//! app this launcher ships but the desktop suite does not). The same \`AppDetails\` shape the
//! official Craft Apps use, so the details view treats a page here exactly like an official
//! one once \`details::set_extra_details\` has registered this slice at startup (done in
//! \`app::new\`). Text here is English; the page follows the official format.

use artcraft_launcher_core::details::{AppDetails, Feature};

/// The community apps' pages, in the order the catalog lists them.
pub static ABOUT: &[AppDetails] = &[
EOF

jq -r '
    .about[]
    | "    AppDetails {\n" +
      "        id: \(.id | @json),\n" +
      "        headline: \(.headline | @json),\n" +
      "        about: \(.about | @json),\n" +
      "        features: &[\n" +
      ([.features[] | "            Feature { title: \(.title | @json), text: \(.text | @json) },"] | join("\n")) + "\n" +
      "        ],\n" +
      "        shots: &[],\n" +
      "        platforms: \(.platforms | @json),\n" +
      "    },"
' "$ABOUT_JSON" >> "$ABOUT_RS"

cat >> "$ABOUT_RS" << 'EOF'
];

/// The apps without a core page: anything in the Android catalog that artcraft-launcher-core's
/// suite does not carry one for.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_belongs_to_the_catalog() {
        let mut ids: Vec<_> = ABOUT.iter().map(|d| d.id).collect();
        for id in &ids {
            assert!(crate::catalog::app_by_id(id).is_some(), "{id} is not in the Android catalog");
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ABOUT.len());
    }

    #[test]
    fn every_android_app_without_a_core_page_has_one() {
        for app in &crate::catalog::SUITE_ANDROID {
            if artcraft_launcher_core::details::AppDetails::by_id(app.id).is_some() {
                continue;
            }
            assert!(ABOUT.iter().any(|d| d.id == app.id), "{} has no page in about.json", app.id);
        }
    }

    #[test]
    fn every_page_is_filled_in() {
        for page in ABOUT {
            assert!(!page.headline.is_empty(), "{}: headline", page.id);
            assert!(!page.about.is_empty() && page.paragraphs().next().is_some(), "{}: about", page.id);
            assert!(!page.features.is_empty(), "{}: features", page.id);
            for feature in page.features {
                assert!(!feature.title.is_empty() && !feature.text.is_empty(), "{}: feature", page.id);
            }
            assert!(!page.platforms.is_empty(), "{}: platforms", page.id);
        }
    }
}
EOF

echo "Generated $ABOUT_RS from $ABOUT_JSON ($TOTAL pages)"
