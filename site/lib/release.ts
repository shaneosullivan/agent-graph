import type {Target} from "./analytics-core";
import published from "../release.json" with {type: "json"};

/**
 * The latest release, which the site points people at: release.json, which
 * scripts/release.sh writes when it publishes one (its version, the commit
 * it was built from, and each program's archive, with its SHA-256). Before
 * the first release, its version is null.
 *
 * Each archive is a .tar.gz holding the program, `agent-graph` (a .zip
 * holding `agent-graph.exe`, on Windows), kept in the release bucket
 * (releases/<version>/mac, linux or windows) and fetched by its Firebase
 * Storage download URL. A release from before Windows was has none for it.
 *
 * `npm` is whether it was published to npm too (`@chofter/agent-graph`, with its
 * program for each platform in @chofter/agent-graph-<os>-<cpu>).
 */

export type ReleaseFile = {url: string; sha256: string};
export type Release = {
  version: string;
  commit: string;
  date: string;
  files: Partial<Record<Target, ReleaseFile>>;
  npm?: boolean;
};

/** The latest release, or null before there's been one. */
export function latestRelease(json: unknown = published): Release | null {
  const r = json as Partial<Release> | null;
  return r?.version && r.files ? (r as Release) : null;
}

/** The public origin, for the commands the site shows. */
export const SITE_URL = (
  process.env.NEXT_PUBLIC_SITE_URL || "https://agentgraph.chofter.com"
).replace(/\/+$/, "");

/**
 * Installing with Homebrew, from the Chofter tap (github.com/chofter/homebrew-tap,
 * which scripts/release.sh updates). Homebrew loads nothing from a tap
 * outside its own until it's trusted: hence the middle line.
 */
export const BREW_COMMAND = [
  "brew tap chofter/tap",
  "brew trust chofter/tap",
  "brew install --cask chofter/tap/agent-graph",
].join("\n");

/** The command that installs the latest release (GET /install.sh). */
export const INSTALL_COMMAND = `curl -fsSL ${SITE_URL}/install.sh | sh`;

/** The same, with wget: some Linux (a fresh Ubuntu desktop) has no curl. */
export const WGET_INSTALL_COMMAND = `wget -qO- ${SITE_URL}/install.sh | sh`;

/** The command that installs it on Windows, in PowerShell (GET /install.ps1). */
export const POWERSHELL_COMMAND = `irm ${SITE_URL}/install.ps1 | iex`;

/** Whether `target` is a Windows program. */
const forWindows = (target: string) => target.endsWith("-windows-msvc");

/** Whether `release` has programs for Windows. */
export function hasWindows(release: Release | null): boolean {
  return Object.keys(release?.files ?? {}).some(forWindows);
}

/** Single-quoted for sh: every ' closed, escaped and reopened. */
const quote = (s: string) => `'${s.replace(/'/g, `'\\''`)}'`;

/**
 * A POSIX sh script that installs `release`'s program for the machine it
 * runs on (macOS or Linux, ARM or x86_64): downloaded, its SHA-256
 * checked, and put in $AGENT_GRAPH_INSTALL_DIR, $XDG_BIN_HOME or
 * ~/.local/bin, in that order. With no release, it says so and fails.
 */
export function installScript(release: Release | null): string {
  if (!release) {
    return `#!/bin/sh
echo "agent-graph hasn't been released yet: see ${SITE_URL}" >&2
exit 1
`;
  }
  const cases = Object.entries(release.files)
    .filter(([target]) => !forWindows(target))
    .map(
      ([target, f]) =>
        `  ${target}) url=${quote(f.url)} sha=${quote(f.sha256)} ;;`,
    )
    .join("\n");
  return `#!/bin/sh
# Installs agent-graph ${release.version} from ${SITE_URL}:
#   curl -fsSL ${SITE_URL}/install.sh | sh
# or, without curl:
#   wget -qO- ${SITE_URL}/install.sh | sh
# It goes in $AGENT_GRAPH_INSTALL_DIR, $XDG_BIN_HOME or ~/.local/bin.
set -eu

