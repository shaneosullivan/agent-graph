#!/usr/bin/env bash
# =============================================================================
# release.sh
#
# Cuts a release of agent-graph for macOS and Linux, from this Mac:
#
#   1. Sets the version (Cargo.toml and Cargo.lock), commits it and pushes
#      it. Nothing is built on this Mac.
#   2. Waits for Chofter CI's run of that commit to finish (at once, if it
#      has), and downloads the builds it made (scripts/fetch-ci-builds.sh:
#      on the build machine's network). The builds have to be of that
#      commit, so they're the version being released.
#   3. Signs and notarizes the macOS builds (scripts/notarize-mac.sh).
#   4. Packs each build as agent-graph-<target>.tar.gz and uploads it to
#      $RELEASE_BUCKET, in releases/<version>/mac or releases/<version>/linux,
#      then downloads it again from its public URL to check it.
#   5. Publishes the npm packages (scripts/npm-packages.mjs):
#      @chofter/agent-graph,
#      and the program for each platform, @chofter/agent-graph-<os>-<cpu>.
#   6. Writes site/release.json, which the site's downloads and /install.sh
#      use, and commits and pushes it: the site shows the release once
#      Vercel has deployed that.
#   7. Writes the Homebrew cask to $HOMEBREW_TAP (Casks/agent-graph.rb),
#      for macOS and Linux, and pushes it. A cask, not a formula: Homebrew
#      checks a formula without a bottle could be built from source, and
#      refuses it when the Command Line Tools are out of date, though this
#      only copies the program into place.
#   8. Waits until the site's install script (/install.sh) installs this
#      version: Vercel has deployed step 6's commit. Up to 15 minutes, then
#      it only warns.
#
# Usage:
#   scripts/release.sh <version>          e.g. scripts/release.sh 0.1.0-beta.1
#   scripts/release.sh <version> --no-npm  without step 5: nothing's
#                                         published to npm, and the site
#                                         says npm's coming, unless
#                                         @chofter/agent-graph <version>
#                                         is there
#   scripts/release.sh --tap-only         step 7 alone, for the release in
#                                         site/release.json
#
# Run again with the same version, it carries on: the version's already
# set, so it fetches that commit's builds, and does the rest again.
#
# It pushes no git tag: a version tag starts dist's release workflow
# (.github/workflows/release.yml), which publishes to GitHub Releases and
# winget as well. Windows isn't released this way yet.
#
# Settings, from the environment, or from .env.local at the repository's
# root (see .env.example), where the environment doesn't set them:
#   RELEASE_BUCKET   the bucket the archives go in (gs://…)
#   HOMEBREW_TAP     the tap's GitHub repository (owner/homebrew-<name>)
#   GCLOUD_ACCOUNT   the gcloud account to upload as (optional: gcloud's
#                    active one otherwise)
#   SITE_URL         the site whose install script is checked at the end
#                    (optional: https://agentgraph.chofter.com otherwise)
# and notarize-mac.sh's (NOTARY_PROFILE, APPLE_TEAM_ID: see its --help).
# npm must be logged in (npm login) as a member of the chofter org.
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
# The site whose install script is checked for the release at the end.
SITE_URL="${SITE_URL:-https://agentgraph.chofter.com}"
SITE_URL="${SITE_URL%/}"
GCLOUD_ACCOUNT="${GCLOUD_ACCOUNT:-}"
NOTARY_PROFILE="${NOTARY_PROFILE:-AgentGraphNotary}"

# The builds released, by folder in the bucket.
MAC_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
LINUX_TARGETS=(aarch64-unknown-linux-musl x86_64-unknown-linux-musl)

case "${1:-}" in
  -h | --help | "")
    sed -n '3,56p' "$0" | sed 's/^# \{0,1\}//'
    [ -n "${1:-}" ] && exit 0 || exit 1
    ;;
  --tap-only) VERSION="" ;;
  *) VERSION="${1#v}" ;;
esac

fail() {
  echo "" >&2
  echo "ERROR: $*" >&2
  exit 1
}
step() {
  echo ""
  echo "=== $* ==="
}

NPM=yes
case "${2:-}" in
  "") ;;
  --no-npm) NPM="" ;;
  *) fail "Unknown option: $2" ;;
esac

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

