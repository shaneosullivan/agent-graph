# Building release binaries to test locally

How to get the same binaries a release ships ([release.md](release.md)) without publishing anything, mainly on an Apple Silicon Mac.

## Everything at once

On an Apple Silicon Mac, one script builds for macOS, Linux and Windows:

```bash
scripts/build-local.sh
```

It builds with the `dist` profile, as a release does, for every target a release ships: ARM and x86_64 for each. The builds are copied to a folder per platform, so they're easy to find:

| Platform | ARM | x86_64 |
|---|---|---|
| macOS | `target/dist/mac/agent-graph` | `target/dist/mac/agent-graph-x86_64` |
| Linux (statically linked) | `target/dist/linux/agent-graph` | `target/dist/linux/agent-graph-x86_64` |
| Windows | `target/dist/windows/agent-graph.exe` | `target/dist/windows/agent-graph-x86_64.exe` |

Each binary goes in as a new file, renamed over the old one, never copied over it: macOS remembers an executable's code signature by its file, so one overwritten in place is killed (`Killed: 9`) whenever it runs, and your hooks and viewer may be running `target/dist/mac/agent-graph`. If you copy a build there yourself, delete the old one first.

Then it runs what it can: the Mac builds (the Intel one with Rosetta, if it's installed), and the Linux builds in Docker if Docker is running.

The first time, install what it cross-compiles with (it says if any is missing, and adds the Rust targets itself):

```bash
brew install zig llvm
cargo install cargo-zigbuild cargo-xwin
```

The rest of this page is what the script does, step by step, and the other ways to get the binaries.

## The Mac build

This builds the binary a release ships for Apple Silicon, with the `dist` profile, and packages it as a release would:

```bash
dist build --artifacts=local --target aarch64-apple-darwin
```

(`brew install cargo-dist` installs `dist`.) It writes to `target/distrib/`:

- `agent-graph-aarch64-apple-darwin.tar.xz`, with the binary, the README and the LICENSE,
- its `.sha256`.

The binary on its own is at `target/aarch64-apple-darwin/dist/agent-graph`. For just that, without the tarball:

```bash
cargo build --profile dist --target aarch64-apple-darwin
```

### Running it

A binary you built, or downloaded with `gh` (below), isn't marked as downloaded from the internet, so macOS runs it without checking its signature:

```bash
target/aarch64-apple-darwin/dist/agent-graph --version
```

A local build is signed ad hoc, not with the Developer ID: that needs the certificate, which only CI has (release.md, "macOS: signing and notarizing"). To see how a binary is signed:

```bash
codesign -dvv agent-graph
```

A CI build signed for release names `Developer ID Application: … (TEAMID)` and has `runtime` in its flags.

### Testing it the way a package manager installs it

Homebrew keeps the binary in a folder named for its version and links to it from a folder on PATH. To check the hooks name the link, which upgrades keep, not the file it leads to (R61 in [review.md](review.md)):

```bash
mkdir -p /tmp/ag-test/Cellar/0.1.0 /tmp/ag-test/bin
cp target/aarch64-apple-darwin/dist/agent-graph /tmp/ag-test/Cellar/0.1.0/
ln -sf /tmp/ag-test/Cellar/0.1.0/agent-graph /tmp/ag-test/bin/agent-graph
PATH="/tmp/ag-test/bin:$PATH" /tmp/ag-test/Cellar/0.1.0/agent-graph install claude-code --dry-run
```

The `Hook command:` line should name `/tmp/ag-test/bin/agent-graph`. `--dry-run` changes nothing.

## Every platform, from CI

The release workflow runs on pull requests too, but by default only plans the release there. To have it build every target on a PR, and keep the builds, add to `dist-workspace.toml`:

```toml
pr-run-mode = "upload"
```

then regenerate the workflow:

```bash
dist generate
```

Open a pull request. When its Release run has finished, download the builds:

```bash
gh run download --dir dist-test --pattern 'artifacts-build-local-*'
```

`dist-test/` then has each target's `.tar.xz` (`.zip` on Windows) and `.sha256`, built exactly as a release builds them.

- A PR run doesn't create a Release, publish to Homebrew or npm, notarize, or open a winget pull request.
- The Mac build is signed with the Developer ID only if the `CODESIGN_*` secrets are set, and never for a pull request from a fork.
- Every PR's CI then takes about 10 minutes longer; turn it back off when you're done.
- While the repository is private, the `ubuntu-22.04-arm` job may wait for a runner that never comes.

## Linux and Windows, on the Mac

Plain `cargo build --target …` fails for these: `ring` needs a C compiler and linker for the target. The script brings them in with these tools.

**Linux**, with Zig as the C compiler and linker:

```bash
cargo zigbuild --profile dist --target aarch64-unknown-linux-musl
```

The binary is at `target/aarch64-unknown-linux-musl/dist/agent-graph`. Run it in Docker, natively on Apple Silicon:

```bash
docker run --rm -v "$PWD/target/aarch64-unknown-linux-musl/dist:/w" alpine /w/agent-graph --version
```

For x86_64, build `--target x86_64-unknown-linux-musl` and add `--platform linux/amd64` to `docker run` (emulated). Swap `alpine` for `ubuntu` or `debian` to try other distributions.

**Windows**, with cargo-xwin, as CI builds Windows on ARM. It downloads the Windows SDK itself, and uses LLVM's clang, which Homebrew doesn't put on PATH:

```bash
PATH="$(brew --prefix llvm)/bin:$PATH" cargo xwin build --cross-compiler clang --profile dist --target aarch64-pc-windows-msvc
```

`--cross-compiler clang` matters: cargo-xwin passes clang-cl's flags by default, but `ring` compiles its C for Windows on ARM with clang, which fails on them (`no such file or directory: '/imsvc'`). CI sets the same (`XWIN_CROSS_COMPILER=clang`, in `.github/build-setup.yml`).

The binary is at `target/aarch64-pc-windows-msvc/dist/agent-graph.exe`. Run it in a Windows 11 on ARM virtual machine (UTM or Parallels): `agent-graph.exe --version`. It isn't signed, so a copy that arrives through a browser gets a SmartScreen warning.

### If linking fails with `Library not loaded: @rpath/libLLVM.dylib`

Some Rust toolchains for macOS (1.98.1, for one) ship `rust-lld`, which links the WebAssembly and Windows binaries, looking for `libLLVM.dylib` in `lib/rustlib/<host>/lib`, but put the library in the toolchain's `lib`, so it can't start. (The same fault makes `rust-objcopy` fail, which cargo reports as a harmless warning: `stripping debug info with rust-objcopy failed`.) Reinstalling the toolchain doesn't help.

`scripts/fix-rust-lld.sh` links the library in where `rust-lld` looks, if it's missing there, and otherwise does nothing. This script, `scripts/test-all.sh` and the site's `npm run build-wasm` run it first. The link is in the toolchain, so a `rustup update` replaces it; the next build puts it back if the new toolchain needs it. To run it yourself:

```bash
scripts/fix-rust-lld.sh
```

If the link can't be made, this script still links Windows binaries, with Zig's copy of lld, through a small `lld-link` wrapper on PATH.
