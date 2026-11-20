#!/usr/bin/env bash
# Generate catalog.rs from app-catalog.json.
#
# Run this before building to update the app catalog from the JSON config, or just run
# ./build-launcher.sh from the project root: it regenerates the catalog automatically.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$SCRIPT_DIR/generate_catalog.py" "$@"
