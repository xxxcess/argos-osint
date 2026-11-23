#!/bin/sh
# Rebuild the macOS launcher icon from the same SVG shipped on Linux.
set -eu
repo="$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)"
resources="$repo/apps/fleetworks-desktop/resources"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/Fleetworks.iconset"
sed 's/viewBox="0 0 208 208"/width="1024" height="1024" viewBox="0 0 208 208"/' \
    "$resources/fleetworks.svg" > "$scratch/launcher.svg"
sips -s format png "$scratch/launcher.svg" --out "$scratch/launcher.png" >/dev/null
for size in 16 32 128 256 512; do
    output="$scratch/Fleetworks.iconset/icon_${size}x${size}"
    sips -z "$size" "$size" "$scratch/launcher.png" --out "$output.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" "$scratch/launcher.png" --out "$output@2x.png" >/dev/null
done
iconutil -c icns "$scratch/Fleetworks.iconset" -o "$resources/Fleetworks.icns"
