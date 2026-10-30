#!/usr/bin/env bash
# THE MAS LANE — the Mac App Store edition's build + submission (PLAN_MAS_FORK
# §3, 2026-10-30). The Developer-ID lane (release-app-sign.sh) is UNTOUCHED:
# same scripts, same chain, same cadence — this fork is ADDITIVE, and the two
# lanes share nothing but the tree.
#
# Usage:
#   scripts/release-app-store.sh --profile embedded.provisionprofile [--dry-run]
#
# ── WHAT THIS IS, AND WHAT IT IS NOT ───────────────────────────────────────
#
# The store edition is the SANDBOXED, UPDATER-FREE, Signal-free fork:
#   - built with the `mas` cargo feature (the updater plugin, its commands,
#     and its module are compiled OUT — guideline 2.5.2: the store owns
#     updates; an excision, not a runtime flag)
#   - built with `tauri.appstore.conf.json`: the DISTINCT bundle id
#     (`com.stewardmem.shell.mas` — store/DMG update paths never cross), the
#     sandbox entitlements, and the privacy manifest placed at
#     Contents/Resources/PrivacyInfo.xcprivacy
#   - packaged as a productbuild pkg (Apple's own installer tech — satisfies
#     2.4.5(ii) for submission packaging)
#   - NOT notarized: App Store review replaces notarization on this channel
#
# D6 DISCIPLINE, UNCHANGED AND EXTENDED: this lane runs OPERATOR-SIDE only.
# The public Actions never sign anything, never hold the Apple Distribution
# or Mac Installer Distribution certificates, never see the provisioning
# profile. There is no workflow that calls this script.
#
# ── THE FAIL-CLOSED PRECONDITIONS (each refusal names what is missing) ────
#
#   1. The Apple DISTRIBUTION identity ("Apple Distribution: Mark Fietje
#      (JHB4UNPL3G)") — NOT the Developer ID identity; the store re-signs,
#      but the upload must carry the distribution signature.
#   2. The provisioning profile for `com.stewardmem.shell.mas` (sandbox
#      capability on) — the operator exports it from the developer portal;
#      this lane validates that its embedded application-identifier matches
#      the overlay's bundle id BEFORE building, so a stale profile costs
#      nothing to discover.
#   3. `productbuild` and the Transporter (`iTMSTransporter`) for submission.
#
# ── THE ORDER ─────────────────────────────────────────────────────────────
#   preflight (identity, profile, tools) → build (mas feature + overlay +
#   entitlements, ad-hoc-rejected: the distribution identity signs) → verify
#   (entitlements ON the binary; sandbox key present; updater ABSENT; privacy
#   manifest present; the three Mach-Os distribution-signed) → pkg → submit.
#
set -euo pipefail

cd "$(dirname "$0")/.."

PROFILE=""
API_ISSUER=""
API_KEY=""
DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile) PROFILE="$2"; shift 2 ;;
    # The ASC API credentials (App Store Connect → Users and Access →
    # Integrations → App Store Connect API): the ISSUER id and the KEY id.
    # The .p8 PRIVATE key itself is fetched by Transporter from
    # ~/.appstoreconnect/private_keys/ (where Xcode/Transporter put it) or
    # via APPLE_API_KEY_PATH — the key IDs are identifiers, not secrets, but
    # they are passed as ARGUMENTS, never committed.
    --api-issuer) API_ISSUER="$2"; shift 2 ;;
    --api-key) API_KEY="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    *) echo "usage: $0 --profile <embedded.provisionprofile> [--api-issuer <id> --api-key <keyid>] [--dry-run]" >&2; exit 2 ;;
  esac
done

fail() { echo "error: $1" >&2; exit 1; }

# ── 1. The distribution identity ──────────────────────────────────────────
# The store's upload signature. NOT the Developer ID identity — a MAS upload
# signed with it is refused by ASC, and a refusal after the build is an hour
# of runners spent learning what `security` knew for free.
IDENTITY_LINE="$(security find-identity -v -p codesigning 2>/dev/null | grep 'Apple Distribution' | head -1 || true)"
[[ -n "$IDENTITY_LINE" ]] || fail "no 'Apple Distribution' codesigning identity in the login keychain — create one in the developer portal (PLAN_MAS_FORK §5.1). Found only: $(security find-identity -v -p codesigning 2>/dev/null | grep -c '"' || echo 0) identity/identities, none Apple Distribution."
# The identity NAME is what codesign and the bundler both take: strip the
# hash, keep the common name, and EXPORT it — the build below must sign with
# THIS identity, not fall back to ad-hoc (a silent ad-hoc fallback would
# fail only at ASC, the expensive place).
SIGNING_IDENTITY="${IDENTITY_LINE#*\"}"
SIGNING_IDENTITY="${SIGNING_IDENTITY%\"}"
export APPLE_SIGNING_IDENTITY="$SIGNING_IDENTITY"
echo ">> distribution identity: $SIGNING_IDENTITY"

