#!/usr/bin/env bash
# Builds release binaries (the `dist` profile, as a release does) for macOS,
# Linux and Windows on one machine, to test before releasing
# (docs/local-builds.md). Every target a release ships: ARM and x86_64
# for each.
#
#   scripts/build-local.sh
#
# The binaries are copied to target/dist/mac, linux and windows: the ARM
# build as agent-graph, and the x86_64 one beside it as agent-graph-x86_64.
# Runs on an Apple Silicon Mac, x86_64 Linux, or x86_64 Windows in Git
# Bash (Chofter CI's machine, from scripts/ci-slow-checks.sh), with:
#   Mac:     brew install zig llvm; cargo install cargo-zigbuild cargo-xwin
#   Linux:   zig, apt install clang lld llvm; cargo install (the same)
#   Windows: winget install zig.zig LLVM.LLVM; cargo install cargo-zigbuild;
#            Visual Studio's C++ build tools, with its ARM64 ones
set -euo pipefail
cd "$(dirname "$0")/.."

targets=(
  aarch64-apple-darwin x86_64-apple-darwin
  aarch64-unknown-linux-musl x86_64-unknown-linux-musl
  aarch64-pc-windows-msvc x86_64-pc-windows-msvc
)
for arg in "$@"; do
  case "$arg" in
    -h | --help)
      sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown option: $arg (see --help)" >&2
      exit 1
      ;;
  esac
done

host="$(uname -s)-$(uname -m)"
case "$host" in MINGW* | MSYS*) host="Windows-${host##*-}" ;; esac
case "$host" in
  Darwin-arm64)
    zig_hint="brew install zig"
    llvm_hint="brew install llvm"
    ;;
  Linux-x86_64)
    zig_hint="https://ziglang.org/download, or: sudo snap install zig --classic --beta"
    llvm_hint="sudo apt install clang lld llvm"
    ;;
  Windows-x86_64)
    zig_hint="winget install zig.zig"
    llvm_hint="winget install LLVM.LLVM"
    # LLVM's installer doesn't always put it on PATH.
    [ -d "/c/Program Files/LLVM/bin" ] && PATH="/c/Program Files/LLVM/bin:$PATH"
    ;;
  *)
    echo "This script is for an Apple Silicon Mac, x86_64 Linux or x86_64 Windows." >&2
    exit 1
    ;;
esac

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

# The toolchain's own linker, if it can't find its library.
bash scripts/fix-rust-lld.sh

missing=()
need() {
  command -v "$1" >/dev/null 2>&1 || missing+=("$1 ($2)")
}
need cargo "Rust: https://rustup.rs"
need rustup "https://rustup.rs"
for target in "${targets[@]}"; do
  case "$target" in
    *-linux-*)
      need zig "$zig_hint"
      need cargo-zigbuild "cargo install cargo-zigbuild"
      ;;
    # Linux and Windows build macOS's with Zig too.
    *-apple-darwin)
      if [ "$host" != Darwin-arm64 ]; then
        need zig "$zig_hint"
        need cargo-zigbuild "cargo install cargo-zigbuild"
      fi
      ;;
    # Windows builds its own with Visual Studio's tools, but ring's C for
    # Windows on ARM needs clang, archived with llvm-lib, since cl.exe/lib.exe
    # for that target only come with the ARM64 build tools, not every install.
    *-windows-*)
      need clang "$llvm_hint"
      need llvm-lib "$llvm_hint"
      if [ "$host" != Windows-x86_64 ]; then
        need cargo-xwin "cargo install cargo-xwin"
      fi
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
if [ "$host" != Windows-x86_64 ] && [[ " ${targets[*]} " == *-windows-* ]]; then
  rust_lld="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/rust-lld"
  if ! "$rust_lld" -flavor link --version >/dev/null 2>&1; then
    echo "rust-lld doesn't run here; Windows links with Zig's lld."
    need zig "$zig_hint"
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
    *-apple-darwin)
      # A Mac builds its own; elsewhere it's cross-compiled, with Zig's copy
      # of the macOS system libraries (nothing here links Apple's frameworks).
      if [ "$host" = Darwin-arm64 ]; then build=(cargo build); else build=(cargo zigbuild); fi
      ;;
    *-linux-*) build=(cargo zigbuild) ;;
    *-windows-*)
      if [ "$host" = Windows-x86_64 ]; then
        build=(cargo build)
        # ring's build script picks clang for Windows on ARM once it has a
        # compiler to inspect, but cc-rs's own default probe for that target
        # looks for cl.exe (and lib.exe, to archive it) first, and fails
        # before ring gets a say, so point it at clang and llvm-lib directly.
        if [ "$target" = aarch64-pc-windows-msvc ]; then
          export CC_aarch64_pc_windows_msvc=clang
          export AR_aarch64_pc_windows_msvc=llvm-lib
        fi
      else
        # ring compiles its C with clang, not clang-cl, for Windows on ARM,
        # so cargo-xwin must pass flags clang takes.
        build=(cargo xwin build --cross-compiler clang)
      fi
      ;;
  esac
  printf '\n==> %s --profile dist --target %s\n' "${build[*]}" "$target"
  "${build[@]}" --profile dist --target "$target"
