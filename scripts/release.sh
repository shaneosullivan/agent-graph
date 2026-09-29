#!/usr/bin/env bash
# =============================================================================
# release.sh
#
# Cuts a release of agent-graph for macOS and Linux, from this Mac:
#
#   1. Sets the version (Cargo.toml, Cargo.lock and the viewer's
#      WebAssembly, which records it), commits it and pushes it.
#   2. Waits for Chofter CI to build that commit, and downloads its builds
#      (scripts/fetch-ci-builds.sh: on the build machine's network).
#   3. Signs and notarizes the macOS builds (scripts/notarize-mac.sh).
#   4. Packs each build as agent-graph-<target>.tar.gz and uploads it to
#      $RELEASE_BUCKET, in releases/<version>/mac or releases/<version>/linux,
#      then downloads it again from its public URL to check it.
#   5. Writes site/release.json, which the site's downloads and /install.sh
#      use, and commits and pushes it: the site shows the release once
#      Vercel has deployed that.
#   6. Writes the Homebrew formula to $HOMEBREW_TAP (Formula/agent-graph.rb),
#      for macOS and Linux, and pushes it.
#
# Usage:
#   scripts/release.sh <version>          e.g. scripts/release.sh 0.1.0-beta.1
#
# Run again with the same version, it carries on: the version's already
# set, so it fetches that commit's builds, and does the rest again.
#
# It pushes no git tag: a version tag starts dist's release workflow
# (.github/workflows/release.yml), which publishes to GitHub Releases, npm
# and winget as well. Windows and npm aren't released this way yet.
#
# Settings, from the environment, or from .env.local at the repository's
# root (see .env.example), where the environment doesn't set them:
#   RELEASE_BUCKET   the bucket the archives go in (gs://…)
#   HOMEBREW_TAP     the tap's GitHub repository (owner/homebrew-<name>)
#   GCLOUD_ACCOUNT   the gcloud account to upload as (optional: gcloud's
#                    active one otherwise)
# and notarize-mac.sh's (NOTARY_PROFILE, APPLE_TEAM_ID: see its --help).
# =============================================================================

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
# cargo's own installs, and cargo itself when rustup doesn't put it on PATH.
PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1 && command -v rustup >/dev/null 2>&1; then
  PATH="$(dirname "$(rustup which cargo)"):$PATH"
fi

# .env.local's settings, each only where the environment doesn't set it
# (read, not run, as notarize-mac.sh does).
if [ -f .env.local ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line#"${line%%[![:space:]]*}"}"
    case "$line" in "" | "#"*) continue ;; esac
    key="${line%%=*}"
    value="${line#*=}"
    case "$key" in
      RELEASE_BUCKET | HOMEBREW_TAP | GCLOUD_ACCOUNT | NOTARY_PROFILE)
        value="${value%\"}"
        value="${value#\"}"
        [ -z "${!key:-}" ] && printf -v "$key" '%s' "$value"
        ;;
    esac
  done <.env.local
fi
RELEASE_BUCKET="${RELEASE_BUCKET:-}"
RELEASE_BUCKET="${RELEASE_BUCKET%/}"
HOMEBREW_TAP="${HOMEBREW_TAP:-}"
GCLOUD_ACCOUNT="${GCLOUD_ACCOUNT:-}"
NOTARY_PROFILE="${NOTARY_PROFILE:-AgentGraphNotary}"

# The builds released, by folder in the bucket.
MAC_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
LINUX_TARGETS=(aarch64-unknown-linux-musl x86_64-unknown-linux-musl)

case "${1:-}" in
  -h | --help | "")
    sed -n '3,38p' "$0" | sed 's/^# \{0,1\}//'
    [ -n "${1:-}" ] && exit 0 || exit 1
    ;;
esac
VERSION="${1#v}"

fail() {
  echo "" >&2
  echo "ERROR: $*" >&2
  exit 1
}
step() {
  echo ""
  echo "=== $* ==="
}

