#!/usr/bin/env bash
# build-all.sh — Build every ArtCraft app sequentially, logging pass/fail
set -euo pipefail

APPS=(
  photocraft vectorcraft filmcraft lightcraft pdfcraft effectcraft
  designcraft figcraft wordcraft soundcraft deckcraft cadcraft gridcraft
  subcraft solidcraft materialcraft texturecraft
  notecraft diagramcraft paintcraft bimcraft tracedraw picassosuite
  capsulecraft humanmotioncraft
)

PASS=()
FAIL=()

for app in "${APPS[@]}"; do
  echo ""
  echo "========================================"
  echo "  Building: $app"
  echo "========================================"
  if bash build.sh "-$app"; then
    PASS+=("$app")
    echo "  ✅ PASSED: $app"
  else
    FAIL+=("$app")
    echo "  ❌ FAILED: $app"
  fi
done

echo ""
echo "========================================"
echo "  BUILD ALL SUMMARY"
echo "========================================"
echo "PASSED (${#PASS[@]}): ${PASS[*]:-none}"
echo "FAILED (${#FAIL[@]}): ${FAIL[*]:-none}"
echo ""

if [ ${#FAIL[@]} -eq 0 ]; then
  echo "🎉 All apps built successfully!"
  exit 0
else
  echo "⚠️  ${#FAIL[@]} app(s) failed — see logs above."
  exit 1
fi
