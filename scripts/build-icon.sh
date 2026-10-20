#!/bin/sh
set -eu
if [ "$(uname -s)" != Darwin ]; then
  printf 'Icon generation requires macOS\n' >&2
  exit 1
fi
build_dir="${CARGO_TARGET_DIR:-target}"
mkdir -p "$build_dir"
staged=$(mktemp -d "$build_dir/.icon.XXXXXX")
trap 'rm -rf "$staged"' EXIT
trap 'exit 1' HUP INT TERM
icons="$staged/Vpdf.iconset"
mkdir -p "$icons"
sips -s format png -z 1024 1024 docs/assets/vpdf.svg \
  --out "$staged/source.png" >/dev/null
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$staged/source.png" \
    --out "$icons/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$staged/source.png" \
    --out "$icons/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$icons" -o "$staged/vpdf.icns"
cp "$icons/icon_256x256.png" docs/assets/vpdf.png
mv "$staged/vpdf.icns" docs/assets/vpdf.icns