# Writes the cask for site/release.json's release to $HOMEBREW_TAP, and
# pushes it (removing the formula that earlier releases wrote).
update_tap() {
  local version tap branch desc homepage
  version="$(node -p "require('./site/release.json').version || ''")"
  [ -n "$version" ] || fail "site/release.json has no release to put in the tap."
  desc="$(sed -n 's/^description = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
  homepage="$(sed -n 's/^homepage = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"

  tap="$(mktemp -d)"
  gh repo clone "$HOMEBREW_TAP" "$tap" -- --quiet 2>/dev/null
  branch="$(gh repo view "$HOMEBREW_TAP" --json defaultBranchRef --jq '.defaultBranchRef.name // empty')"
  branch="${branch:-main}"
  git -C "$tap" checkout --quiet -B "$branch"
  mkdir -p "$tap/Casks"
  # Each archive's url and sha256, from site/release.json.
  DESC="$desc" HOMEPAGE="$homepage" node -e '
    const r = require("./site/release.json");
    const at = target => {
      const f = r.files[target];
      if (!f) throw new Error(`site/release.json has no ${target} archive`);
      return `      url "${f.url}"\n      sha256 "${f.sha256}"`;
    };
    process.stdout.write(`# Written by agent-graph'"'"'s scripts/release.sh: edits are overwritten.
cask "agent-graph" do
  version "${r.version}"

  on_macos do
    on_arm do
${at("aarch64-apple-darwin")}
    end
    on_intel do
${at("x86_64-apple-darwin")}
    end
  end

  on_linux do
    on_arm do
${at("aarch64-unknown-linux-musl")}
    end
    on_intel do
${at("x86_64-unknown-linux-musl")}
    end
  end

  name "Agent Graph"
  desc "${process.env.DESC}"
  homepage "${process.env.HOMEPAGE}"

  binary "agent-graph"
end
`);
  ' >"$tap/Casks/agent-graph.rb"
  git -C "$tap" add Casks/agent-graph.rb
  # A formula of the same name, as earlier releases wrote, would be picked
  # before the cask.
  if [ -f "$tap/Formula/agent-graph.rb" ]; then
    git -C "$tap" rm --quiet Formula/agent-graph.rb
  fi
  if git -C "$tap" diff --cached --quiet; then
    echo "✓ The cask's already $version"
  else
    git -C "$tap" commit --quiet -m "agent-graph $version"
    git -C "$tap" push --quiet origin "HEAD:refs/heads/$branch"
    echo "✓ Pushed Casks/agent-graph.rb ($version)"
  fi
  rm -rf "$tap"
}

# How to install from the tap: Homebrew loads nothing from it until it's
# trusted.
brew_steps() {
  local owner="${HOMEBREW_TAP%%/*}" name="${HOMEBREW_TAP#*/homebrew-}"
  echo "brew tap $owner/$name && brew trust $owner/$name && brew install --cask $owner/$name/agent-graph"
}

if [ -z "$VERSION" ]; then
  [[ "$HOMEBREW_TAP" == */homebrew-* ]] ||
    fail "Set HOMEBREW_TAP (in .env.local): the tap's repository, as owner/homebrew-tap."
  gh repo view "$HOMEBREW_TAP" >/dev/null 2>&1 || fail "gh can't see $HOMEBREW_TAP."
  echo "=== Agent Graph: the Homebrew tap, $HOMEBREW_TAP ==="
  update_tap
  echo "    $(brew_steps)"
  exit 0
fi

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

if [ -n "$NPM" ]; then
  npm_user="$(npm whoami 2>/dev/null)" || fail "npm isn't logged in: npm login"
  # A publishing token without the Organizations permission (enough to
  # publish) can't list the org's members: then it can't be checked.
  if members="$(npm org ls chofter "$npm_user" 2>/dev/null)"; then
    grep -q "$npm_user" <<<"$members" ||
      fail "npm's $npm_user isn't in the chofter org, which @chofter/agent-graph and its platform packages are published under."
    echo "✓ npm: $npm_user"
  else
    echo "✓ npm: $npm_user (its token can't list the chofter org's members, so that's left to npm publish)"
  fi
fi

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
  # Cargo.lock records it too. Nothing's built here: CI builds it (step 2).
  cargo metadata --format-version 1 >/dev/null
  git add Cargo.toml Cargo.lock
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
  # Without macOS's own metadata (extended attributes like
  # com.apple.provenance), which Linux's tar warns it doesn't know.
  COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -czf "$out/$name" -C "$dir" agent-graph LICENSE
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
# 5. NPM
# =============================================================================

