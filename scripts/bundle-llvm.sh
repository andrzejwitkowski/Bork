#!/usr/bin/env bash
# Copy libLLVM's SONAME next to `bork`. The binary's RUNPATH is $ORIGIN.
set -euo pipefail

bin="${1:-target/release/bork}"
if [[ ! -f "$bin" ]]; then
  echo "binary not found: $bin" >&2
  exit 1
fi

bin_dir=$(cd "$(dirname "$bin")" && pwd)
bin="$bin_dir/$(basename "$bin")"

mapfile -t needed < <(readelf -d "$bin" | sed -n 's/.*Shared library: \[\(libLLVM[^]]*\)\].*/\1/p')
if [[ ${#needed[@]} -eq 0 ]]; then
  echo "no libLLVM NEEDED entry in $bin" >&2
  exit 1
fi

if [[ -n "${LLVM_SYS_231_PREFIX:-}" && -x "${LLVM_SYS_231_PREFIX}/bin/llvm-config" ]]; then
  config="${LLVM_SYS_231_PREFIX}/bin/llvm-config"
elif command -v llvm-config-23 >/dev/null 2>&1; then
  config=$(command -v llvm-config-23)
else
  echo "llvm-config-23 not found; set LLVM_SYS_231_PREFIX" >&2
  exit 1
fi

libdir=$("$config" --libdir)
for name in "${needed[@]}"; do
  src="$libdir/$name"
  if [[ ! -e "$src" && -e "$libdir/libLLVM.so" ]]; then
    src="$(dirname "$(readlink -f "$libdir/libLLVM.so")")/$name"
  fi
  if [[ ! -e "$src" ]]; then
    echo "could not find $name under $libdir" >&2
    exit 1
  fi
  cp -f "$(readlink -f "$src")" "$bin_dir/$name"
  echo "bundled $name"
done
