#!/usr/bin/env bash
# III-7 — the SIGNING LANE: the one place the private key exists.
#
# Usage:
#   scripts/release-app-sign.sh --tag v1.29.27 [--repo markfietje/brain-server] [--dry-run]
#   scripts/release-app-sign.sh --tag v1.29.27 --artifact "dist/Brain Shell.app.tar.gz" \
#                               --bundle "dist/Brain Shell.app" [--dry-run]
#
# ── WHAT THIS IS, AND WHAT IT IS NOT ───────────────────────────────────────
#
# The PUBLIC workflow proves the build. It never signs: the public Actions
# must never hold the key (D6), and the key is a secret of the same standing
# as the passphrase. So the signing happens HERE, on a machine that holds the
# key — the operator's own machine today, and a PRIVATE runner's secret if
# the operator ever wants an unattended lane. The two routes run the SAME
# commands; only where the passphrase comes from differs.
#
# The APPLE identity is NOT here and is not coming from here (D4): the
# Developer ID certificate is Apple's, is deferred to the first paying
# customer, and gates the notarization tail (III-5). Everything this script
# produces under an ad-hoc signature is STRUCTURE, not shippability (D5).
#
# ── THE CHAIN, AND THE ORDER THE STEPS RUN IN ─────────────────────────────
#
#   1. THE PAIRING. The app ships ONE public key (pinned in
#      `shell/src-tauri/tauri.conf.json`), and the key that signs must be
#      that key's other half. This is checked FIRST, before anything is
#      built, because a mismatch must cost nothing to discover. A minisign
#      SECRET key does not embed the public key, so the comparison is made
#      between the two PUBLIC halves: the operator's `.pub` and the pinned
#      one. (D2.)
#
#   2. THE ARTEFACT. The bundler makes its OWN archive — `Brain
#      Shell.app.tar.gz` — and the Tauri CLI signs it with the key. A
#      hand-made tarball stalls the install (measured in III-4), so the
#      packaging is never done here by hand.
#
#   3. THE ROUND TRIP. The signature this script produced is verified,
#      with `minisign`, against the PINNED public key — not against the
#      operator's copy of it. That is the proof the key that signs is the
#      key the app ships, and it is the assertion no drill in III-4 made
#      (every drill there minted a fresh keypair and threw it away).
#
#   4. THE MANIFEST. `shell/scripts/updater.mjs write` builds it and refuses
#      rather than guesses: a tag that does not resolve, a signature bound
#      to another artifact, an announced version the artifact does not carry
#      — each is a refusal naming what it found.
#
#   5. THE THREE-ASSET GATE. The artifact, the signature and the manifest
#      travel together or not at all (D3). Half a chain is worse than no
#      chain: a manifest that names an artifact the release does not carry
#      is a broken promise the updater discovers at the user's machine.
#
#   6. THE PUBLICATION. One `gh release upload` with all three files.
#
# ── THE PASSPHRASE ─────────────────────────────────────────────────────────
#
# It is read from a FILE (mode 0600, outside this repository), from an
# environment variable, or typed at a silent prompt. It is never a command
# line argument (the process list is readable by anyone on the machine) and
# it is never echoed, never logged and never written to a file this script
# creates. The Tauri CLI has no way to read it from a terminal, so it is
# handed over in the environment of the one command that needs it.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SHELL_DIR="$REPO_ROOT/shell"
CONFIG="$SHELL_DIR/src-tauri/tauri.conf.json"
RELEASE_CONFIG="$SHELL_DIR/src-tauri/tauri.release.conf.json"

# The operator's custody. Read-only: this script never writes to either of
# these paths, and it never reads the double-base64'd `updater.key` in any
# way that changes it.
DEFAULT_SECRET_KEY="$HOME/.config/brain-server/updater.key"
DEFAULT_PUBLIC_KEY="$HOME/.config/brain-server/updater.key.pub"

# The PUBLIC repository. The public Actions must never hold the key, and
# this is where that law is enforced rather than merely intended.
PUBLIC_REPO="markfietje/brain-server"

