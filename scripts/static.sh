#!/bin/sh
set -eu
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc
export CC_x86_64_unknown_linux_musl=musl-gcc
export CXX_x86_64_unknown_linux_musl=g++
export BINDGEN_EXTRA_CLANG_ARGS_x86_64_unknown_linux_musl=-I/usr/include/x86_64-linux-musl
exec "$@"
