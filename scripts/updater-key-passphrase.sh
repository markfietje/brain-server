#!/usr/bin/env bash
# III-7 — set a NEW passphrase for the updater's signing key, and keep the
# login keychain as its home.
#
# Usage:
#   scripts/updater-key-passphrase.sh [--key <file>] [--service <name>] [--account <name>]
#
# WHAT IT DOES, and why each step is there:
#
#   1. `minisign -C -s <key>` re-encrypts the SECRET key under a NEW
#      passphrase. **It asks for the OLD passphrase first**, then the new one
#      twice (measured 2026-10-22) — and it changes ONLY the encryption.
#   2. The key is re-encoded into the BASE64 form the Tauri CLI reads, with
#      **no trailing newline** — BSD `base64` adds one, and the CLI's
#      decoder rejects it (`Invalid symbol 10, offset 348`).
#   3. The new passphrase is stored in the LOGIN KEYCHAIN, under the item
#      `app.brain.shell/updater-signing` / `default`, which is where
#      `scripts/release-app-sign.sh` reads it from.
#
# **THE PUBLIC KEY DOES NOT CHANGE, and that is the point.** A passphrase
# protects the SECRET half; the keypair is the same keypair afterwards.
# Measured: after `minisign -C`, the recreated public half carries the SAME
# payload (`RWTVYmAZER58wru0LpbhsAatm/WMC7Td0pXawykWLGmTii1wekuICRR9`) —
# so **the key the application pins stays valid and nothing has to be
# re-pinned**. A passphrase change is NOT a re-key.
#
# NOTHING SECRET IS EVER IN THIS FILE, and nothing secret is ever printed:
# the passphrase is read from the keychain and handed to `minisign` on stdin.
# The old passphrase is prompted for (or read from the keychain), and the new
# one is prompted for twice.
#
# The signing key's own custody is untouched: the RAW form
# (`~/.config/brain-server/updater.key.minisign`) is what `minisign` edits,
# and the BASE64 form (`~/.config/brain-server/updater.key`) is what the Tauri
# CLI reads. Both are re-derived from the same key.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEY_DIR="${HOME}/.config/brain-server"
RAW_KEY="${KEY_DIR}/updater.key.minisign"
B64_KEY="${KEY_DIR}/updater.key"
SERVICE="app.brain.shell/updater-signing"
ACCOUNT="default"

die() {
	echo "error: $*" >&2
	exit 1
}

usage() {
	cat >&2 <<'USAGE'
usage: scripts/updater-key-passphrase.sh [--key <file>] [--service <name>] [--account <name>]

  --key <file>      the RAW secret key to re-encrypt
                    (default: ~/.config/brain-server/updater.key.minisign)
  --service <name>  the keychain item's service
                    (default: app.brain.shell/updater-signing)
  --account <name>  the keychain item's account (default: default)
USAGE
	exit 2
}

while [[ $# -gt 0 ]]; do
	case "$1" in
		--key) RAW_KEY="${2:-}"; shift 2;;
		--service) SERVICE="${2:-}"; shift 2;;
		--account) ACCOUNT="${2:-}"; shift 2;;
		-h|--help) usage;;
		*) usage;;
	esac
done

[[ "$(uname -s)" == "Darwin" ]] || die "the login keychain is macOS-only; on another platform keep the passphrase in a 0600 file and pass --passphrase-file"
command -v minisign >/dev/null 2>&1 || die "minisign is not installed (brew install minisign)"
command -v security >/dev/null 2>&1 || die "the 'security' command is missing; this script stores the passphrase in the login keychain"
[[ -f "$RAW_KEY" ]] || die "the secret key is absent: $RAW_KEY"

# The BASE64 form's name is the RAW form's name with `.minisign` removed —
# which is the operator's own layout: `updater.key.minisign` (raw, what
# minisign edits) beside `updater.key` (base64, what the Tauri CLI reads).
B64_KEY="${RAW_KEY%.minisign}"
[[ "$B64_KEY" != "$RAW_KEY" ]] || B64_KEY="${RAW_KEY%.rs}.key"

echo ">> the signing key: $RAW_KEY"
echo ">> the passphrase's home: login keychain item '$SERVICE' / '$ACCOUNT'"

# ── 1 · THE OLD PASSPHRASE ─────────────────────────────────────────────────
#
# It is read from the keychain (the same place the new one will go), so the
# operator is not asked for it twice. When the item does not exist, it is
# PROMPTED for here rather than left to `minisign` — because `minisign`
# asking for it in the MIDDLE of the sequence makes the stdin order
# depend on whether the item exists, and a wrong order is a failed
# re-encryption with a confusing error.
OLD_PASSPHRASE=""
if command -v security >/dev/null 2>&1; then
	OLD_PASSPHRASE="$(security find-generic-password -s "$SERVICE" -a "$ACCOUNT" -w 2>/dev/null || true)"
fi
if [[ -n "$OLD_PASSPHRASE" ]]; then
	echo ">> the current passphrase is in the keychain; using it to re-encrypt the key"
else
	echo -n ">> CURRENT passphrase (nothing is echoed): " >&2
	IFS= read -rs OLD_PASSPHRASE || true
	echo "" >&2
	[[ -n "$OLD_PASSPHRASE" ]] || die "the current passphrase is empty"
	echo ">> using the current passphrase from the terminal"
