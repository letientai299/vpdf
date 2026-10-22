#!/bin/sh
set -eu
if [ "$#" -eq 0 ]; then
  find src tests benches crates -name '*.rs' -type f \
    -exec sh "$0" {} +
else
  awk 'length($0) > 80 { print FILENAME ":" FNR ": exceeds 80 columns"; bad=1 } END { exit bad }' "$@"
fi
