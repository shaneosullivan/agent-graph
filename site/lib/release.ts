import type {Target} from "./analytics-core";
import published from "../release.json" with {type: "json"};

/**
 * The latest release, which the site points people at: release.json, which
 * scripts/release.sh writes when it publishes one (its version, the commit
 * it was built from, and each program's archive, with its SHA-256). Before
 * the first release, its version is null.
 *
 * Each archive is a .tar.gz holding the program, `agent-graph`, kept in the
 * release bucket (releases/<version>/mac or linux) and fetched by its
 * Firebase Storage download URL.
 */

export type ReleaseFile = {url: string; sha256: string};
export type Release = {
  version: string;
  commit: string;
  date: string;
  files: Partial<Record<Target, ReleaseFile>>;
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
    .map(
      ([target, f]) =>
        `  ${target}) url=${quote(f.url)} sha=${quote(f.sha256)} ;;`,
    )
    .join("\n");
  return `#!/bin/sh
# Installs agent-graph ${release.version} from ${SITE_URL}:
#   curl -fsSL ${SITE_URL}/install.sh | sh
# It goes in $AGENT_GRAPH_INSTALL_DIR, $XDG_BIN_HOME or ~/.local/bin.
set -eu

case "$(uname -s)" in
  Darwin) os=apple-darwin ;;
  Linux) os=unknown-linux-musl ;;
  *)
    echo "agent-graph ${release.version} is for macOS and Linux only, for now." >&2
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

dir="\${AGENT_GRAPH_INSTALL_DIR:-\${XDG_BIN_HOME:-$HOME/.local/bin}}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading agent-graph ${release.version} ($target)…"
curl -fsSL --retry 3 -o "$tmp/agent-graph.tar.gz" "$url"
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
