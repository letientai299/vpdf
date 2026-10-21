#!/bin/sh
# Sourced by both optimized build scripts.
set -- --no-default-features
case ${VPDF_BENCH_EVENTS:-0} in
  0|'') ;;
  1) set -- "$@" --features bench-events ;;
  *) printf 'VPDF_BENCH_EVENTS requires 0 or 1\n' >&2; exit 1 ;;
esac
