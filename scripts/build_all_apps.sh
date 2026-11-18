#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APPS=(
  anicraft
  bimcraft
  cadcraft
  capsulecraft
  coimanide
  deckcraft
  designcraft
  diagramcraft
  earthcraft
  effectcraft
  figcraft
  filmcraft
  gridcraft
  humanmotioncraft
  lightcraft
  mapcraft
  materialcraft
  notecraft
  paintcraft
  pdfcraft
  photocraft
  picassosuite
  prowave
  rustgis
  solidcraft
  soundcraft
  subcraft
  texturecraft
  tracedraw
  transit
  vectorcraft
  wordcraft
)

TOTAL="${#APPS[@]}"
CURRENT=0
FAILED=()

echo "Starting build of all $TOTAL apps..."

for app in "${APPS[@]}"; do
  CURRENT=$((CURRENT + 1))
  echo ""
  echo "================================================================================"
  echo "[$CURRENT/$TOTAL] Building $app ($(date '+%Y-%m-%d %H:%M:%S'))"
  echo "================================================================================"

  # Check if APK already exists in output/
  if compgen -G "output/${app}-*.apk" > /dev/null; then
    echo "  -> output/${app}-*.apk already exists, skipping."
    continue
  fi

  if bash build.sh -"$app"; then
    echo "  -> SUCCESS: $app built successfully!"
  else
    rc=$?
    echo "  -> ERROR: $app build failed with exit code $rc!" >&2
    FAILED+=("$app")
    exit $rc
  fi
done

echo ""
echo "================================================================================"
if [ ${#FAILED[@]} -eq 0 ]; then
  echo "ALL $TOTAL APKS BUILT SUCCESSFULLY!"
else
  echo "FAILED APKS: ${FAILED[*]}"
  exit 1
fi
echo "================================================================================"
