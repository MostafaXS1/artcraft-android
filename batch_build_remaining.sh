#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

DEVICE="192.168.1.48:5555"

APPS=(
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
  paintcraft
  pdfcraft
  picassosuite
  prowave
  rustgis
  soundcraft
  subcraft
  texturecraft
  tracedraw
  transit
  vectorcraft
  wordcraft
)

# First install already-built APKs to device
ALREADY=(anicraft cadcraft solidcraft notecraft photocraft bimcraft capsulecraft)
echo "=== Installing already-built APKs to $DEVICE ==="
for a in "${ALREADY[@]}"; do
    apk=(output/"$a"-*.apk)
    if [[ -f "${apk[0]}" ]]; then
        echo "Installing ${apk[0]}..."
        adb -s "$DEVICE" install -r "${apk[0]}" || echo "Failed to install ${apk[0]}"
    fi
done

SUCCESSES=()
FAILURES=()

for app in "${APPS[@]}"; do
    echo ""
    echo "=========================================================="
    echo " BUILDING: $app ($(date))"
    echo "=========================================================="
    if bash build.sh -"$app" --noupdate; then
        echo "=== SUCCESS: $app ==="
        SUCCESSES+=("$app")
        # Find output apk
        apk=(output/"$app"-*.apk)
        if [[ -f "${apk[0]}" ]]; then
            echo "Installing ${apk[0]} to $DEVICE..."
            adb -s "$DEVICE" install -r "${apk[0]}" || echo "Failed to install ${apk[0]}"
        fi
        rm -rf "build/$app"
    else
        echo "=== FAILED: $app ==="
        FAILURES+=("$app")
    fi
    df -h .
done

echo ""
echo "=========================================================="
echo " BUILD SUMMARY"
echo "=========================================================="
echo "Successes (${#SUCCESSES[@]}): ${SUCCESSES[*]}"
echo "Failures (${#FAILURES[@]}): ${FAILURES[*]}"

if [[ ${#FAILURES[@]} -gt 0 ]]; then
    exit 1
fi
exit 0
