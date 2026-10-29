#!/bin/sh
set -eu
package=$1
shift
for arg; do
  case $arg in
    -r | --release | --profile | --profile=* | --target | --target=*)
      printf 'Debug builds take no profile or target flags\n' >&2
      exit 1
      ;;
  esac
done
build_dir="${CARGO_TARGET_DIR:-target}"
cargo build --locked -p "$package" -p vpdf-dev \
  --bin "$package" --bin help-pdf "$@"
triple=${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}
python3 -B -m tools.help.assets \
  --exporter "$build_dir/${triple}debug/help-pdf" >/dev/null
