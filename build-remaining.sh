#!/usr/bin/env bash
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

APPS=(
  photocraft
  vectorcraft
  filmcraft
  lightcraft
  pdfcraft
  effectcraft
  designcraft
  figcraft
  wordcraft
  soundcraft
  deckcraft
  cadcraft
  gridcraft
  subcraft
  solidcraft
  materialcraft
  texturecraft
  notecraft
  diagramcraft
  paintcraft
  bimcraft
  tracedraw
  picassosuite
  capsulecraft
  humanmotioncraft
  rustgis
  earthcraft
  mapcraft
  prowave
  transit
  anicraft
  coimanide
  pcbcraft
  fieldcraft
  papercraft
  ferrite
  pixelcraft
  clothcraft
)

mkdir -p output logs

echo "=================================================="
echo "ArtCraft Suite Sequential Build Loop"
echo "Total apps in suite: ${#APPS[@]}"
echo "=================================================="

FAILED=()
BUILT=()
SKIPPED=()

for app in "${APPS[@]}"; do
  existing=(output/${app}-*.apk)
  if [[ -f "${existing[0]}" ]]; then
    echo ">>> [SKIP] ${app} already built: ${existing[0]}"
    SKIPPED+=("$app")
    continue
  fi

  echo ""
  echo "=================================================="
  echo ">>> [BUILDING] ${app} ($(date '+%Y-%m-%d %H:%M:%S'))"
  echo "=================================================="

  if bash build.sh "-${app}" --noupdate; then
    echo ">>> [SUCCESS] ${app} build finished."
    BUILT+=("$app")
  else
    echo ">>> [ERROR] ${app} build failed!" >&2
    FAILED+=("$app")
  fi
done

echo ""
echo "=================================================="
echo "Build Loop Finished at $(date '+%Y-%m-%d %H:%M:%S')"
echo "Skipped (already built): ${#SKIPPED[@]}"
echo "Newly built:             ${#BUILT[@]}"
echo "Failed:                  ${#FAILED[@]}"
if [[ ${#FAILED[@]} -gt 0 ]]; then
  echo "Failed apps: ${FAILED[*]}"
fi
echo "=================================================="
