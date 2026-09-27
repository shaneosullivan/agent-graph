#!/usr/bin/env bash
# Builds release binaries (the `dist` profile, as a release does) for macOS,
# Linux and Windows on this Mac, to test before releasing
# (docs/local-builds.md). By default, for ARM on each: the Mac's own, and
# what Docker and a Windows on ARM virtual machine run natively here.
#
#   scripts/build-local.sh           # ARM: macOS, Linux, Windows
#   scripts/build-local.sh --x86     # and x86_64 Linux and Windows too
#
# The binaries are copied to target/dist/mac, linux and windows, one file
# in each (and with --x86, an agent-graph-x86_64 beside it). Needs an Apple
# Silicon Mac, with:
#   Linux:   brew install zig && cargo install cargo-zigbuild
#   Windows: brew install llvm && cargo install cargo-xwin
set -euo pipefail
cd "$(dirname "$0")/.."

targets=(aarch64-apple-darwin aarch64-unknown-linux-musl aarch64-pc-windows-msvc)
for arg in "$@"; do
  case "$arg" in
    --x86) targets+=(x86_64-unknown-linux-musl x86_64-pc-windows-msvc) ;;
    -h | --help)
      sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown option: $arg (see --help)" >&2
      exit 1
      ;;
  esac
done

if [ "$(uname -s)-$(uname -m)" != "Darwin-arm64" ]; then
  echo "This script is for an Apple Silicon Mac." >&2
  exit 1
fi

# cargo's own installs (cargo-zigbuild, cargo-xwin), and cargo itself when
# rustup (Homebrew's, say) doesn't put it on PATH.
PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1 && command -v rustup >/dev/null 2>&1; then
  PATH="$(dirname "$(rustup which cargo)"):$PATH"
fi
# LLVM's clang-cl and lld-link, for cargo-xwin. Homebrew doesn't link them.
if command -v brew >/dev/null 2>&1 && [ -d "$(brew --prefix llvm 2>/dev/null)/bin" ]; then
  PATH="$(brew --prefix llvm)/bin:$PATH"
fi

missing=()
need() {
  command -v "$1" >/dev/null 2>&1 || missing+=("$1 ($2)")
}
need cargo "Rust: https://rustup.rs"
need rustup "https://rustup.rs"
for target in "${targets[@]}"; do
  case "$target" in
    *-linux-*)
      need zig "brew install zig"
      need cargo-zigbuild "cargo install cargo-zigbuild"
      ;;
    *-windows-*)
      need cargo-xwin "cargo install cargo-xwin"
      need clang-cl "brew install llvm"
      ;;
  esac
done
if [ ${#missing[@]} -gt 0 ]; then
  echo "Missing what the builds need:" >&2
  printf '%s\n' "${missing[@]}" | sort -u | sed 's/^/  /' >&2
  exit 1
fi

printf '==> rustup target add %s\n' "${targets[*]}"
rustup target add "${targets[@]}"

# Windows links with the toolchain's rust-lld. If that can't run (some
# toolchains ship it looking for libLLVM.dylib where it isn't), link with
# Zig's copy of lld instead, by the name rustc runs: lld-link.
if [[ " ${targets[*]} " == *-windows-* ]]; then
  rust_lld="$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/rust-lld"
  if ! "$rust_lld" -flavor link --version >/dev/null 2>&1; then
    echo "rust-lld doesn't run here; Windows links with Zig's lld."
    need zig "brew install zig"
    if [ ${#missing[@]} -gt 0 ]; then
      echo "Missing: ${missing[*]}" >&2
      exit 1
    fi
    shim="$(mktemp -d)"
    trap 'rm -rf "$shim"' EXIT
    # rustc passes rust-lld's `-flavor link`, which lld-link doesn't take.
    printf '#!/bin/sh\nif [ "$1" = "-flavor" ]; then shift 2; fi\nexec zig lld-link "$@"\n' >"$shim/lld-link"
    chmod +x "$shim/lld-link"
    PATH="$shim:$PATH"
  fi
fi

for target in "${targets[@]}"; do
  case "$target" in
    *-apple-darwin) build=(cargo build) ;;
    *-linux-*) build=(cargo zigbuild) ;;
    # ring compiles its C with clang, not clang-cl, for Windows on ARM,
    # so cargo-xwin must pass flags clang takes.
    *-windows-*) build=(cargo xwin build --cross-compiler clang) ;;
  esac
  printf '\n==> %s --profile dist --target %s\n' "${build[*]}" "$target"
  "${build[@]}" --profile dist --target "$target"
done

# Where each build is copied: one file per folder, named for its OS, and
# the x86_64 builds (--x86) beside the ARM ones, named for their arch.
# (target/dist is also cargo's folder for the dist profile's build scripts,
# so only these three folders are replaced.)
out=target/dist
dest() {
  local os ext=""
  case "$1" in
    *-apple-darwin) os=mac ;;
    *-linux-*) os=linux ;;
    *-windows-*) os=windows ext=.exe ;;
  esac
  case "$1" in
    aarch64-*) echo "$out/$os/agent-graph$ext" ;;
    *) echo "$out/$os/agent-graph-${1%%-*}$ext" ;;
  esac
}
rm -rf "$out/mac" "$out/linux" "$out/windows"
printf '\n==> Built\n'
for target in "${targets[@]}"; do
  bin="$(dest "$target")"
  mkdir -p "$(dirname "$bin")"
  cp "target/$target/dist/$(basename "${bin/-x86_64/}")" "$bin"
  printf '%s\n    %s\n' "$bin" "$(file -b "$bin")"
done

# Runs what this Mac can: its own build, and Linux's in Docker if it's up.
printf '\n==> Smoke test: running each binary this Mac can\n'
"$(dest aarch64-apple-darwin)" --version
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  for target in "${targets[@]}"; do
    case "$target" in
      aarch64-unknown-linux-musl) platform=linux/arm64 ;;
      x86_64-unknown-linux-musl) platform=linux/amd64 ;;
      *) continue ;;
    esac
    printf '%s (Docker, Alpine): ' "$(dest "$target")"
    docker run --rm --platform "$platform" -v "$PWD/$(dest "$target"):/agent-graph:ro" alpine /agent-graph --version
  done
else
  echo "(Docker isn't running, so the Linux binaries were built but not tested. Start Docker and run this again to test them.)"
fi
echo "The Windows binaries can't run on a Mac, so they weren't tested: copy $out/windows/agent-graph.exe to a Windows on ARM virtual machine to try it."
