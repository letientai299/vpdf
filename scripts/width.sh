#!/bin/sh
set -eu
find src tests benches -name '*.rs' -type f 2>/dev/null |
  xargs awk 'length($0) > 80 { print FILENAME ":" FNR ": exceeds 80 columns"; bad=1 } END { exit bad }'
