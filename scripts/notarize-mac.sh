#!/usr/bin/env bash
# =============================================================================
# notarize-mac.sh
#
# Builds the macOS binaries (Apple silicon and Intel), signs each with the
# Developer ID and the hardened runtime, and has Apple notarize them: what a
# release does in CI (dist, then .github/workflows/notarize.yml), on this Mac.
#
# Usage:
#   scripts/notarize-mac.sh              # build both, sign, notarize
#   scripts/notarize-mac.sh --no-build   # use the builds already in target/
#
# The notarized binaries go to target/notarized/<target>/agent-graph, and
# each beside it as the .zip Apple was sent (agent-graph-<target>.zip, as
# dist names a release's archives), which is a fine way to hand one to
# someone.
#
# A bare binary can't have its ticket stapled to it (only an app, a disk
# image or an installer package can), and doesn't need to: Apple records the
# notarization against the binary's signature, and Gatekeeper looks it up
# online the first time a downloaded copy runs. So there's no stapling step.
#
# Prerequisites:
#   - Xcode's command line tools (`xcode-select --install`)
#   - A "Developer ID Application" certificate in the Keychain
#   - Notarization credentials stored in the Keychain (see NOTARY_PROFILE)
#
# Settings, from the environment, or from .env.local at the repository's
# root (git ignores it; copy .env.example to make it), where the
# environment doesn't set them. Nothing
# about an account is kept in this script:
#   APPLE_TEAM_ID    the Apple Developer team to sign as. Optional with one
#                    Developer ID certificate in the Keychain: its team.
#   NOTARY_PROFILE   the Keychain profile holding the notarization
#                    credentials (default: AgentGraphNotary)
#   APPLE_ID         the developer account's Apple ID, and
#   NOTARY_PASSWORD  an app-specific password for it: only to store the
#                    credentials in NOTARY_PROFILE, the first time
# =============================================================================

set -euo pipefail

# =============================================================================
# CONFIGURATION (see the settings above)
# =============================================================================

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# .env.local's settings (KEY=value lines; # comments), each only where the
# environment doesn't already set it. Read, not run: it's only settings.
if [ -f "$REPO_ROOT/.env.local" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line#"${line%%[![:space:]]*}"}"
    case "$line" in "" | "#"*) continue ;; esac
    key="${line%%=*}"
    value="${line#*=}"
    case "$key" in
      APPLE_TEAM_ID | APPLE_ID | NOTARY_PROFILE | NOTARY_PASSWORD)
        # Quotes around the value are dropped.
        value="${value%\"}"
        value="${value#\"}"
        [ -z "${!key:-}" ] && printf -v "$key" '%s' "$value"
        ;;
    esac
  done <"$REPO_ROOT/.env.local"
fi

APPLE_TEAM_ID="${APPLE_TEAM_ID:-}"
APPLE_ID="${APPLE_ID:-}"
# The Keychain profile notarytool reads the credentials from. To store one,
# once, with an app-specific password (appleid.apple.com → Sign-In and
# Security → App-Specific Passwords):
#   APPLE_ID=you@example.com NOTARY_PASSWORD=xxxx-xxxx-xxxx-xxxx scripts/notarize-mac.sh
# or store it yourself:
#   xcrun notarytool store-credentials AgentGraphNotary \
#     --apple-id you@example.com --team-id TEAMID --password xxxx-xxxx-xxxx-xxxx
# A profile is the account's, not an app's: one stored for another app works.
NOTARY_PROFILE="${NOTARY_PROFILE:-AgentGraphNotary}"
NOTARY_PASSWORD="${NOTARY_PASSWORD:-}"

# =============================================================================
# PATHS
# =============================================================================

OUTPUT_DIR="$REPO_ROOT/target/notarized"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

BUILD=1
for arg in "$@"; do
  case "$arg" in
    --no-build) BUILD=0 ;;
    -h | --help)
      sed -n '3,26p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown option: $arg (see --help)" >&2
      exit 1
      ;;
  esac
done

cd "$REPO_ROOT"
# cargo's own installs, and cargo itself when rustup doesn't put it on PATH.
PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1 && command -v rustup >/dev/null 2>&1; then
  PATH="$(dirname "$(rustup which cargo)"):$PATH"
fi

echo "=== Agent Graph: notarize the macOS binaries ==="
echo ""

# =============================================================================
# VALIDATION
# =============================================================================

if [ "$(uname -s)" != Darwin ]; then
  echo "ERROR: signing and notarizing need macOS (codesign, notarytool)." >&2
  exit 1
fi

# The Developer ID certificates in the Keychain, by their full names (as
# codesign takes them), each ending with its team in parentheses; APPLE_TEAM_ID's
# only, if it's set.
IDENTITIES=()
while IFS= read -r identity; do
  IDENTITIES+=("$identity")
done < <(security find-identity -v -p codesigning |
  sed -n 's/.*"\(Developer ID Application: .*([A-Z0-9]*)\)".*/\1/p' |
  sort -u | grep -F "(${APPLE_TEAM_ID}" || true)