TAG=""
REPO_SLUG="markfietje/brain-server"
SECRET_KEY="$DEFAULT_SECRET_KEY"
PUBLIC_KEY="$DEFAULT_PUBLIC_KEY"
ARTIFACT=""
BUNDLE=""
OUT_DIR=""
PASSPHRASE_FILE=""
PASSPHRASE_ENV="BRAIN_UPDATER_PASSPHRASE"
# The KEYCHAIN ITEM that holds the passphrase. Its NAME is a constant, not a
# secret: it says where the passphrase lives. The same name is written by
# `scripts/updater-key-passphrase.sh`.
KEYCHAIN_SERVICE="app.brain.shell/updater-signing"
KEYCHAIN_ACCOUNT="default"
DRY_RUN=0
NOTES=""
CHECK_CHAIN=""

die() {
	echo "error: $*" >&2
	exit 1
}

usage() {
	cat >&2 <<'USAGE'
usage:
  scripts/release-app-sign.sh --tag vX.Y.Z [--repo markfietje/brain-server] [--dry-run]
  scripts/release-app-sign.sh --tag vX.Y.Z --artifact <file.tar.gz> --bundle <App.app> [--dry-run]

options:
  --tag <vX.Y.Z>          the RELEASE tag (not the app's version). Required.
  --artifact <file>       an already-built updater artifact to sign.
  --bundle <App.app>      the built bundle (its Info.plist carries the version).
  --out-dir <dir>         where latest.json is written (default: the artifact's directory).
  --check-chain <file>    the release's own fail-closed list, for the app chain:
                          refuse unless <file>, <file>.sig and <dir>/latest.json are
                          all present. Needs no key and publishes nothing.
  --secret-key <file>     the minisign secret key (default: ~/.config/brain-server/updater.key)
  --public-key <file>     the minisign PUBLIC half (default: ~/.config/brain-server/updater.key.pub)
  --passphrase-file <f>   the passphrase, in a 0600 file outside this repository.
  --passphrase-env <NAME> the environment variable holding the passphrase
                          (default: BRAIN_UPDATER_PASSPHRASE).
  --notes <text>          the manifest's release notes.
  --repo <slug>           the GitHub repository the release lives in.
  --dry-run               stage everything and publish NOTHING.
USAGE
	exit 2
}

while [[ $# -gt 0 ]]; do
	case "$1" in
		--tag) TAG="${2:-}"; shift 2;;
		--artifact) ARTIFACT="${2:-}"; shift 2;;
		--bundle) BUNDLE="${2:-}"; shift 2;;
		--out-dir) OUT_DIR="${2:-}"; shift 2;;
		--check-chain) CHECK_CHAIN="${2:-}"; shift 2;;
		--secret-key) SECRET_KEY="${2:-}"; shift 2;;
		--public-key) PUBLIC_KEY="${2:-}"; shift 2;;
		--passphrase-file) PASSPHRASE_FILE="${2:-}"; shift 2;;
		--passphrase-env) PASSPHRASE_ENV="${2:-}"; shift 2;;
		--notes) NOTES="${2:-}"; shift 2;;
		--repo) REPO_SLUG="${2:-}"; shift 2;;
		--dry-run) DRY_RUN=1; shift;;
		-h|--help) usage;;
		*) usage;;
	esac
done

[[ -n "$TAG" || -n "$CHECK_CHAIN" ]] || usage
[[ -z "$TAG" || "$TAG" == v* ]] || die "the release tag must look like vX.Y.Z: $TAG"

# ── THE TOOLS, SCOPED TO THE STEP THAT NEEDS THEM ──────────────────────────
#
# The chain gate is pure shell and needs NOTHING; the pairing check and the
# manifest writer need node; the ROUND TRIP's verification needs minisign.
# Asking for minisign up front made the script refuse in a CI image that has
# no minisign — and the CI image never reaches the verification step anyway,
# because the public Actions refuse to SIGN (below).

