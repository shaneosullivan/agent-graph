#!/usr/bin/env bash
# Waits for Chofter CI's run of a commit to finish and, if it passed,
# downloads the six builds it made to target/ci/mac, linux and windows,
# laid out as scripts/build-local.sh lays out target/dist: the ARM build as
# agent-graph, and the x86_64 one beside it as agent-graph-x86_64.
#
#   scripts/fetch-ci-builds.sh [commit]    (HEAD by default: push it first)
#
# The run uploads artifacts.txt, which lists each build's URL on the build
# machine (scripts/ci-slow-checks.sh); those are served on its LAN, so this
# has to run there too. Needs gh, logged in, and curl.
set -euo pipefail
cd "$(dirname "$0")/.."

workflow=chofter-ci
sha="$(git rev-parse "${1:-HEAD}")"
out=target/ci

# The run can take a moment to appear after a push.
run=""
for _ in $(seq 60); do
  run="$(gh run list --workflow "$workflow" --commit "$sha" --limit 1 \
    --json databaseId --jq '.[0].databaseId // empty')"
  [ -n "$run" ] && break
  sleep 5
done
if [ -z "$run" ]; then
  echo "No $workflow run for ${sha:0:7} after 5 minutes. Has it been pushed?" >&2
  exit 1
fi

echo "Waiting for $workflow run $run (${sha:0:7})…"
if ! gh run watch "$run" --interval 30 --exit-status >/dev/null; then
  conclusion="$(gh run view "$run" --json conclusion --jq .conclusion)"
  echo "The run didn't pass ($conclusion): nothing downloaded. $(gh run view "$run" --json url --jq .url)" >&2
  exit 1
fi
echo "The run passed."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
gh run download "$run" --name artifacts --dir "$tmp"
list="$tmp/artifacts.txt"
if [ ! -s "$list" ]; then
  echo "The run uploaded no artifacts.txt." >&2
  exit 1
fi

# Where each build goes, from its name (agent-graph-<target>[.exe]).
dest() {
  local target="${1#agent-graph-}" os ext=""
  target="${target%.exe}"
  case "$target" in
    *-apple-darwin) os=mac ;;
    *-linux-*) os=linux ;;
    *-windows-*) os=windows ext=.exe ;;
    *) return 1 ;;
  esac
  case "$target" in
    aarch64-*) echo "$out/$os/agent-graph$ext" ;;
    *) echo "$out/$os/agent-graph-${target%%-*}$ext" ;;
  esac
}

while IFS= read -r url; do
  url="${url%$'\r'}"
  [ -n "$url" ] || continue
  name="${url##*/}"
  if ! bin="$(dest "$name")"; then
    echo "Skipping $url: not a build this knows where to put." >&2
    continue
  fi
  mkdir -p "$(dirname "$bin")"
  # Downloaded beside it, then renamed over it: macOS kills an executable
  # overwritten in place (see scripts/build-local.sh).
  if ! curl -fsSL --connect-timeout 10 --retry 3 -o "$bin.new.$$" "$url"; then
    rm -f "$bin.new.$$"
    echo "Couldn't download $url. Is this on the build machine's network, and is it serving the run's output (ChofterCI)?" >&2
    exit 1
  fi
  chmod +x "$bin.new.$$"
  mv -f "$bin.new.$$" "$bin"
  echo "$bin  ($name)"
done <"$list"