if [ ${#IDENTITIES[@]} -eq 0 ]; then
  echo "ERROR: No 'Developer ID Application' certificate${APPLE_TEAM_ID:+ for team $APPLE_TEAM_ID} in the Keychain." >&2
  echo "" >&2
  echo "To create one:" >&2
  echo "  1. Open Xcode → Settings → Accounts → select the team" >&2
  echo "  2. Manage Certificates → '+' → 'Developer ID Application'" >&2
  echo "  Or: https://developer.apple.com/account/resources/certificates" >&2
  exit 1
fi
if [ ${#IDENTITIES[@]} -gt 1 ]; then
  echo "ERROR: More than one Developer ID certificate in the Keychain; say which team with APPLE_TEAM_ID:" >&2
  printf '  %s\n' "${IDENTITIES[@]}" >&2
  exit 1
fi
IDENTITY="${IDENTITIES[0]}"
# The team, from the certificate's name, when it wasn't given.
APPLE_TEAM_ID=$(sed -n 's/.*(\([A-Z0-9]*\))$/\1/p' <<<"$IDENTITY")
echo "✓ Signing as: $IDENTITY"

if [ -n "$NOTARY_PASSWORD" ]; then
  if [ -z "$APPLE_ID" ]; then
    echo "ERROR: Storing the credentials needs the account's Apple ID too: set APPLE_ID." >&2
    exit 1
  fi
  echo ""
  echo "=== Storing notarization credentials in the Keychain ($NOTARY_PROFILE) ==="
  xcrun notarytool store-credentials "$NOTARY_PROFILE" \
    --apple-id "$APPLE_ID" \
    --team-id "$APPLE_TEAM_ID" \
    --password "$NOTARY_PASSWORD"
  echo "✓ Stored. Next time, NOTARY_PASSWORD isn't needed."
fi

# Checked now, not after the build: a quick request with the stored
# credentials (the last submission, if any).
if ! xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" >/dev/null 2>&1; then
  echo "ERROR: No working notarization credentials in the Keychain profile '$NOTARY_PROFILE'." >&2
  echo "" >&2
  echo "Store them once, with an app-specific password" >&2
  echo "(appleid.apple.com → Sign-In and Security → App-Specific Passwords):" >&2
  echo "  APPLE_ID=you@example.com NOTARY_PASSWORD=xxxx-xxxx-xxxx-xxxx scripts/notarize-mac.sh" >&2
  echo "Or name a profile you've already stored (one for another app works):" >&2
  echo "  NOTARY_PROFILE=<its name> scripts/notarize-mac.sh" >&2
  exit 1
fi
echo "✓ Notarization credentials: $NOTARY_PROFILE"

# =============================================================================
# BUILD
# =============================================================================

if [ "$BUILD" = 1 ]; then
  for target in "${TARGETS[@]}"; do
    echo ""
    echo "=== Building $target (dist profile, as a release does) ==="
    rustup target add "$target" >/dev/null
    cargo build --profile dist --target "$target"
  done
fi

# =============================================================================
# SIGN
# =============================================================================

rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

for target in "${TARGETS[@]}"; do
  built="$REPO_ROOT/target/$target/dist/agent-graph"
  bin="$OUTPUT_DIR/$target/agent-graph"
  if [ ! -f "$built" ]; then
    echo "ERROR: $built isn't built. Run without --no-build." >&2
    exit 1
  fi
  echo ""
  echo "=== Signing $target ==="
  mkdir -p "$OUTPUT_DIR/$target"
  # A copy, as a new file: macOS remembers an executable's signature by its
  # file, so re-signing one that's run may get it killed next time it runs.
  cp "$built" "$bin"
  # The hardened runtime and a secure timestamp: notarization refuses a
  # binary without either.
  codesign --force --sign "$IDENTITY" --options runtime --timestamp \
    --identifier agent-graph "$bin"
  codesign --verify --strict --verbose=2 "$bin"
  info=$(codesign -dvv "$bin" 2>&1)
  grep -q 'flags=.*runtime' <<<"$info" || {
    echo "ERROR: $bin: no hardened runtime" >&2
    exit 1
  }
  echo "✓ Signed, with the hardened runtime"
done

# =============================================================================
# NOTARIZE
# =============================================================================

for target in "${TARGETS[@]}"; do
  name=$target
  bin="$OUTPUT_DIR/$target/agent-graph"
  zip="$OUTPUT_DIR/agent-graph-$target.zip"
  echo ""
  echo "=== Notarizing $target (this can take a few minutes) ==="
  # notarytool takes a zip, a disk image or a package, not a bare binary.
  ditto -c -k --keepParent "$bin" "$zip"
  result=$(xcrun notarytool submit "$zip" \
    --keychain-profile "$NOTARY_PROFILE" \
    --wait --output-format json)
  status=$(sed -n 's/.*"status" *: *"\([^"]*\)".*/\1/p' <<<"$result" | head -n 1)
  id=$(sed -n 's/.*"id" *: *"\([^"]*\)".*/\1/p' <<<"$result" | head -n 1)
  if [ "$status" != Accepted ]; then
    echo "$result" >&2
    echo "" >&2
    echo "ERROR: Apple didn't accept $name ($status). Its log:" >&2
    [ -n "$id" ] && xcrun notarytool log "$id" --keychain-profile "$NOTARY_PROFILE" >&2 || true
    exit 1
  fi
  echo "✓ Notarized (submission $id)"

  # What Gatekeeper will make of a downloaded copy: it asks Apple for the
  # ticket, as a Mac would the first time the copy runs.
  if command -v syspolicy_check >/dev/null 2>&1; then
    syspolicy_check distribution "$bin"
    echo "✓ Ready for distribution"
  fi
done

# =============================================================================
# DONE
# =============================================================================

echo ""
echo "============================================"
echo "  Notarized:"
for target in "${TARGETS[@]}"; do
  echo "    $OUTPUT_DIR/$target/agent-graph"
  echo "    $OUTPUT_DIR/agent-graph-$target.zip"
done
echo "============================================"
