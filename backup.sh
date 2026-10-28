#!/usr/bin/env bash
# backup.sh — Zip all files not in .gitignore to a timestamped archive
# at the project root. Excludes the gitignored source/ directory.
set -Eeuo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
ARCHIVE="artcraft-backup-${TIMESTAMP}.zip"

# Gather tracked files + untracked-but-not-ignored files, excluding the gitignored source/ dir.
mapfile -t FILES < <(
    git ls-files --cached | grep -v '^source/' || true
    git ls-files --others --exclude-standard | grep -v '^source/' || true
)

# Remove duplicates while preserving order.
mapfile -t FILES < <(printf '%s\n' "${FILES[@]}" | awk '!seen[$0]++')

if [[ ${#FILES[@]} -eq 0 || -z "${FILES[0]}" ]]; then
    echo "No files to back up." >&2
    exit 1
fi

echo "Backing up ${#FILES[@]} files to ${ARCHIVE}..."

LIST_FILE="$(mktemp)"
trap 'rm -f "$LIST_FILE"' EXIT
printf '%s\n' "${FILES[@]}" > "$LIST_FILE"

zip -r "$ARCHIVE" -@ < "$LIST_FILE"

echo "Done: ${ROOT}/${ARCHIVE}"