fi

# ── 2 · THE NEW PASSPHRASE ─────────────────────────────────────────────────
#
# Prompted for TWICE, and minisign asks for it twice as well — the two must
# agree. Nothing is echoed. The prompt reads from stdin, so it works on a
# terminal AND on a pipe; at EOF the passphrase comes back EMPTY and the
# script refuses, rather than hanging.
echo -n ">> NEW passphrase (twice, nothing is echoed): " >&2
IFS= read -rs NEW_PASSPHRASE || true
echo "" >&2
echo -n ">> again: " >&2
IFS= read -rs NEW_PASSPHRASE_CONFIRM || true
echo "" >&2
[[ -n "$NEW_PASSPHRASE" ]] || die "the new passphrase is empty"
[[ "$NEW_PASSPHRASE" == "$NEW_PASSPHRASE_CONFIRM" ]] || die "the two new passphrases do not match"
[[ "$NEW_PASSPHRASE" != "$OLD_PASSPHRASE" ]] || die "the new passphrase is the old one"

# ── 3 · RE-ENCRYPT ─────────────────────────────────────────────────────────
#
# `minisign -C` reads: the OLD passphrase, then the NEW one, then the NEW
# one again. The raw key is copied first so a failed run leaves the
# operator's key untouched.
WORK="$(mktemp -d)"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT
cp "$RAW_KEY" "$WORK/key.raw"
chmod 600 "$WORK/key.raw"

echo ">> re-encrypting the key…"
{
	printf '%s\n' "$OLD_PASSPHRASE"
	printf '%s\n' "$NEW_PASSPHRASE"
	printf '%s\n' "$NEW_PASSPHRASE"
} | minisign -C -s "$WORK/key.raw" >/dev/null

# ── 4 · THE BASE64 FORM THE TAURI CLI READS ───────────────────────────────
#
# The trailing newline is REMOVED on purpose: BSD `base64` ends with one and
# the Tauri CLI's decoder rejects it (`Invalid symbol 10, offset 348`).
base64 -i "$WORK/key.raw" | tr -d '\n' > "$WORK/key.b64"

# ── 5 · THE PUBLIC HALF MUST NOT HAVE MOVED ────────────────────────────────
#
# The proof that this was a passphrase change and NOT a re-key: the recreated
# public half carries the same payload. (The public half's FILE form differs
# from the stored base64 form — `minisign -R` writes the raw two lines — so
# the comparison is made on the DECODED text.)
if [[ -f "$B64_KEY.pub" ]]; then
	minisign -R -s "$WORK/key.raw" -p "$WORK/pub.after" >/dev/null 2>&1 <<<"$NEW_PASSPHRASE" || true
	if [[ -f "$WORK/pub.after" ]]; then
		BEFORE="$(base64 -d < "$B64_KEY.pub" 2>/dev/null | tr -d '\n' || true)"
		AFTER="$(cat "$WORK/pub.after" | tr -d '\n')"
		# Compare the PAYLOAD: the second line of each, which is the key
		# itself. The comments differ in spelling between the two writers.
		BEFORE_KEY="$(printf '%s\n' "$BEFORE" | sed -n '2p')"
		AFTER_KEY="$(printf '%s\n' "$AFTER" | sed -n '2p')"
		if [[ -n "$BEFORE_KEY" && "$BEFORE_KEY" != "$AFTER_KEY" ]]; then
			die "the keypair MOVED: the public half changed, so the application's pinned key would have to be re-pinned. Nothing was written."
		fi
		echo ">> the keypair did NOT move: the public half is unchanged, so the key the application pins stays valid"
	fi
fi

# ── 6 · WRITE IT ───────────────────────────────────────────────────────────
#
# The RAW form first (it is the key), then the base64 form, then the
# keychain. The keychain is written LAST: if it fails, the key still works
# with the passphrase the operator just chose, and the item can be re-added
# by hand.
install -m 600 "$WORK/key.raw" "$RAW_KEY"
install -m 600 "$WORK/key.b64" "$B64_KEY"
echo ">> re-encrypted: $RAW_KEY"
echo ">> re-encoded:   $B64_KEY"

# The keychain is written LAST: if it fails, the key still works with the
# passphrase the operator just chose, and the item can be re-added by hand.
#
# **THE PASSPHRASE IS NEVER ON THE COMMAND LINE.** `security`'s own help
# says it: "Use of the -p or -w options is insecure. Specify -w as the last
# option to be prompted." So `-w` is passed with NO value and the value is
# fed on stdin — `security` asks for it TWICE ("password data for new item:"
# and "retype password for new item:"), which is also the check that the two
# entries agree.
printf '%s\n%s\n' "$NEW_PASSPHRASE" "$NEW_PASSPHRASE" \
	| security add-generic-password -U -s "$SERVICE" -a "$ACCOUNT" -w >/dev/null 2>&1 \
	|| die "the login keychain refused the item: $SERVICE / $ACCOUNT"
echo ">> stored in the login keychain: $SERVICE / $ACCOUNT"
echo
echo "The key is unchanged; only its passphrase is. The application pins the PUBLIC"
echo "half, and that did not move — nothing needs re-pinning."
