#!/bin/sh
set -eu
dependencies=$(otool -L "$1")
printf '%s\n' "$dependencies" | tail -n +2 |
  while read -r dependency _; do
    case "$dependency" in
      /System/Library/*|/usr/lib/*) ;;
      *) printf 'Non-system dependency: %s\n' "$dependency" >&2; exit 1 ;;
    esac
  done