# ── D6 · THE PUBLIC ACTIONS NEVER SIGN ─────────────────────────────────────
#
# NOT a comment: a refusal — and it is a refusal of the SIGNING STEP, not of
# the whole script. **That distinction is the law, and the public CI is what
# taught it.** The first version of this guard sat at the top and refused the
# SCRIPT, which read as "the public Actions must never run this" — and the
# public Actions must run the parts that CANNOT sign: the chain gate is a
# VERIFICATION of what a private lane produced, and the pairing check reads
# two PUBLIC halves. Refusing the whole script made both of them unreachable
# from CI, and the shell lane went RED on the very pins that enforce the
# law.
#
# So the law is exactly what it says: **the public Actions must never SIGN.**
# The key is what makes a job dangerous, and the signing step is the only
# thing that touches it. Everything before it — the chain gate, the pairing
# check — is verification, and verification is what the public lane is FOR.
#
# THE REFUSAL LIVES WITH THE SIGNING STEP, where the key is used.
signing_refused_in_public_actions() {
	[[ "${GITHUB_ACTIONS:-}" == "true" && "${GITHUB_REPOSITORY:-}" == "markfietje/brain-server" ]]
}

TMPDIR_SIGN="$(mktemp -d)"
cleanup() { rm -rf "$TMPDIR_SIGN"; }
trap cleanup EXIT

