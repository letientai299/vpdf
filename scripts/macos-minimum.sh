#!/bin/sh
set -eu
headers=$(otool -l "$1")
printf '%s\n' "$headers" | awk '
  $1 == "cmd" { command = $2 }
  (command == "LC_BUILD_VERSION" && $1 == "minos") ||
  (command == "LC_VERSION_MIN_MACOSX" && $1 == "version") {
    print $2
    found = 1
    exit
  }
  END { if (!found) exit 1 }
'
