#!/bin/sh
set -eu
build_dir="${CARGO_TARGET_DIR:-target}"
case $(uname -s) in
  Darwin)
    target=$(rustc --print host-tuple)
    cargo build --release --locked --target "$target"
    binary="$build_dir/$target/release/vpdf"
    dependencies=$(otool -L "$binary")
    printf '%s\n' "$dependencies" | tail -n +2 | \
      while read -r dependency _; do
        case "$dependency" in
          /System/Library/*|/usr/lib/*) ;;
          *) printf 'Non-system dependency: %s\n' "$dependency" >&2; exit 1 ;;
        esac
      done
    ;;
  Linux)
    if [ "$(uname -m)" != x86_64 ]; then
      printf 'Static build requires x86-64 Ubuntu\n' >&2
      exit 1
    fi
    target=x86_64-unknown-linux-musl
    rustup target add "$target"
    sh scripts/static.sh cargo build --release --locked --target "$target"
    binary="$build_dir/$target/release/vpdf"
    headers=$(readelf -l "$binary")
    dynamic=$(readelf -d "$binary")
    if printf '%s\n' "$headers" | grep -q INTERP || \
      printf '%s\n' "$dynamic" | grep -q NEEDED; then
      printf 'Linux executable requires shared libraries\n' >&2
      exit 1
    fi
    ;;
  *) printf 'Build requires macOS or Ubuntu\n' >&2; exit 1 ;;
esac
mkdir -p "$build_dir/release"
staged=$(mktemp "$build_dir/release/.vpdf.XXXXXX")
trap 'rm -f "$staged"' EXIT HUP INT TERM
install -m 755 "$binary" "$staged"
mv -f "$staged" "$build_dir/release/vpdf"
if [ "$(uname -s)" = Darwin ]; then
  "$build_dir/release/vpdf" --version >/dev/null
fi
