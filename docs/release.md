# Releasing Agent Graph

Agent Graph ships as a prebuilt binary for each platform, so nobody needs Rust to install it. [dist](https://axodotdev.github.io/cargo-dist/) builds the binaries on each version tag, signs and notarizes the macOS ones, and publishes them to each package manager.

## What people install with

| Platform | Command | Where it comes from |
|---|---|---|
| macOS (also Linux) | `brew install shaneosullivan/tap/agent-graph` | The formula in [`shaneosullivan/homebrew-tap`](https://github.com/shaneosullivan/homebrew-tap), which each release updates |
| macOS and Linux | `curl --proto '=https' --tlsv1.2 -LsSf https://github.com/shaneosullivan/agent-graph/releases/latest/download/agent-graph-installer.sh \| sh` | The shell installer on the GitHub Release |
| Windows | `winget install ShaneOSullivan.AgentGraph` | [`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs), through a pull request each release opens |
| Windows | `powershell -ExecutionPolicy Bypass -c "irm https://github.com/shaneosullivan/agent-graph/releases/latest/download/agent-graph-installer.ps1 \| iex"` | The PowerShell installer on the GitHub Release |
| Anywhere with Node | `npm install -g agent-graph` | The `agent-graph` npm package, which downloads the binary for its platform from the GitHub Release |
| From source | `cargo install --path .` | This repository |

The shell and PowerShell installers put the binary in `$XDG_BIN_HOME`, or `~/.local/bin` (on Windows, `%USERPROFILE%\.local\bin`), and add that to PATH. They don't install an updater: to upgrade, run the installer again, or the package manager's upgrade.

The site's home page shows the same commands, in tabs (`site/app/install.tsx`).

## What's built

For each release, on GitHub's own runners:

| Target | Runner |
|---|---|
| `aarch64-apple-darwin` | `macos-14` |
| `x86_64-apple-darwin` | `macos-15-intel` |
| `x86_64-unknown-linux-musl` | `ubuntu-22.04` |
| `aarch64-unknown-linux-musl` | `ubuntu-22.04-arm` |
| `x86_64-pc-windows-msvc` | `windows-2022` |
| `aarch64-pc-windows-msvc` | `ubuntu-22.04`, cross-compiled with cargo-xwin |

The Linux binaries are linked statically against musl, so they run on any distribution, whatever its glibc. Each is a `.tar.xz` (a `.zip` on Windows) with a `.sha256` beside it. They're built with the `dist` profile (`Cargo.toml`), which is the release profile.

## The files

| File | What it is |
|---|---|
| `dist-workspace.toml` | dist's config: targets, installers, publish jobs, signing |
| `.github/workflows/release.yml` | **Generated** from the config by `dist generate`. Don't edit it: change the config and regenerate. |
| `.github/build-setup.yml` | A step dist adds before every build: it has macOS signing use the hardened runtime, which notarization requires, and has cargo-xwin pass clang's flags, not clang-cl's, which `ring` needs for Windows on ARM |
| `.github/workflows/notarize.yml` | Notarizes the macOS binaries (below). A publish job: the release isn't announced unless it succeeds. |
| `.github/workflows/winget.yml` | Opens the winget-pkgs pull request, once the release is announced |
| `wasm/Cargo.toml` | Marked `dist = false`: the WebAssembly crate is built for the site, not released |

To change what's built or published, edit `dist-workspace.toml`, then:

```bash
dist generate
```

(`brew install cargo-dist` installs `dist`; keep it at the version the config names, `cargo-dist-version`.) `dist plan` lists what a release would contain without building anything.

To build the binaries and try them before releasing, see [local-builds.md](local-builds.md).

## Making a release

1. Set the version in `Cargo.toml` (and let `Cargo.lock` follow: `cargo check`), and commit.
2. Tag that commit with the same version, and push the tag:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

`release.yml` then:

1. **plans** the release from the tag,
2. **builds** each target, signing the macOS binaries,
3. **hosts** them: creates the GitHub Release with the binaries, checksums and installers,
4. **publishes**, in parallel: the Homebrew formula to the tap, the npm package, and the notarization,
5. **announces** the release, once all of those have succeeded,
6. then opens the **winget** pull request.

A tag with a prerelease version (`v0.2.0-beta.1`) makes a prerelease, which isn't published to Homebrew, npm or winget.

## macOS: signing and notarizing

macOS refuses to run a binary downloaded by a browser unless it's signed with a Developer ID and notarized by Apple. (Homebrew, `curl` and npm don't mark what they download as from the internet, so their copies run either way, but a copy downloaded from the Release page wouldn't.)

- **Signing:** dist signs each macOS binary in its build job, with the certificate in the `CODESIGN_*` secrets, and (from `build-setup.yml`) `--options runtime`: the hardened runtime.
- **Notarizing:** `notarize.yml` downloads the macOS builds, checks each binary is signed with the Developer ID for the team in `CODESIGN_IDENTITY` and has the hardened runtime, zips it, and submits it with `xcrun notarytool submit --wait`, the way `syncawesome/scripts/export-macos-app.sh` does. If Apple doesn't accept one, the job prints Apple's log and fails, and the release isn't announced.
- **No stapling:** a ticket can't be stapled to a bare binary (only to an app, a disk image or an installer package). Nothing needs to be: Apple records the notarization against the binary's signature, and Gatekeeper looks it up online the first time a downloaded copy runs. So the published tarballs, their checksums and the Homebrew formula stay exactly as built.

To check a published binary's signature (it should name the Developer ID and `runtime` in its flags):

```bash
codesign -dvv agent-graph
```

and to check it's notarized, download its tarball from the Release page with a browser, unpack it, and run it: macOS asks only whether to open something downloaded from the internet, instead of refusing to.

To do the same on your own Mac, outside a release (say, to hand someone a binary), `scripts/notarize-mac.sh` builds both macOS targets, signs each with the Developer ID and the hardened runtime, has Apple notarize it, and checks the result with `syspolicy_check`. It needs a Developer ID certificate in your Keychain, and notarization credentials stored in a Keychain profile. It takes your account's details from the environment (`APPLE_TEAM_ID`, `NOTARY_PROFILE`, and `APPLE_ID` and `NOTARY_PASSWORD` to store the credentials the first time); `--help` lists them. The binaries go to `target/notarized/`.

## Windows

The Windows binaries aren't signed yet. winget and the PowerShell installer install them without a prompt, but a copy downloaded with a browser gets a SmartScreen warning the first time it runs, and Defender may be more suspicious of it. To sign them later, dist supports Azure Trusted Signing and SSL.com (`dist-workspace.toml`, `azure-windows-sign` or `ssldotcom-windows-sign`).

### winget

`winget.yml` uses [winget-releaser](https://github.com/vedantmgoyal9/winget-releaser) to add each release's two Windows zips to the package `ShaneOSullivan.AgentGraph`, by a pull request from your fork of `microsoft/winget-pkgs`. winget's maintainers review it, usually within a day or two; until it's merged, `winget install` gets the previous version.

It only updates a package that's already there, so the first version goes in by hand, after the first release:

```bash
komac new ShaneOSullivan.AgentGraph --version 0.1.0 \
  --urls https://github.com/shaneosullivan/agent-graph/releases/download/v0.1.0/agent-graph-x86_64-pc-windows-msvc.zip \
         https://github.com/shaneosullivan/agent-graph/releases/download/v0.1.0/agent-graph-aarch64-pc-windows-msvc.zip
```

(or `wingetcreate new` with the same URLs). Each zip holds a portable `agent-graph.exe`: winget installs it and links it from its `Links` folder, which is on PATH.

## Hooks survive upgrades

`agent-graph install claude-code` writes a command into Claude Code's settings that runs this binary by its full path, since hooks run with Claude Code's own PATH, which may not have it. A package manager keeps the binary in a folder named for its version (Homebrew's `Cellar/agent-graph/0.1.0/bin/`, WinGet's `Packages\…`) and links to it from a folder on PATH; an upgrade removes the old version's folder. So when the `agent-graph` on PATH is this binary, the hooks name it by that path, the link, which upgrades keep (`install::lasting_exe`, R61 in `review.md`). Otherwise, they name the binary's own path, as before.

Scoop's shims and npm's global folder don't change between versions, so they need nothing more.

## One-time setup

1. **Make this repository public.** A private repository's Releases can't be downloaded by anyone else, and every installer, the formula and the npm package download from them. (GitHub's ARM Linux runner, which `aarch64-unknown-linux-musl` builds on, may also not be available to a private repository.)
2. **Create [`shaneosullivan/homebrew-tap`](https://github.com/new)**: an empty public repository. dist writes `Formula/agent-graph.rb` to it.
3. **Fork [`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs)** to your account.
4. **Add the repository secrets** (Settings → Secrets and variables → Actions, or `gh secret set NAME`):

| Secret | What it is | How to get it |
|---|---|---|
| `CODESIGN_CERTIFICATE` | Your Developer ID Application certificate and its private key, as a base64-encoded .p12 | Keychain Access → My Certificates → right-click "Developer ID Application: …" → Export, as .p12 with a password; then `base64 -i cert.p12 \| gh secret set CODESIGN_CERTIFICATE` |
| `CODESIGN_CERTIFICATE_PASSWORD` | The .p12's password | The one you chose exporting it |
| `CODESIGN_IDENTITY` | The certificate's name | `security find-identity -v -p codesigning`: the `Developer ID Application: … (TEAMID)` line, without the hash |
| `APPLE_ID` | The developer account's Apple ID | |
| `APPLE_APP_SPECIFIC_PASSWORD` | An app-specific password for that Apple ID | [appleid.apple.com](https://appleid.apple.com) → Sign-In and Security → App-Specific Passwords |
| `HOMEBREW_TAP_TOKEN` | A token that can push to the tap | A fine-grained token with Contents: read and write on `homebrew-tap` |
| `NPM_TOKEN` | A token that can publish `agent-graph` | npmjs.com → Access Tokens → a granular token with publish access (the name was free when this was set up) |
| `WINGET_TOKEN` | A token for the winget pull request | A classic token with `public_repo` |

5. **Cut the first release** (above), then **submit the first winget version** by hand (above).

## Before the first release

- **The Windows ARM build is cross-compiled** (cargo-xwin on Linux), the build most likely to need fixing on the first run. It builds on a Mac with the same settings (`scripts/build-local.sh`). If it can't be made to work, dist can build it natively on `windows-11-arm` instead (`github-custom-runners` in the config).
