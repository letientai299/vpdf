#!/bin/sh
set -eu
if [ "$(uname -s)" != Darwin ]; then
  printf 'App installation requires macOS\n' >&2
  exit 1
fi
install_dir="$HOME/Applications"
mkdir -p "$install_dir"
staged=$(mktemp -d "$install_dir/.vpdf.XXXXXX")
trap 'rm -rf "$staged"' EXIT
trap 'exit 1' HUP INT TERM
ditto "${CARGO_TARGET_DIR:-target}/dist/vpdf.app" "$staged/vpdf.app"
codesign --verify --strict "$staged/vpdf.app"
"$staged/vpdf.app/Contents/MacOS/vpdf-gui" --version >/dev/null
python3 -B scripts/replace_app.py "$staged/vpdf.app" "$install_dir/vpdf.app"
printf 'Installed %s/vpdf.app\n' "$install_dir"
