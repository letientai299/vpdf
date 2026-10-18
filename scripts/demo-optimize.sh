#!/bin/sh
set -eu
if [ "$#" -ne 2 ]; then
  printf 'Usage: demo-optimize.sh input.png output.webp\n' >&2
  exit 2
fi
source_image=$1
target_image=$2
dimensions=$(magick identify -format '%wx%h' "$source_image")
if [ "$dimensions" != 2560x1440 ]; then
  printf 'Capture must be 2560x1440; got %s\n' "$dimensions" >&2
  exit 1
fi
staged=$(mktemp "${target_image}.XXXXXX")
trap 'rm -f "$staged"' EXIT HUP INT TERM
magick "$source_image" -strip -define webp:lossless=true \
  -define webp:method=6 "webp:$staged"
magick compare -metric AE "$source_image" "$staged" null: 2>/dev/null
chmod 644 "$staged"
mv "$staged" "$target_image"