gcloud_() {
  if [ -n "$GCLOUD_ACCOUNT" ]; then
    gcloud --account="$GCLOUD_ACCOUNT" "$@"
  else
    gcloud "$@"
  fi
}

# Where target $1's build is once fetched (scripts/fetch-ci-builds.sh's
# layout: the ARM build as agent-graph, the x86_64 one as
# agent-graph-x86_64).
fetched() {
  local os
  case "$1" in
    *-apple-darwin) os=mac ;;
    *) os=linux ;;
  esac
  case "$1" in
    aarch64-*) echo "target/ci/$os/agent-graph" ;;
    *) echo "target/ci/$os/agent-graph-x86_64" ;;
  esac
}

echo "=== Agent Graph: release $VERSION (macOS and Linux) ==="

# =============================================================================
# CHECKS: everything the release needs, before it changes anything
# =============================================================================

[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] ||
  fail "'$VERSION' isn't a version (as 0.1.0, or 0.1.0-beta.1)."
[ "$(uname -s)" = Darwin ] || fail "Releasing needs macOS, to notarize the macOS builds."
for tool in gh gcloud git cargo npm node curl shasum tar xcrun uuidgen; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool isn't installed."
done

[ -n "$RELEASE_BUCKET" ] || fail "Set RELEASE_BUCKET (in .env.local): the gs:// bucket the archives go in."
[[ "$RELEASE_BUCKET" == gs://* ]] || fail "RELEASE_BUCKET should be a gs:// bucket, not $RELEASE_BUCKET."
[ -n "$HOMEBREW_TAP" ] || fail "Set HOMEBREW_TAP (in .env.local): the tap's repository, as owner/homebrew-tap."
[[ "$HOMEBREW_TAP" == */homebrew-* ]] || fail "HOMEBREW_TAP should be a repository named homebrew-<name>, not $HOMEBREW_TAP."

[ "$(git branch --show-current)" = main ] || fail "Release from main."
git diff --quiet && git diff --cached --quiet ||
  fail "There are uncommitted changes: commit or stash them first."
git fetch --quiet origin main
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] ||
  fail "main isn't the same as origin/main: pull or push first."

released="$(node -p "require('./site/release.json').version || ''")"
[ "$released" != "$VERSION" ] ||
  fail "$VERSION is already released (site/release.json)."

if ! gcloud_ storage ls "$RELEASE_BUCKET" >/dev/null 2>&1; then
  fail "gcloud can't list $RELEASE_BUCKET${GCLOUD_ACCOUNT:+ as $GCLOUD_ACCOUNT}. Log in with an account that can write to it: gcloud auth login${GCLOUD_ACCOUNT:+ $GCLOUD_ACCOUNT}"
fi
echo "✓ Bucket: $RELEASE_BUCKET"

if ! gh repo view "$HOMEBREW_TAP" >/dev/null 2>&1; then
  fail "No GitHub repository $HOMEBREW_TAP (or gh can't see it). Make it once, public: gh repo create $HOMEBREW_TAP --public"
fi
echo "✓ Tap: $HOMEBREW_TAP"

xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" >/dev/null 2>&1 ||
  fail "No working notarization credentials in the Keychain profile '$NOTARY_PROFILE': see scripts/notarize-mac.sh --help."
echo "✓ Notarization credentials: $NOTARY_PROFILE"

# =============================================================================
# 1. THE VERSION
# =============================================================================

current="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
if [ "$current" = "$VERSION" ]; then
  step "1. Version: already $VERSION (committed)"
else
  step "1. Version: $current → $VERSION"
  # The package's version: the first `version =` line, [package]'s.
  sed -i '' "1,/^version = /s/^version = \".*\"$/version = \"$VERSION\"/" Cargo.toml
  # Cargo.lock records it; so does the viewer's WebAssembly, which CI
  # checks is built from what's committed.
  cargo metadata --format-version 1 >/dev/null
  npm --prefix site run build-wasm
  git add Cargo.toml Cargo.lock site/public/viewer
  git commit --quiet -m "Release $VERSION"
  git push --quiet origin main
  echo "✓ Committed and pushed $(git rev-parse --short HEAD)"
fi
COMMIT="$(git rev-parse HEAD)"

# =============================================================================
# 2. CI'S BUILDS
# =============================================================================

step "2. Chofter CI's builds of ${COMMIT:0:7}"
scripts/fetch-ci-builds.sh "$COMMIT"
for target in "${MAC_TARGETS[@]}" "${LINUX_TARGETS[@]}"; do
  bin="$(fetched "$target")"
  [ -f "$bin" ] || fail "CI's run has no build for $target."
  # Each is built from this version: its --version is compiled in.
  LC_ALL=C grep -aqF "$VERSION" "$bin" ||
    fail "$bin ($target) isn't version $VERSION."
done
[ "$("$(fetched "${MAC_TARGETS[0]}")" --version)" = "agent-graph $VERSION" ] ||
  fail "$(fetched "${MAC_TARGETS[0]}") --version isn't 'agent-graph $VERSION'."
echo "✓ All four builds are $VERSION"

# =============================================================================
# 3. NOTARIZE
# =============================================================================

step "3. Notarizing the macOS builds"
bins=()
for target in "${MAC_TARGETS[@]}"; do
  bins+=(--bin "$target=$(fetched "$target")")
done
scripts/notarize-mac.sh "${bins[@]}"

# =============================================================================
# 4. PACK AND UPLOAD
# =============================================================================

step "4. Uploading to $RELEASE_BUCKET/releases/$VERSION"
out="target/release-$VERSION"
rm -rf "$out"
mkdir -p "$out"
bucket="${RELEASE_BUCKET#gs://}"
files="$out/files.tsv" # target, URL, SHA-256
: >"$files"

for target in "${MAC_TARGETS[@]}" "${LINUX_TARGETS[@]}"; do
  case "$target" in
    *-apple-darwin)
      os=mac
      bin="target/notarized/$target/agent-graph"
      ;;
    *)
      os=linux
      bin="$(fetched "$target")"
      ;;
  esac
  name="agent-graph-$target.tar.gz"
  dir="$out/$target"
  mkdir -p "$dir"
  cp "$bin" "$dir/agent-graph"
  chmod 755 "$dir/agent-graph"
  cp LICENSE "$dir/LICENSE"
  # No macOS metadata in the archive (._ files).
  COPYFILE_DISABLE=1 tar -czf "$out/$name" -C "$dir" agent-graph LICENSE
  sha="$(shasum -a 256 "$out/$name" | cut -d' ' -f1)"

  object="releases/$VERSION/$os/$name"
  # A Firebase Storage download token: the object's public URL has it, so
  # the bucket itself stays private.
  token="$(uuidgen | tr '[:upper:]' '[:lower:]')"
  gcloud_ storage cp "$out/$name" "$RELEASE_BUCKET/$object" \
    --content-type=application/gzip \
    --cache-control="public, max-age=31536000, immutable" \
    --custom-metadata="firebaseStorageDownloadTokens=$token" \
    --quiet
  url="https://firebasestorage.googleapis.com/v0/b/$bucket/o/${object//\//%2F}?alt=media&token=$token"

  # Downloaded again from that URL, as a user would.
  got="$(curl -fsSL --retry 3 "$url" | shasum -a 256 | cut -d' ' -f1)" ||
    fail "$name can't be downloaded from $url"
  [ "$got" = "$sha" ] || fail "$name downloaded from $url isn't what was uploaded."
  printf '%s\t%s\t%s\n' "$target" "$url" "$sha" >>"$files"
  echo "✓ $object"
