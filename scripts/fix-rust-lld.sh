#!/usr/bin/env bash
# Rust 1.98's macOS toolchain ships rust-lld, the linker the WebAssembly
# build uses, looking for libLLVM.dylib in rustlib/<host>/lib, but puts the
# library in the toolchain's own lib. rust-lld then aborts ("Library not
# loaded: @rpath/libLLVM.dylib"). This links the library in where rust-lld
# looks, if it's missing there. It does nothing elsewhere, or once a
# toolchain has it in place (a `rustup update` replaces the link).
set -euo pipefail

[ "$(uname -s)" = Darwin ] || exit 0

# rustc from PATH, or from rustup (Homebrew's doesn't put it on PATH).
PATH="$HOME/.cargo/bin:$PATH:/opt/homebrew/bin"
if command -v rustc >/dev/null 2>&1; then
  rustc=rustc
elif command -v rustup >/dev/null 2>&1; then
  rustc="$(rustup which rustc)"
else
  exit 0 # No Rust; the build will say so.
fi

sysroot="$("$rustc" --print sysroot)"
host="$("$rustc" -vV | sed -n 's/^host: //p')"
lld="$sysroot/lib/rustlib/$host/bin/rust-lld"
wanted="$sysroot/lib/rustlib/$host/lib/libLLVM.dylib"
shipped="$sysroot/lib/libLLVM.dylib"

[ -e "$lld" ] || exit 0
[ -e "$wanted" ] && exit 0
if [ ! -e "$shipped" ]; then
  echo "Note: rust-lld may not run: no libLLVM.dylib in $sysroot/lib to link to." >&2
  exit 0
fi

ln -s ../../../libLLVM.dylib "$wanted"
echo "Linked libLLVM.dylib in for rust-lld: $wanted" >&2
