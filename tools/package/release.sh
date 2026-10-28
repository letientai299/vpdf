#!/bin/sh
set -eu
build_dir="${CARGO_TARGET_DIR:-target}"
help_assets=$(python3 -B -m tools.help.assets --check)
fingerprint=$(basename "$help_assets")
"$build_dir/release/vpdf" --version | grep -Fx "Help assets: $fingerprint"
release_dir="$build_dir/dist"
mkdir -p "$release_dir"
staged=$(mktemp -d "$release_dir/.release.XXXXXX")
trap 'rm -rf "$staged"' EXIT HUP INT TERM
case $(uname -s) in
  Linux)
    upx --best --lzma -o "$staged/vpdf" "$build_dir/release/vpdf"
    upx --test "$staged/vpdf"
    ;;
  Darwin)
    cp "$build_dir/release/vpdf" "$staged/vpdf"
    printf 'UPX cannot pack macOS; retaining stripped executable\n'
    ;;
  *) printf 'Release requires macOS or Ubuntu\n' >&2; exit 1 ;;
esac
"$staged/vpdf" --version
mv -f "$staged/vpdf" "$release_dir/vpdf"
if [ "$(uname -s)" = Darwin ]; then
  "$release_dir/vpdf" --version >/dev/null
fi
printf 'Release executable: %s/vpdf\n' "$release_dir"
