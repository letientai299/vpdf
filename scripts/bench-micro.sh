#!/bin/sh
set -eu
packages="vpdf vpdf-model vpdf-search vpdf-reader vpdf-pdf vpdf-terminal vpdf-protocol vpdf-ui"
if [ "${1:-}" = --package ]; then
  case "${2:-}" in
    vpdf|vpdf-model|vpdf-search|vpdf-reader|vpdf-pdf|vpdf-terminal|vpdf-protocol|vpdf-ui) packages=$2 ;;
    *) printf 'Unknown benchmark package\n' >&2; exit 1 ;;
  esac
  shift 2
fi
for package in $packages; do
  cargo bench --locked --profile release -p "$package" --bench micro -- "$@"
  if [ "$package" = vpdf ]; then
    cargo bench --locked --profile release -p vpdf --bench presentation -- "$@"
  fi
done