case "$(uname -s)" in
  Darwin) os=apple-darwin ;;
  Linux) os=unknown-linux-musl ;;
  *)
    echo "This installs agent-graph on macOS and Linux. On Windows, in PowerShell: ${POWERSHELL_COMMAND}" >&2
    exit 1
    ;;
esac
arch="$(uname -m)"
# A shell run under Rosetta says x86_64 on Apple silicon.
if [ "$os" = apple-darwin ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
  arch=arm64
fi
case "$arch" in
  arm64 | aarch64) arch=aarch64 ;;
  x86_64 | amd64) arch=x86_64 ;;
  *)
    echo "There's no agent-graph ${release.version} for $arch." >&2
    exit 1
    ;;
esac
target="$arch-$os"
case "$target" in
${cases}
  *)
    echo "There's no agent-graph ${release.version} for $target." >&2
    exit 1
    ;;
esac

# curl, or wget where there's no curl (a fresh Ubuntu desktop has only wget).
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL --retry 3 -o "$1" "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q --tries=3 -O "$1" "$2"; }
else
  echo "Installing agent-graph needs curl or wget, and this has neither." >&2
  exit 1
fi

dir="\${AGENT_GRAPH_INSTALL_DIR:-\${XDG_BIN_HOME:-$HOME/.local/bin}}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading agent-graph ${release.version} ($target)…"
fetch "$tmp/agent-graph.tar.gz" "$url"
if command -v sha256sum >/dev/null 2>&1; then
  got="$(sha256sum "$tmp/agent-graph.tar.gz" | cut -d' ' -f1)"
else
  got="$(shasum -a 256 "$tmp/agent-graph.tar.gz" | cut -d' ' -f1)"
fi
if [ "$got" != "$sha" ]; then
  echo "The download's SHA-256 isn't the release's: not installed." >&2
  exit 1
fi
tar -xzf "$tmp/agent-graph.tar.gz" -C "$tmp"

mkdir -p "$dir"
# Copied beside it, then renamed over it: macOS kills a program that's
# overwritten in place.
cp "$tmp/agent-graph" "$dir/.agent-graph.new.$$"
chmod 755 "$dir/.agent-graph.new.$$"
mv -f "$dir/.agent-graph.new.$$" "$dir/agent-graph"
echo "Installed $dir/agent-graph ($("$dir/agent-graph" --version))."

case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Add $dir to your PATH to run it as agent-graph." ;;
esac
echo "Next: agent-graph install claude-code (or codex)"
`;
}

/** Single-quoted for PowerShell: every ' doubled. */
const psQuote = (s: string) => `'${s.replace(/'/g, "''")}'`;

/**
 * A PowerShell script (Windows PowerShell 5.1, or PowerShell 7) that
 * installs `release`'s program for the Windows it runs on (ARM64 or x64):
 * downloaded, its SHA-256 checked, unpacked, and put in
 * $env:AGENT_GRAPH_INSTALL_DIR, or %USERPROFILE%\.local\bin, which is added
 * to the user's PATH (and this window's) if it isn't there. It runs in a
 * script block of its own, piped to `iex`: nothing's left defined in the
 * window, and a failure stops it, not the window. With no release, or none
 * for Windows, it says so.
 */
