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
ditto "${CARGO_TARGET_DIR:-target}/dist/Vpdf.app" "$staged/Vpdf.app"
codesign --verify --strict "$staged/Vpdf.app"
"$staged/Vpdf.app/Contents/MacOS/vpdf-gui" --version >/dev/null
python3 -B -m tools.package.replace_app "$staged/Vpdf.app" "$install_dir/Vpdf.app"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
  -f "$install_dir/Vpdf.app"
printf 'Installed %s/Vpdf.app\n' "$install_dir"
