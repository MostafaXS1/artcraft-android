#!/usr/bin/env bash
# Remove all build artifacts, generated files, and secrets so the builder
# folder returns to its clean, publishable state.
set -euo pipefail
cd "$(dirname "$0")"

echo "Cleaning ArtCraft Android Builder..."
echo

# App source checkouts (cloned during build)
rm -rf source/

# Cargo build artifacts (per-app target dirs)
rm -rf build/

# Built APKs
rm -rf output/

# Build logs
rm -rf logs/

# Signing key and password (generated on first build)
rm -rf keys/

# Stale backups
rm -rf backups/

# Cargo's local registry cache (if redirected here)
rm -rf .cargo-home/

# OS noise
rm -f .DS_Store Thumbs.db

echo "Done. Remaining files:"
find . -maxdepth 2 -not -path '*/\.*' | sort