done

# =============================================================================
# 5. THE SITE
# =============================================================================

step "5. site/release.json"
VERSION="$VERSION" COMMIT="$COMMIT" FILES="$files" node -e '
  const fs = require("fs");
  const files = {};
  for (const line of fs.readFileSync(process.env.FILES, "utf8").trim().split("\n")) {
    const [target, url, sha256] = line.split("\t");
    files[target] = {url, sha256};
  }
  const release = {
    version: process.env.VERSION,
    commit: process.env.COMMIT,
    date: new Date().toISOString().slice(0, 10),
    files,
  };
  fs.writeFileSync("site/release.json", JSON.stringify(release, null, 2) + "\n");
'
npx --prefix site prettier --write site/release.json >/dev/null
git add site/release.json
git commit --quiet -m "Release $VERSION: the site points at it"
# Anything CI pushed meanwhile, then this.
git pull --quiet --rebase origin main
git push --quiet origin main
echo "✓ Committed and pushed $(git rev-parse --short HEAD): the site shows $VERSION once Vercel has deployed it"

# =============================================================================
# 6. THE HOMEBREW TAP
# =============================================================================

step "6. Homebrew: $HOMEBREW_TAP"
url_of() { awk -F'\t' -v t="$1" '$1 == t { print $2 }' "$files"; }
sha_of() { awk -F'\t' -v t="$1" '$1 == t { print $3 }' "$files"; }
desc="$(sed -n 's/^description = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
homepage="$(sed -n 's/^homepage = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
license="$(sed -n 's/^license = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"

