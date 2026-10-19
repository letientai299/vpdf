#!/bin/sh
set -eu
packages="vpdf vpdf-terminal vpdf-protocol"
if [ "${1:-}" = --package ]; then
  case "${2:-}" in
    vpdf|vpdf-terminal|vpdf-protocol) packages=$2 ;;
    *) printf 'Unknown benchmark package\n' >&2; exit 1 ;;
  esac
  shift 2
fi
for package in $packages; do
  cargo bench --locked --profile release -p "$package" --bench micro -- "$@"
done