# ── 2. The provisioning profile ───────────────────────────────────────────
[[ -n "$PROFILE" ]] || fail "no --profile given — export the provisioning profile for com.stewardmem.shell.mas from the developer portal (§5.3) and pass its path."
[[ -f "$PROFILE" ]] || fail "the provisioning profile does not exist: $PROFILE"
# The profile's application-identifier: the CMS-decoded profile is an XML
# plist whose key LITERALLY contains dots (`com.apple.application-identifier`),
# which plutil's key-path syntax cannot address (measured: every quoted-key
# path form returns "no value at that key path"). The grep is the extractor
# — proven against the real StewardMem_MAS profile on 2026-10-31.
PROFILE_APP_ID="$(security cms -D -i "$PROFILE" 2>/dev/null | grep -A 1 'com.apple.application-identifier' | tail -1 | sed -e 's/.*<string>//' -e 's|</string>.*||' -e 's/^[[:space:]]*//' || true)"
[[ "$PROFILE_APP_ID" == "JHB4UNPL3G.com.stewardmem.shell.mas" ]] || fail "the profile's application-identifier is '${PROFILE_APP_ID:-unreadable}' — expected JHB4UNPL3G.com.stewardmem.shell.mas. A stale or wrong-app profile must be refused BEFORE the build, not by ASC afterwards."
echo ">> profile verified: $PROFILE_APP_ID"

# ── 3. The tools ──────────────────────────────────────────────────────
command -v productbuild >/dev/null 2>&1 || fail "productbuild not found (Xcode command-line tools)."
# Transporter's CLI is not on the default PATH (it lives inside the app
# bundle); the lane extends PATH with the well-known location rather than
# requiring the operator to export anything.
if ! command -v iTMSTransporter >/dev/null 2>&1 && [[ -x "/Applications/Transporter.app/Contents/itms/bin/iTMSTransporter" ]]; then
  export PATH="/Applications/Transporter.app/Contents/itms/bin:$PATH"
fi
command -v iTMSTransporter >/dev/null 2>&1 || fail "iTMSTransporter not found — install the Transporter app (the submission tool); this lane does not invent a submission path."
# The INSTALLER signature (a second identity, a second portal act): ASC
# refuses an unsigned installer pkg, and `productbuild --sign` wants the
# installer certificate by name. NOT found under `-p codesigning`: the
# installer cert carries Apple's installer-signing EKU (measured:
# 1.2.840.113635.100.4.9), which the codesigning policy filter excludes by
# design — the first drill caught the lane refusing a valid identity. The
# unfiltered valid-identity list is the source.
INSTALLER_IDENTITY="$(security find-identity -v 2>/dev/null | grep -E 'Mac Installer Distribution|3rd Party Mac Developer Installer' | head -1 || true)"
[[ -n "$INSTALLER_IDENTITY" ]] || fail "no Mac Installer Distribution identity in the keychain — create one in the developer portal (§5.1); the store's installer pkg must carry it."
INSTALLER_NAME="${INSTALLER_IDENTITY#*\"}"
INSTALLER_NAME="${INSTALLER_NAME%\"}"
echo ">> installer identity: $INSTALLER_NAME"

if [[ "$DRY_RUN" == "1" ]]; then
  echo ">> dry-run: preflight green (identity, profile $PROFILE_APP_ID, tools). Stopping before the build."
  exit 0
fi

# ── 4. The build: mas feature + appstore overlay + the profile ────────────
# The profile overlay is GENERATED here (never committed): config merging is
# additive per key, so the profile rides its own tiny config whose only job
# is to place `embedded.provisionprofile` inside Contents/.
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp "$PROFILE" "$STAGE/embedded.provisionprofile"
cat > "$STAGE/tauri.appstore.profile.conf.json" <<JSON
{
  "bundle": {
    "macOS": {
      "files": {
        "embedded.provisionprofile": "$STAGE/embedded.provisionprofile"
      }
    }
  }
}
JSON