tap="$(mktemp -d)"
trap 'rm -rf "$tap"' EXIT
gh repo clone "$HOMEBREW_TAP" "$tap" -- --quiet 2>/dev/null
branch="$(gh repo view "$HOMEBREW_TAP" --json defaultBranchRef --jq '.defaultBranchRef.name // empty')"
branch="${branch:-main}"
git -C "$tap" checkout --quiet -B "$branch"
mkdir -p "$tap/Formula"
cat >"$tap/Formula/agent-graph.rb" <<EOF
# Written by agent-graph's scripts/release.sh: edits are overwritten.
class AgentGraph < Formula
  desc "$desc"
  homepage "$homepage"
  version "$VERSION"
  license "$license"

  on_macos do
    on_arm do
      url "$(url_of aarch64-apple-darwin)"
      sha256 "$(sha_of aarch64-apple-darwin)"
    end
    on_intel do
      url "$(url_of x86_64-apple-darwin)"
      sha256 "$(sha_of x86_64-apple-darwin)"
    end
  end

  on_linux do
    on_arm do
      url "$(url_of aarch64-unknown-linux-musl)"
      sha256 "$(sha_of aarch64-unknown-linux-musl)"
    end
    on_intel do
      url "$(url_of x86_64-unknown-linux-musl)"
      sha256 "$(sha_of x86_64-unknown-linux-musl)"
    end
  end

  def install
    bin.install "agent-graph"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/agent-graph --version")
  end
end
EOF
git -C "$tap" add Formula/agent-graph.rb
if git -C "$tap" diff --cached --quiet; then
  echo "✓ The formula's already $VERSION"
else
  git -C "$tap" commit --quiet -m "agent-graph $VERSION"
  git -C "$tap" push --quiet origin "HEAD:refs/heads/$branch"
  echo "✓ Pushed Formula/agent-graph.rb"
fi

# =============================================================================
# DONE
# =============================================================================

owner="${HOMEBREW_TAP%%/*}"
tap_name="${HOMEBREW_TAP#*/homebrew-}"
echo ""
echo "============================================"
echo "  Released agent-graph $VERSION (${COMMIT:0:7})"
echo "    brew tap $owner/$tap_name && brew trust $owner/$tap_name && brew install $owner/$tap_name/agent-graph"
echo "    Archives: $RELEASE_BUCKET/releases/$VERSION/"
echo "    Local copies: $out/"
echo "============================================"
