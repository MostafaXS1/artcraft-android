#!/usr/bin/env bash
# backup.sh — create a dated tarball with only the named folders and root files
set -Eeuo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

DATE="$(date +%Y-%m-%d)"
ARCHIVE="backups/${DATE}.tar.gz"

mkdir -p backups

# Folder names to include (matching backups/New Folder/):
FOLDERS=(adapters .github docs lib templates tests vendor)

# Build the tar argument list: root files only (no subfolders), plus named folders.
ARGS=()
for f in *; do
  if [ -f "$f" ]; then
    ARGS+=("$f")
  fi
done
for d in "${FOLDERS[@]}"; do
  if [ -d "$d" ]; then
    ARGS+=("$d")
  fi
done

tar -czf "$ARCHIVE" "${ARGS[@]}"

echo "Created $ARCHIVE"
