#!/bin/sh
set -eu
install_dir="$HOME/.local/bin"
mkdir -p "$install_dir"
staged_binary=$(mktemp "$install_dir/.vpdf.XXXXXX")
trap 'rm -f "$staged_binary"' EXIT HUP INT TERM
install -m 755 "${CARGO_TARGET_DIR:-target}/release/vpdf" "$staged_binary"
mv -f "$staged_binary" "$install_dir/vpdf"
if [ "$(uname -s)" = Darwin ]; then
  "$install_dir/vpdf" --version >/dev/null
fi
printf 'Installed %s/vpdf\n' "$install_dir"
