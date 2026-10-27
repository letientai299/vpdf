#!/bin/sh
set -eu
if [ "$#" -ne 2 ] && [ "$#" -ne 3 ]; then
  printf 'Usage: demo-optimize.sh input.png output.jpg [WIDTHxHEIGHT]\n' >&2
  exit 2
fi
source_image=$1
target_image=$2
expected=${3:-2560x1440}
dimensions=$(magick identify -format '%wx%h' "$source_image")
if [ "$dimensions" != "$expected" ]; then
  printf 'Capture must be %s; got %s\n' "$expected" "$dimensions" >&2
  exit 1
fi
staged=$(mktemp "${target_image}.XXXXXX")
trap 'rm -f "$staged"' EXIT HUP INT TERM
magick "$source_image" -colorspace sRGB -background white -alpha remove \
  -alpha off -strip -depth 8 -sampling-factor 4:4:4 -quality 75 \
  -interlace Plane -define jpeg:optimize-coding=true "jpeg:$staged"
decoded=$(magick -regard-warnings "$staged" -format '%m %wx%h' info:)
if [ "$decoded" != "JPEG $expected" ]; then
  printf 'Invalid JPEG capture: %s\n' "$decoded" >&2
  exit 1
fi
chmod 644 "$staged"
mv "$staged" "$target_image"