# ── THE THREE-ASSET GATE (D3, D7) ──────────────────────────────────────────
#
# The artifact, the signature and the manifest travel together or not at all.
# Half a chain is worse than no chain: a manifest that names an artifact the
# release does not carry is a broken promise the updater discovers at the
# user's machine.
#
# ONE implementation, TWO callers — the publish path below and the
# `--check-chain` mode, so the gate that runs before an upload is the same
# gate that can be driven on its own. (A check that exists in only one of
# them fails only in the more expensive one.)
require_chain() {
	local artifact_path="$1"
	local signature_path="$2"
	local manifest_path="$3"
	local missing=()
	[[ -f "$artifact_path" ]] || missing+=("artifact: $artifact_path")
	[[ -f "$signature_path" ]] || missing+=("signature: $signature_path")
	[[ -f "$manifest_path" ]] || missing+=("manifest: $manifest_path")
	if [[ ${#missing[@]} -gt 0 ]]; then
		for entry in "${missing[@]}"; do echo "::error::required asset missing: $entry" >&2; done
		die "refusing to publish a partial chain (D3): the artifact, its signature and the manifest travel together"
	fi
}

# The standalone check: the release's own fail-closed list, for the app
# chain, driven on its own. It names the MISSING asset by name — the same
# shape as the release's per-binary loop. It needs no key and publishes
# nothing, so it can run anywhere — including the public CI, as a check on
# what a signing lane produced.
if [[ "$CHECK_CHAIN" != "" ]]; then
	require_chain "$CHECK_CHAIN" "$CHECK_CHAIN.sig" "$(dirname "$CHECK_CHAIN")/latest.json"
	echo "OK  the chain is complete: $CHECK_CHAIN, $CHECK_CHAIN.sig, $(dirname "$CHECK_CHAIN")/latest.json"
	exit 0
fi

# ── 1 · THE PAIRING (D2) ───────────────────────────────────────────────────
#
# The app ships ONE public key. The key that signs must be that key's other
# half. The comparison is between the two PUBLIC halves, because a minisign
# secret key does not embed the public key — an earlier reading of this tree
# compared the secret's encrypted payload against the pinned public payload
# and concluded, wrongly, that no private half existed.
#
# This check runs FIRST, before anything is built and before the tree is
# even looked at: a mismatch must cost nothing to discover, and the refusal
# must be about the KEY, not about whatever else the machine was doing.
#
# It needs node (to decode and compare the two public halves) and nothing
# else — no key, no passphrase, no minisign. **Verification is what the public
# lane is FOR.**
command -v node >/dev/null 2>&1 || die "node is not on PATH"
PINNED_PUBKEY="$(node -e '
const fs = require("node:fs");
const config = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const key = config?.plugins?.updater?.pubkey;
if (typeof key !== "string" || key.length === 0) {
	console.error("the app configuration carries no updater public key");
	process.exit(1);
}
process.stdout.write(key.trim());
' "$CONFIG")" || die "the app configuration carries no updater public key"

[[ -f "$PUBLIC_KEY" ]] || die "the public half of the signing key is absent: $PUBLIC_KEY"

# Both sides are base64 of the minisign public-key text. Decode each, then
# compare BYTES — not shapes, not keynum prefixes.
node -e '
const fs = require("node:fs");
const decode = (value) => Buffer.from(String(value).trim(), "base64").toString("utf8");

// The KEYNUM, read out of the untrusted comment. Two writers spell it
// differently — the Tauri CLI writes "minisign public key: <KEYNUM>" and
// minisign itself writes "minisign public key <KEYNUM>" — so the comment is
// read with a pattern that accepts both.
const keynum = (text) => {
	const match = /minisign public key:?\s+([0-9A-Fa-f]{16})/.exec(text.split("\n")[0] ?? "");
	return match?.[1]?.toUpperCase() ?? "<unknown keynum>";
};

// The KEY ID, read out of the key packet: the two bytes after the "Ed"
// algorithm marker are the eight bytes minisign calls the key id, and they
// are what actually distinguishes one key from another. (The first two bytes
// of every minisign packet are the same "Ed", so naming them would name
// nothing.)
const keyId = (text) => {
	const packet = Buffer.from((text.split("\n")[1] ?? "").trim(), "base64");
	return packet.subarray(2, 10).toString("hex");
};

const pinned = decode(process.argv[1]);
const held = decode(fs.readFileSync(process.argv[2], "utf8"));
if (pinned === held) {
	process.stdout.write(`ok\t${keynum(pinned)}\t${keyId(pinned)}\n`);
	process.exit(0);
}
console.error(
	`the signing key is NOT the key the app ships (D2).\n` +
	`  the app pins:  keynum ${keynum(pinned)}  key id ${keyId(pinned)}\n` +
	`  the key here:  keynum ${keynum(held)}  key id ${keyId(held)}\n` +
	`  (${process.argv[2]})`
);
process.exit(1);
' "$PINNED_PUBKEY" "$PUBLIC_KEY" > "$TMPDIR_SIGN/pairing" || {
	cat "$TMPDIR_SIGN/pairing" >&2
	die "refusing to sign: the key that signs is not the key the app ships"
}
PINNED_KEYNUM="$(cut -f2 "$TMPDIR_SIGN/pairing")"
echo ">> the signing key IS the key the app ships: keynum $PINNED_KEYNUM"

# The pinned public half, decoded, is what the round trip verifies against —
# not the operator's copy of it. So a swapped `.pub` cannot make a wrong
# signature verify.
node -e '
const fs = require("node:fs");
fs.writeFileSync(process.argv[2], Buffer.from(process.argv[1].trim(), "base64").toString("utf8"));
' "$PINNED_PUBKEY" "$TMPDIR_SIGN/pinned.pub"

# ── D6 · THE PUBLIC ACTIONS NEVER SIGN ─────────────────────────────────────
#
# NOT a comment: a refusal — and it is a refusal of the SIGNING PATH, not of
# the whole script. **That distinction is the law, and the public CI is what
# taught it.** The first version of this guard sat at the top and refused the
# SCRIPT, which reads as "the public Actions must never run this" — and the
# public Actions must run the parts that CANNOT sign: the chain gate is a
# VERIFICATION of what a private lane produced, and the pairing check reads
# two PUBLIC halves. Refusing the whole script made both of them unreachable
# from CI, and the shell lane went RED on the very pins that enforce the
# law.
#
# So the law is exactly what it says: **the public Actions must never SIGN.**
# The key is what makes a job dangerous, and the signing step is the only
# thing that touches it. Everything before it — the chain gate, the pairing
# check — is verification, and verification is what the public lane is FOR.
#
# It sits HERE, after the pairing check and before anything that could
# refuse for an unrelated reason, so the refusal a caller sees in the public
# Actions is about the SIGNING and not about the tree it happened to be in.
signing_refused_in_public_actions() {
	[[ "${GITHUB_ACTIONS:-}" == "true" && "${GITHUB_REPOSITORY:-}" == "markfietje/brain-server" ]]
}

if signing_refused_in_public_actions; then
	die "the public Actions must never SIGN the release (D6). Run this on a machine that holds the key, or on a private runner."
fi

# The tree the release names must be the tree that was built: the manifest
# writer refuses a dirty tree, and this refuses it earlier with the same
# reason. A release built from an uncommitted tree is not reproducible.
if [[ -n "$(git -C "$REPO_ROOT" status --porcelain)" ]]; then
	die "the working tree is dirty: a manifest names a commit, and a commit is not a dirty tree"
fi

# The release tag must EXIST in this repository: the manifest's URL names it,
# and a URL naming a tag that does not resolve is a promise the release
# cannot keep (D1).
if ! git -C "$REPO_ROOT" rev-parse --verify --quiet "refs/tags/$TAG" >/dev/null; then
	die "the release tag does not resolve in this repository: $TAG"
fi

# ── THE PASSPHRASE ─────────────────────────────────────────────────────────
#
# The D6 REFUSAL is above, right after the pairing check, and it refuses the
# SIGNING PATH — the only part of this script that touches the key. It sits
# there, before the passphrase is asked for, because a CI job that asked for
# one and then refused would be a job that HANGS on a prompt nobody can
# answer.
#
# WHERE THE PASSPHRASE COMES FROM, in order:
#
#   1. `--passphrase-file` — a 0600 file OUTSIDE this repository.
#   2. `--passphrase-env` — an environment variable.
#   3. **THE LOGIN KEYCHAIN** — the default, and the custody this round
#      settled on. The item is `app.brain.shell/updater-signing` / `default`
#      and it holds the passphrase for the updater's signing key. **Only the
#      ITEM'S NAME is in this script — never a passphrase, never a key.** The
#      read is `security find-generic-password -w`, which returns the value
#      on stdout and nothing else; it is never echoed, never logged, and
#      never written to a file this script creates.
#   4. A silent prompt, for a machine with no keychain item.
#
# A passphrase in the REPOSITORY is the same secret as a key in the
# repository, so the file form refuses a path inside the tree and checks the
# mode.
read_passphrase() {
	if [[ -n "$PASSPHRASE_FILE" ]]; then
		[[ -f "$PASSPHRASE_FILE" ]] || die "the passphrase file is absent: $PASSPHRASE_FILE"
		# The passphrase must not live in the repository: the tree's own law is
		# that the private key is in no tracked file, and a passphrase in the
		# tree is the same secret with a shorter fuse.
		case "$PASSPHRASE_FILE" in
			"$REPO_ROOT"/*) die "the passphrase file is inside the repository: $PASSPHRASE_FILE";;
		esac
		local mode
		mode="$(stat -f '%Lp' "$PASSPHRASE_FILE" 2>/dev/null || stat -c '%a' "$PASSPHRASE_FILE" 2>/dev/null || echo '')"
		if [[ -n "$mode" && "$mode" != "600" ]]; then
			die "the passphrase file must be owner-only (0600): $PASSPHRASE_FILE is $mode"
		fi
		IFS= read -r PASSPHRASE < "$PASSPHRASE_FILE" || true
		[[ -n "$PASSPHRASE" ]] || die "the passphrase file is empty: $PASSPHRASE_FILE"
		echo ">> using the passphrase from the file $PASSPHRASE_FILE"
		return
	fi
	if [[ -n "${!PASSPHRASE_ENV:-}" ]]; then
		PASSPHRASE="${!PASSPHRASE_ENV}"
		[[ -n "$PASSPHRASE" ]] || die "the passphrase environment variable is empty: $PASSPHRASE_ENV"
		echo ">> using the passphrase from the environment variable $PASSPHRASE_ENV"
		return
	fi
	# THE KEYCHAIN. Its name is a CONSTANT, not a secret: it says WHERE the
	# passphrase lives, and it is the same name `scripts/updater-key-
	# passphrase.sh` writes to.
	if command -v security >/dev/null 2>&1; then
		PASSPHRASE="$(security find-generic-password -s "$KEYCHAIN_SERVICE" -a "$KEYCHAIN_ACCOUNT" -w 2>/dev/null || true)"
		if [[ -n "$PASSPHRASE" ]]; then
			echo ">> using the passphrase from the login keychain ($KEYCHAIN_SERVICE / $KEYCHAIN_ACCOUNT)"
			return
		fi
	fi
	if [[ ! -t 0 ]]; then
		die "no passphrase: the login keychain has no '$KEYCHAIN_SERVICE' item for '$KEYCHAIN_ACCOUNT', and this is not a terminal. Pass --passphrase-file, set $PASSPHRASE_ENV, or run this on a terminal."
	fi
	echo -n ">> passphrase for the updater signing key: " >&2
	IFS= read -rs PASSPHRASE || true
	echo "" >&2
	[[ -n "$PASSPHRASE" ]] || die "the passphrase is empty"
	echo ">> using the passphrase from the terminal"
}
read_passphrase

# ── THE SECRET KEY, IN THE FORM THE CLI READS ─────────────────────────────
#
# The Tauri CLI reads a private key FILE as base64 of the minisign key text.
# The operator's stored form is exactly that; the readable copy beside it is
# the raw text. Both are accepted, and neither is modified.
[[ -f "$SECRET_KEY" ]] || die "the signing key is absent: $SECRET_KEY"
SIGN_KEY="$SECRET_KEY"
KEY_FIRST_LINE="$(head -n 1 "$SECRET_KEY" 2>/dev/null || true)"
if [[ "$KEY_FIRST_LINE" == "untrusted comment:"* ]]; then
	# The raw form: re-encode it for the CLI, in a temp file that is removed.
	# **The trailing newline is REMOVED on purpose (measured 2026-10-22):**
	# BSD `base64` ends its output with a newline, and the Tauri CLI's
	# decoder rejects it — `failed to decode base64 secret key: Invalid
	# symbol 10, offset 348`. The operator's stored form has no trailing
	# newline, which is why it reads.
	base64 -i "$SECRET_KEY" | tr -d '\n' > "$TMPDIR_SIGN/secret.b64"
	SIGN_KEY="$TMPDIR_SIGN/secret.b64"
fi

APP_VERSION="$(node -e '
const fs = require("node:fs");
const config = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
process.stdout.write(String(config?.version ?? ""));
' "$CONFIG")"
[[ -n "$APP_VERSION" ]] || die "the app configuration carries no version"

# ── 2 · THE ARTEFACT ───────────────────────────────────────────────────────
#
# Built by the BUNDLER (never by hand: a hand-made archive stalls the
# install), and signed by the Tauri CLI. `--bundles app` plus the release
# configuration turns the updater artifacts on, which is what makes the CLI
# sign the artifact it just built.
if [[ -z "$ARTIFACT" ]]; then
	command -v pnpm >/dev/null 2>&1 || die "pnpm is unavailable — the release artifact must be built by the bundler"
	echo ">> building the release artifact (bundler archive + Tauri CLI signature)…"
	(
		cd "$SHELL_DIR"
		TAURI_SIGNING_PRIVATE_KEY="$SIGN_KEY" \
		TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$PASSPHRASE" \
			pnpm tauri build --bundles app --target universal-apple-darwin \
				--config "$RELEASE_CONFIG"
	)
	BUNDLE_DIR="$SHELL_DIR/src-tauri/target/universal-apple-darwin/release/bundle/macos"
	ARTIFACT="$(ls "$BUNDLE_DIR"/*.app.tar.gz 2>/dev/null | head -1 || true)"
	[[ -n "$ARTIFACT" ]] || die "the bundler produced no updater artifact under $BUNDLE_DIR"
	BUNDLE="${BUNDLE:-$BUNDLE_DIR/$(basename "$ARTIFACT" .tar.gz)}"
else
	[[ -f "$ARTIFACT" ]] || die "the artifact is absent: $ARTIFACT"
	[[ -n "$BUNDLE" ]] || die "--artifact needs --bundle (the built .app, for its own version)"
	[[ -d "$BUNDLE" ]] || die "the bundle is absent: $BUNDLE"
fi

# ── THE PUBLICATION NAME ──────────────────────────────────────────────────
#
# The artefact's NAME is part of the URL the updater fetches, and a release
# asset does not keep every character a file may carry. **MEASURED
# (2026-10-22):** GitHub replaces a SPACE in an asset's name with a DOT, so
# `Brain Shell.app.tar.gz` is published as `Brain.Shell.app.tar.gz` — and the
# manifest's URL, which names the space (`Brain%20Shell.app.tar.gz`), then
# 404s. The updater would download NOTHING.
#
# So the artefact is RENAMED to a name a release asset keeps, and — the part
# that matters — it is renamed BEFORE it is signed, so the signature's
# `file:` binding names the artefact as it will be PUBLISHED. The writer
# refuses a signature that names another file, and that refusal is right.
if [[ "$(basename "$ARTIFACT")" == *" "* ]]; then
	SAFE_NAME="$(basename "$ARTIFACT" | tr ' ' '-')"
	PUBLISHED_ARTIFACT="$(dirname "$ARTIFACT")/$SAFE_NAME"
	echo ">> renaming the artifact for publication: $(basename "$ARTIFACT") -> $SAFE_NAME"
	rm -f "$PUBLISHED_ARTIFACT" "$PUBLISHED_ARTIFACT.sig"
	mv "$ARTIFACT" "$PUBLISHED_ARTIFACT"
	ARTIFACT="$PUBLISHED_ARTIFACT"
fi

SIGNATURE="$ARTIFACT.sig"

# The artifact must be signed. If the bundler already signed it, that
# signature is verified; if not, it is produced here — by the same CLI, with
# the same key.
if [[ ! -f "$SIGNATURE" ]]; then
	echo ">> signing the artifact with the updater key…"
	if ! (
		cd "$SHELL_DIR"
		TAURI_SIGNING_PRIVATE_KEY_PATH="$SIGN_KEY" \
		TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$PASSPHRASE" \
			pnpm tauri signer sign --app-version "$APP_VERSION" "$ARTIFACT"
	) > "$TMPDIR_SIGN/sign.log" 2>&1; then
		cat "$TMPDIR_SIGN/sign.log" >&2
		die "the signing key did not sign the artifact: the passphrase is wrong, or the file is not a usable minisign secret key"
	fi
fi
[[ -f "$SIGNATURE" ]] || die "the artifact carries no signature beside it: $SIGNATURE"

# ── 3 · THE ROUND TRIP (D2) ────────────────────────────────────────────────
#
# Verify what was just signed against the PINNED public key. `minisign` reads
# the signature file as the four-line text, and the `.sig` file the Tauri
# CLI writes is the BASE64 of that text — so both are decoded first.
#
# `minisign` is required HERE and not at the top of the script: it is the
# VERIFIER, the public Actions never reach it (they refuse to sign), and a
# CI image with no minisign must still be able to run the chain gate and the
# pairing check.
command -v minisign >/dev/null 2>&1 || die "minisign is not installed (brew install minisign) — the round trip cannot be verified without it"
node -e '
const fs = require("node:fs");
fs.writeFileSync(process.argv[2], Buffer.from(fs.readFileSync(process.argv[1], "utf8").trim(), "base64").toString("utf8"));
' "$SIGNATURE" "$TMPDIR_SIGN/signature.txt"

if ! minisign -V -p "$TMPDIR_SIGN/pinned.pub" -m "$ARTIFACT" -x "$TMPDIR_SIGN/signature.txt" > "$TMPDIR_SIGN/verify" 2>&1; then
	cat "$TMPDIR_SIGN/verify" >&2
	die "refusing to publish: the signature does not verify against the key the app ships"
fi
echo ">> the round trip holds: the artifact's signature verifies against the pinned key."

# ── 4 · THE MANIFEST ───────────────────────────────────────────────────────
OUT_DIR="${OUT_DIR:-$(dirname "$ARTIFACT")}"
mkdir -p "$OUT_DIR"
MANIFEST="$OUT_DIR/latest.json"
STAGED="$SHELL_DIR/src-tauri/binaries"

echo ">> writing the update manifest for $TAG…"
node "$SHELL_DIR/scripts/updater.mjs" write \
	--root "$REPO_ROOT" \
	--staged "$STAGED" \
	--artifact "$ARTIFACT" \
	--bundle "$BUNDLE" \
	--release-tag "$TAG" \
	--notes "$NOTES" \
	--out "$MANIFEST" > "$TMPDIR_SIGN/manifest.log" 2>&1 || {
	cat "$TMPDIR_SIGN/manifest.log" >&2
	die "refusing to publish: the manifest writer refused"
}
cat "$TMPDIR_SIGN/manifest.log"

# ── 5 · THE THREE-ASSET GATE (D3, D7) ─────────────────────────────────────
#
# The gate itself is defined above, next to the other rules the script
# refuses by: ONE implementation, and this is its second caller.
require_chain "$ARTIFACT" "$SIGNATURE" "$MANIFEST"
# The manifest must name the artifact this run signed, and the tag this run
# was given — one chain, three pieces, checked together.
node -e '
const fs = require("node:fs");
const manifest = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const artifact = process.argv[2];
const tag = process.argv[3];
const basename = artifact.split("/").pop();
const problems = [];
if (manifest.release_tag !== tag) {
	problems.push(`the manifest names release tag ${manifest.release_tag}, not ${tag}`);
}
for (const [target, entry] of Object.entries(manifest.platforms ?? {})) {
	// The signature is PER PLATFORM: that is the manifest shape the app
	// updater reads and verifies (the Tauri format), the shape the app own
	// fixture uses, and the shape the writer emits. There is no top-level
	// signature field in that format -- so checking for one refused a
	// correct manifest and made the signed path unreachable.
	if (!entry.signature) problems.push(`${target}: the manifest carries no signature`);
	const url = entry.url ?? "";
	if (!url.includes(tag)) problems.push(`${target}: the URL does not name ${tag}`);
	const named = decodeURIComponent(url.split("/").pop() ?? "");
	if (named !== basename) problems.push(`${target}: the URL names ${named}, not ${basename}`);
}
if (problems.length > 0) {
	for (const problem of problems) console.error(`::error::${problem}`);
	process.exit(1);
}
' "$MANIFEST" "$ARTIFACT" "$TAG" || die "refusing to publish: the manifest does not describe this artifact and tag"

# ── 6 · THE PUBLICATION ────────────────────────────────────────────────────
if [[ "$DRY_RUN" == "1" ]]; then
	echo ">> dry run: nothing published. The chain is:"
	echo "     artifact:  $ARTIFACT"
	echo "     signature: $SIGNATURE"
	echo "     manifest:  $MANIFEST"
	exit 0
fi

command -v gh >/dev/null 2>&1 || die "gh is not on PATH — the chain cannot be published from here"
echo ">> publishing the chain to $REPO_SLUG $TAG…"
gh release upload "$TAG" "$ARTIFACT" "$SIGNATURE" "$MANIFEST" --repo "$REPO_SLUG" --clobber
echo ">> published. The updater's endpoint now resolves to this release's latest.json."