export function powerShellScript(release: Release | null): string {
  const windows = Object.entries(release?.files ?? {}).filter(([target]) =>
    forWindows(target),
  );
  if (!release || windows.length === 0) {
    const why = release
      ? `There's no agent-graph ${release.version} for Windows yet: see ${SITE_URL}`
      : `agent-graph hasn't been released yet: see ${SITE_URL}`;
    return `Write-Error ${psQuote(why)}\n`;
  }
  const builds = windows
    .map(
      ([target, f]) =>
        `    ${psQuote(target)} = @{ Url = ${psQuote(f.url)}; Sha = ${psQuote(f.sha256)} }`,
    )
    .join("\n");
  return `# Installs agent-graph ${release.version} from ${SITE_URL}, in PowerShell:
#   ${POWERSHELL_COMMAND}
# It goes in $env:AGENT_GRAPH_INSTALL_DIR, or %USERPROFILE%\\.local\\bin,
# which is added to your PATH.
& {
  $ErrorActionPreference = 'Stop'
  # (Windows PowerShell's progress bar slows a download down a lot.)
  $ProgressPreference = 'SilentlyContinue'
  # Windows PowerShell 5.1 may not offer TLS 1.2 by itself.
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

  $builds = @{
${builds}
  }
  # The machine's, not this PowerShell's: an x64 PowerShell on ARM64 runs
  # emulated.
  $arch = $null
  try { $arch = [string][System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture } catch {}
  if (-not $arch) { $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE } }
  $target = switch -Regex ($arch) {
    '^(Arm64|ARM64)$' { 'aarch64-pc-windows-msvc' }
    '^(X64|AMD64)$' { 'x86_64-pc-windows-msvc' }
    default { $null }
  }
  if (-not $target -or -not $builds.ContainsKey($target)) {
    Write-Error "There's no agent-graph ${release.version} for $arch."
    return
  }
  $build = $builds[$target]

  $default = Join-Path $env:USERPROFILE '.local\\bin'
  $dir = if ($env:AGENT_GRAPH_INSTALL_DIR) { $env:AGENT_GRAPH_INSTALL_DIR } else { $default }
  $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ('agent-graph-' + [guid]::NewGuid())
  New-Item -ItemType Directory -Force $tmp | Out-Null
  try {
    Write-Host "Downloading agent-graph ${release.version} ($target)..."
    $zip = Join-Path $tmp 'agent-graph.zip'
    Invoke-WebRequest -Uri $build.Url -OutFile $zip -UseBasicParsing
    if ((Get-FileHash $zip -Algorithm SHA256).Hash.ToLower() -ne $build.Sha) {
      Write-Error "The download's SHA-256 isn't the release's: not installed."
      return
    }
    Expand-Archive -Path $zip -DestinationPath $tmp -Force

    New-Item -ItemType Directory -Force $dir | Out-Null
    $exe = Join-Path $dir 'agent-graph.exe'
    # A running program can't be replaced, but can be renamed (watch-remote
    # running at login, say): the old one's moved aside, to be removed
    # next time, and keeps running until it stops.
    Get-ChildItem $dir -Filter 'agent-graph.exe.old-*' -ErrorAction SilentlyContinue |
      Remove-Item -Force -ErrorAction SilentlyContinue
    if (Test-Path $exe) {
      Move-Item $exe ($exe + '.old-' + [guid]::NewGuid().ToString('N').Substring(0, 8)) -Force
    }
    Move-Item (Join-Path $tmp 'agent-graph.exe') $exe -Force
    Write-Host "Installed agent-graph ${release.version} as $exe."

    $onPath = ($env:Path -split ';' | ForEach-Object { $_.TrimEnd('\\') }) -contains $dir.TrimEnd('\\')
    if (-not $onPath) {
      if ($dir -eq $default) {
        $user = [Environment]::GetEnvironmentVariable('Path', 'User')
        $parts = if ($user) { $user -split ';' | Where-Object { $_ } } else { @() }
        if (($parts | ForEach-Object { $_.TrimEnd('\\') }) -notcontains $dir.TrimEnd('\\')) {
          [Environment]::SetEnvironmentVariable('Path', ((@($dir) + $parts) -join ';'), 'User')
        }
        $env:Path = $dir + ';' + $env:Path
        Write-Host "Added $dir to your PATH (new windows have it too)."
      } else {
        Write-Host "Add $dir to your PATH to run it as agent-graph."
      }
    }
    Write-Host 'Next: agent-graph install claude-code (or codex)'
  } finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
  }
}
`;
}