if [ -z "$NPM" ]; then
  step "5. npm: skipped (--no-npm)"
else
  step "5. npm: @chofter/agent-graph $VERSION"
  builds=()
  for target in "${MAC_TARGETS[@]}" "${LINUX_TARGETS[@]}"; do
    builds+=("$target=$out/$target/agent-graph")
  done
  # A prerelease isn't what `npm install @chofter/agent-graph` gets: npm needs it
  # tagged as something else.
  case "$VERSION" in
    *-*) npm_tag=next ;;
    *) npm_tag=latest ;;
  esac
  # The platform packages first: @chofter/agent-graph depends on them. Listed first,
  # not read in the loop, so npm publish has the terminal: it asks for a
  # two-factor code only there.
  packages=()
  while IFS= read -r dir; do
    packages+=("$dir")
  done < <(node scripts/npm-packages.mjs "$VERSION" "$out/npm" "${builds[@]}")
  [ "${#packages[@]}" -gt 0 ] || fail "scripts/npm-packages.mjs made no packages."
  for dir in "${packages[@]}"; do
    name="$(node -p "require('./$dir/package.json').name")"
    # Run again, it carries on: a version can only be published once.
    if [ "$(npm view "$name@$VERSION" version 2>/dev/null)" = "$VERSION" ]; then
      echo "✓ $name@$VERSION (already published)"
      continue
    fi
    # ./ so npm reads it as a folder, not a GitHub repository. What it says
    # isn't hidden: with two-factor authentication, it asks for a code, or
    # gives a link to log in with.
    npm publish "./$dir" --access public --tag "$npm_tag"
    echo "✓ $name@$VERSION"
  done
fi
# The site offers npm once @chofter/agent-graph is there, at this version.
# Just published, npm can take a few minutes to show it (0.1.17 took four):
# up to ten, then it's left out.
on_npm=false
if [ -n "$NPM" ]; then
  for _ in $(seq 1 20); do
    if [ "$(npm view "@chofter/agent-graph@$VERSION" version 2>/dev/null)" = "$VERSION" ]; then
      on_npm=true
      break
    fi
    echo "Waiting for npm to show @chofter/agent-graph@$VERSION…"
    sleep 30
  done
  [ "$on_npm" = true ] ||
    echo "WARNING: npm doesn't show @chofter/agent-graph@$VERSION yet, so the site's npm tab says it's coming. Once it does, set \"npm\": true in site/release.json, and commit and push it."
elif [ "$(npm view "@chofter/agent-graph@$VERSION" version 2>/dev/null)" = "$VERSION" ]; then
  on_npm=true
fi

# =============================================================================
# 6. THE SITE
# =============================================================================

step "6. site/release.json"
VERSION="$VERSION" COMMIT="$COMMIT" FILES="$files" NPM="$on_npm" node -e '
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
    npm: process.env.NPM === "true",
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
# 7. THE HOMEBREW TAP
# =============================================================================

step "7. Homebrew: $HOMEBREW_TAP"
update_tap

# =============================================================================
# 8. THE SITE
# =============================================================================

# Released means installable: the site's install script, once Vercel has
# deployed release.json (and its cache, half a minute, has run out), installs
# this version. Up to 15 minutes; past that, it only warns.
step "8. Waiting for $SITE_URL/install.sh to install $VERSION"
served=""
for _ in $(seq 90); do
  if curl -fsSL "$SITE_URL/install.sh" 2>/dev/null | grep -qF "agent-graph $VERSION"; then
    served=yes
    break
  fi
  sleep 10
done
if [ -n "$served" ]; then
  echo "✓ $SITE_URL/install.sh installs $VERSION"
else
  echo "WARNING: after 15 minutes, $SITE_URL/install.sh doesn't install $VERSION yet. Check Vercel's deployment of $(git rev-parse --short HEAD)."
fi

# =============================================================================
# DONE
# =============================================================================

echo ""
echo "============================================"
echo "  Released agent-graph $VERSION (${COMMIT:0:7})"
echo "    $(brew_steps)"
echo "    Archives: $RELEASE_BUCKET/releases/$VERSION/"
echo "    Local copies: $out/"
echo "============================================"