done

# Where each build is copied: one folder per OS, the ARM build named for
# it, and the x86_64 build beside it, named for its arch.
# (target/dist is also cargo's folder for the dist profile's build scripts,
# so only these three folders are touched.)
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
# Each binary goes in as a new file, renamed over the old one, never copied
# over it: macOS remembers an executable's code signature by its file, so
# one overwritten in place is killed ("Killed: 9") whenever it runs. The
# hooks and the viewer may run target/dist/mac/agent-graph, and a rename
# also means there's always a whole one there.
install_bin() {
  local tmp="$2.new.$$"
  cp "$1" "$tmp" && mv -f "$tmp" "$2"
}
printf '\n==> Built\n'
built=()
for target in "${targets[@]}"; do
  bin="$(dest "$target")"
  mkdir -p "$(dirname "$bin")"
  install_bin "target/$target/dist/$(basename "${bin/-x86_64/}")" "$bin"
  built+=("$bin")
  if command -v file >/dev/null 2>&1; then
    printf '%s\n    %s\n' "$bin" "$(file -b "$bin")"
  else
    printf '%s\n' "$bin"
  fi
done
# Anything else in those folders is left from an older build.
for f in "$out"/mac/* "$out"/linux/* "$out"/windows/*; do
  [ -e "$f" ] || continue
  [[ " ${built[*]} " == *" $f "* ]] || rm -rf "$f"
done

# Runs what this machine can. Linux and Windows run their own x86_64
# build; the others were only built. A Mac runs its own builds (the Intel one with Rosetta,
# if it's installed), and Linux's in Docker if it's up.
own=""
case "$host" in
  Linux-x86_64) own=x86_64-unknown-linux-musl ;;
  Windows-x86_64) own=x86_64-pc-windows-msvc ;;
esac
if [ -n "$own" ]; then
  printf '\n==> Smoke test: running the build for this machine\n'
  printf '%s: ' "$(dest "$own")"
  "$(dest "$own")" --version
  echo "The other binaries were built but can't run here, so they weren't tested."
  exit 0
fi
printf '\n==> Smoke test: running each binary this Mac can\n'
printf '%s: ' "$(dest aarch64-apple-darwin)"
"$(dest aarch64-apple-darwin)" --version
if arch -x86_64 /usr/bin/true 2>/dev/null; then
  printf '%s (Rosetta): ' "$(dest x86_64-apple-darwin)"
  arch -x86_64 "$(dest x86_64-apple-darwin)" --version
else
  echo "(Rosetta isn't installed, so the Intel Mac binary was built but not tested: softwareupdate --install-rosetta.)"
fi
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
echo "The Windows binaries can't run on a Mac, so they weren't tested: copy $out/windows/agent-graph.exe to a Windows on ARM virtual machine (or agent-graph-x86_64.exe to an x86_64 one) to try it."
