#!/usr/bin/env bash
# Regenerate the icon asset set from build/appicon.svg (the source of
# truth — Zorite's outliner page: a silver wireframe note with indented
# gold bullets, on PortFinder's navy, matching the packetThrower app
# family). Requires rsvg-convert (librsvg), ImageMagick 7 (magick), and
# macOS `iconutil` for the .icns.
set -euo pipefail
cd "$(dirname "$0")/.."          # repo root
SVG="build/appicon.svg"

# 1024 master PNG — build/ copy + the source for cargo-packager, the
# .deb `files` icon, and the fpm .rpm icon.
rsvg-convert -w 1024 -h 1024 "$SVG" -o build/appicon.png
cp build/appicon.png resources/icons/icon.png

# Windows multi-resolution .ico — embedded into the .exe PE section via
# resources/icon.rc (build.rs) and listed in cargo-packager's icons.
magick build/appicon.png -define icon:auto-resize=256,128,64,48,32,16 build/windows/icon.ico
cp build/windows/icon.ico resources/icons/icon.ico

# macOS .icns — listed in cargo-packager's macos icons.
TMP="$(mktemp -d)"; ICONSET="$TMP/icon.iconset"; mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  rsvg-convert -w "$s"          -h "$s"          "$SVG" -o "$ICONSET/icon_${s}x${s}.png"
  rsvg-convert -w "$((s*2))"    -h "$((s*2))"    "$SVG" -o "$ICONSET/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$ICONSET" -o resources/icons/icon.icns
rm -rf "$TMP"

echo "Regenerated: build/appicon.png, build/windows/icon.ico, resources/icons/icon.{png,ico,icns}"
