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

rustup component add clippy rustfmt
rustup target add wasm32-unknown-unknown
(cd site && npm ci)
