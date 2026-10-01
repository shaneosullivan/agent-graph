#!/usr/bin/env bash
# Gets a build machine ready for scripts/test-all.sh and
# scripts/ci-slow-checks.sh (.chofter.json): checks the tools they need are
# there, adds the Rust components and the WebAssembly target, and installs
# the site's packages and the cross-compiling cargo tools. What it can't
# install (Rust, Node, Java, Zig, LLVM, Visual Studio's tools) it names.
set -euo pipefail
cd "$(dirname "$0")/.."

missing=()
need() {
  command -v "$1" >/dev/null 2>&1 || missing+=("$1 ($2)")
}
need git "https://git-scm.com"
need cargo "Rust: https://rustup.rs"
need rustup "https://rustup.rs, for the WebAssembly target"
need node "Node 22 or later: https://nodejs.org"
need npm "comes with Node"
if [ ${#missing[@]} -gt 0 ]; then
  echo "This machine is missing what the checks need:" >&2
  printf '  %s\n' "${missing[@]}" >&2
  exit 1
fi
# The site's tests run TypeScript with --experimental-strip-types (22.6+).
node_major="$(node -p 'process.versions.node.split(".")[0]')"
if [ "$node_major" -lt 22 ]; then
  echo "The site needs Node 22 or later; this machine has $(node --version)." >&2
  exit 1
fi
# Only the slow checks need it (the Firestore emulator), so a warning.
if ! command -v java >/dev/null 2>&1; then
  echo "Note: no java on PATH. The slow checks (scripts/ci-slow-checks.sh) need Java 21 for the Firestore emulator." >&2
fi
# The slow checks also build every release target (scripts/build-local.sh),
# cross-compiling with Zig, and LLVM elsewhere than Windows; that script
# says what's missing.
PATH="$HOME/.cargo/bin:$PATH"
case "$(uname -s)" in
  Darwin)
    if command -v brew >/dev/null 2>&1 && [ -d "$(brew --prefix llvm 2>/dev/null)/bin" ]; then
      PATH="$(brew --prefix llvm)/bin:$PATH"
    fi
    cross_tools=(zig clang llvm-lib)
    hint="brew install zig llvm"
    ;;
  MINGW* | MSYS*)
    [ -d "/c/Program Files/LLVM/bin" ] && PATH="/c/Program Files/LLVM/bin:$PATH"
    cross_tools=(zig clang)
    hint="winget install zig.zig LLVM.LLVM, and Visual Studio's ARM64 C++ build tools"
    ;;
  *)
    cross_tools=(zig clang llvm-lib)
    hint="sudo apt install clang lld llvm, and zig from https://ziglang.org/download"
    ;;
esac
command -v cargo-zigbuild >/dev/null 2>&1 || cargo install --locked cargo-zigbuild
# Windows builds its own with Visual Studio's tools, not cargo-xwin.
case "$(uname -s)" in
  MINGW* | MSYS*) ;;
  *) command -v cargo-xwin >/dev/null 2>&1 || cargo install --locked cargo-xwin ;;
esac
cross_missing=()
for tool in "${cross_tools[@]}"; do
  command -v "$tool" >/dev/null 2>&1 || cross_missing+=("$tool")
done
if [ ${#cross_missing[@]} -gt 0 ]; then
  echo "Note: no ${cross_missing[*]} on PATH. The slow checks build every release target, and need them: $hint" >&2
fi

# On Chofter CI, cargo builds into a folder kept between runs. Each run
# checks out into a new folder, so with cargo's own target/ every run built
# every dependency again, for each of the seven targets. Later steps (the
# tests, the slow checks, and the agent that fixes them) get it through
# GITHUB_ENV.
if [ -n "${CHOFTER_OUTPUT_DIR:-}" ] && [ -n "${GITHUB_ENV:-}" ] && [ -z "${CARGO_TARGET_DIR:-}" ]; then
  slug="$(printf %s "${GITHUB_REPOSITORY:-agent-graph}" | sed 's#[^A-Za-z0-9._-][^A-Za-z0-9._-]*#__#g')"
  cache="$HOME/.chofter/cache/$slug"
  # C:/Users/... rather than /c/Users/..., which cargo, node and bash all take.
  command -v cygpath >/dev/null 2>&1 && cache="$(cygpath -m "$cache")"
  target_dir="$cache/target"
  # It only grows: a new Rust builds everything afresh beside the old, so
  # start again then, or when it's past 40GB.
  rustc_now="$(rustc -vV)"
  if [ -d "$target_dir" ]; then
    why=""
    if [ "$(cat "$cache/rustc-version" 2>/dev/null)" != "$rustc_now" ]; then
      why="Rust has changed"
    elif [ "$(du -sm "$target_dir" 2>/dev/null | cut -f1 || echo 0)" -gt 40000 ] 2>/dev/null; then
      why="it's past 40GB"
    fi
    if [ -n "$why" ]; then
      echo "Emptying the kept cargo builds ($target_dir): $why."
      # Something a cancelled run left running may still hold a file; cargo
      # rebuilds whatever's missing, so what's left doesn't matter.
      rm -rf "$target_dir" 2>/dev/null || echo "Some of it is still in use; left for now."
    fi
  fi
  mkdir -p "$target_dir"
  printf '%s\n' "$rustc_now" >"$cache/rustc-version"
  echo "Cargo builds into $target_dir, kept between runs ($(du -sh "$target_dir" 2>/dev/null | cut -f1 | tr -d " ") now)."
  {
    echo "CARGO_TARGET_DIR=$target_dir"
    # Incremental builds are for editing; they'd only grow the folder here.
    echo "CARGO_INCREMENTAL=0"
  } >>"$GITHUB_ENV"
fi

rustup component add clippy rustfmt
rustup target add wasm32-unknown-unknown
(cd site && npm ci)