echo ">> building the MAS edition (mas feature, appstore overlay)…"
# `--target` is PINNED, not implied: without it the bundler outputs to
# target/release/bundle/ (no triple), while the verification below inspects
# the triple-qualified path — a mismatch that once had the lane verifying
# the DIRECT channel's old bundle (no entitlements) and refusing a perfect
# MAS build. The explicit triple makes the output path the one the lane
# verifies, and matches the direct lane's own convention.
( cd shell && pnpm tauri build \
    --target aarch64-apple-darwin \
    --config src-tauri/tauri.appstore.conf.json \
    --config "$STAGE/tauri.appstore.profile.conf.json" \
    --features mas )

APP="shell/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/StewardMem.app"
[[ -d "$APP" ]] || fail "the MAS bundle did not appear at $APP"

# ── 5. The verification: the store's shape, ON the bytes ──────────────────
echo ">> verifying the MAS edition's shape…"
# NOTE on the greps: NO `-q`. Under `set -o pipefail`, `grep -q` closing its
# stdin on the first match SIGPIPEs `codesign` mid-write (exit 141), and the
# pipeline reads as failure EVEN WHEN THE MATCH SUCCEEDED — the first drill
# failed on whichever arm matched first (the entitlements arm, then the
# identity arm). `grep >/dev/null` reads the whole stream; the exit code is
# then the match's alone.
codesign -d --entitlements :- "$APP" 2>/dev/null | grep 'com.apple.security.app-sandbox' >/dev/null \
  || fail "the bundle's entitlements do not carry app-sandbox — the store build is the sandboxed one; refusing to ship an unsandboxed edition."
codesign -dvv "$APP" 2>&1 | grep 'Authority=Apple Distribution' >/dev/null \
  || fail "the bundle is not signed with the Apple Distribution identity."
codesign -dvv "$APP" 2>&1 | grep 'Identifier=com.stewardmem.shell.mas' >/dev/null \
  || fail "the bundle id is not the store edition's — the overlay's distinct identifier did not apply."
[[ -f "$APP/Contents/Resources/PrivacyInfo.xcprivacy" ]] \
  || fail "the privacy manifest is missing from Contents/Resources — the ASC upload would take ITMS-91053."
plutil -p "$APP/Contents/Info.plist" | grep -q 'ITSAppUsesNonExemptEncryption' \
  || fail "ITSAppUsesNonExemptEncryption is missing from Info.plist."
# The excision, proven on the bytes: the updater's own strings (its source
# paths and its error prose, measured in the direct binary) must not appear
# in the mas binary at all — the plugin, its commands and its module are
# compiled out under the feature, and this is the post-build witness.
if strings "$APP/Contents/MacOS/brain-shell" \
    | grep -E 'tauri-plugin-updater|Updater does not have any endpoints' >/dev/null; then
  fail "the updater's code is IN the mas binary — the excision did not apply."
fi

echo ">> the MAS edition's shape is green: sandboxed, updater-free, manifest present."

# ── 6. The pkg ────────────────────────────────────────────────────────
echo ">> productbuild…"
PKG="target/StewardMem-MAS.pkg"
mkdir -p "$(dirname "$PKG")"
productbuild --component "$APP" /Applications "$PKG" --sign "$INSTALLER_NAME" \
  || fail "productbuild failed — the store's installer could not be built or signed."

# ── 7. The submission ───────────────────────────────────────────────
echo ">> submitting via Transporter (this is the operator's ASC record's door)…"
if [[ -z "$API_ISSUER" || -z "$API_KEY" ]]; then
  fail "no ASC API credentials given — pass --api-issuer <issuer-id> --api-key <key-id> (App Store Connect → Integrations → App Store Connect API; the .p8 private key must sit in ~/.appstoreconnect/private_keys/ or be given via APPLE_API_KEY_PATH). The legacy Apple-ID + app-specific-password shape is deliberately not offered: a password in a command line is the posture this repo declined."
fi
iTMSTransporter -m upload -assetFile "$PKG" \
  -apiIssuer "$API_ISSUER" -apiKey "$API_KEY" \
  || fail "the Transporter submission failed — inspect the ASC record; this lane does not retry."
echo ">> submitted. The ASC record, the 'Data Not Collected' privacy label, and the"
echo "   FoundationModels cloud entitlement request are OPERATOR acts (§5) — code cannot do them."
