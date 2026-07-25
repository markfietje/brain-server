#!/usr/bin/env bash
# Build the release binary, install it to ~/.local/bin, and restart the
# launchd service so the running com.brain.server picks up the new binary.
#
# Usage: scripts/install-service.sh
#
# Idempotent: safe to run repeatedly. Requires the launchd plist at
# ~/Library/LaunchAgents/com.brain.server.plist (install it once first).
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_NAME="brain-server"
SRC_BIN="$REPO/target/release/$BIN_NAME"
DEST_DIR="$HOME/.local/bin"
DEST_BIN="$DEST_DIR/$BIN_NAME"
# Operator CLIs shipped alongside the server: brain (doctor/bench/ingest),
# mcp (MCP bridge), bench (latency harness), brain-migrate-rehearse (v0.9.9
# Qualify — copy-and-verify migration rehearsal). All are tiny static-ish
# clients.
# v0.9.6: brain-connector-stub is always built; brain-connector-gh needs the
# connector-github feature (reqwest + jsonwebtoken deps).
CLI_BINS=("brain" "mcp" "bench" "brain-connector-stub")
# v0.9.9: brain-migrate-rehearse needs the `migrate` feature to compile. Built
# in the same `cargo build` invocation as the others via --features bench,migrate.
FEATURE_BINS=("brain-migrate-rehearse:migrate")
# Optional binaries that require extra features to build. Built best-effort:
# if the feature flag is off, the binary is just absent from target/release.
OPTIONAL_BINS=("brain-connector-gh:connector-github")
LABEL="com.brain.server"
PLIST="$HOME/Library/LaunchAgents/${LABEL}.plist"
TOKEN_FILE="$HOME/.config/brain-server/auth-token"
UID_NUM="$(id -u)"

log() { printf '>> %s\n' "$*"; }
ok()  { printf 'OK  %s\n' "$*"; }
die() { printf 'ERR %s\n' "$*" >&2; exit 1; }

[[ -f "$PLIST" ]] || die "plist not found: $PLIST -- install the launchd service first."

# 1. Build the release binary + operator CLIs.
#    `--features bench,migrate` are empty features (no extra deps) — they only
#    gate compilation of the bench + brain-migrate-rehearse binaries.
log "building release binaries (server + brain/mcp/bench/brain-connector-stub/brain-migrate-rehearse)..."
( cd "$REPO" && cargo build --release --features bench,migrate \
    --bin "$BIN_NAME" --bin brain --bin mcp --bin bench --bin brain-connector-stub --bin brain-migrate-rehearse )
[[ -x "$SRC_BIN" ]] || die "build did not produce $SRC_BIN"

# 1b. Build optional binaries that need extra features. Best-effort: if the
#     operator didn't enable the feature, the binary is just absent and the
#     install step below skips it. The first build with a new feature pays the
#     dep-compile cost (~30-60s); subsequent builds are incremental.
for entry in "${OPTIONAL_BINS[@]}"; do
    bin="${entry%%:*}"
    feature="${entry##*:}"
    if [[ -x "$REPO/target/release/$bin" ]]; then
        log "$bin already built (feature $feature)"
    else
        log "building $bin (feature $feature)..."
        if ( cd "$REPO" && cargo build --release --features "$feature" --bin "$bin" ); then
            ok "$bin built"
        else
            log "$bin build failed (feature $feature not enabled?); skipping"
        fi
    fi
done

# 2. Install server + operator CLIs to ~/.local/bin
#    (decoupled from the checkout: survives cargo clean).
#    On macOS (Sonoma+), newly-written executables get a `com.apple.provenance`
#    xattr that Gatekeeper uses to SIGKILL the process on first exec. Strip it
#    so the binary is actually runnable.
mkdir -p "$DEST_DIR"
install_bin() {
	src="$1"; dest="$2"
	cp -f "$src" "$dest"
	chmod +x "$dest"
	xattr -d com.apple.provenance "$dest" 2>/dev/null || true
	ok "installed $dest"
}
install_bin "$SRC_BIN" "$DEST_BIN"
for bin in "${CLI_BINS[@]}"; do
	src="$REPO/target/release/$bin"
	[[ -x "$src" ]] || die "build did not produce $src"
	install_bin "$src" "$DEST_DIR/$bin"
done
# v0.9.9: feature-gated CLI bins (built in the main cargo line, so they exist).
for entry in "${FEATURE_BINS[@]}"; do
	bin="${entry%%:*}"
	src="$REPO/target/release/$bin"
	if [[ -x "$src" ]]; then
		install_bin "$src" "$DEST_DIR/$bin"
	fi
done
# Optional bins: install only if they were built (feature on).
for entry in "${OPTIONAL_BINS[@]}"; do
	bin="${entry%%:*}"
	src="$REPO/target/release/$bin"
	if [[ -x "$src" ]]; then
		install_bin "$src" "$DEST_DIR/$bin"
	fi
done

# 2b. Ensure the auth token is read from a 0600 secret file, not the plist env.
#     Never changes an existing token value — only relocates it from the plist's
#     plaintext AUTH_TOKEN into the file (or leaves an existing file untouched).
#     Idempotent: safe to run repeatedly.
AUTH_VAL="$(plutil -extract EnvironmentVariables.AUTH_TOKEN raw "$PLIST" 2>/dev/null || true)"
if [[ -n "$AUTH_VAL" ]]; then
	# Plaintext token in plist: relocate it verbatim to a 0600 file.
	mkdir -p "$(dirname "$TOKEN_FILE")"
	printf '%s' "$AUTH_VAL" > "$TOKEN_FILE"
	chmod 600 "$TOKEN_FILE"
	plutil -remove EnvironmentVariables.AUTH_TOKEN "$PLIST"
	# Clear any stale AUTH_TOKEN_FILE first so the insert is idempotent.
	plutil -remove EnvironmentVariables.AUTH_TOKEN_FILE "$PLIST" 2>/dev/null || true
	plutil -insert EnvironmentVariables.AUTH_TOKEN_FILE -string "$TOKEN_FILE" "$PLIST"
	ok "relocated AUTH_TOKEN verbatim -> $TOKEN_FILE (0600); plaintext removed from plist"
elif plutil -extract EnvironmentVariables.AUTH_TOKEN_FILE raw "$PLIST" >/dev/null 2>&1; then
	# Already file-backed. Preserve contents; only enforce perms.
	[[ -f "$TOKEN_FILE" ]] || die "plist sets AUTH_TOKEN_FILE but $TOKEN_FILE is missing — refusing to invent a token"
	chmod 600 "$TOKEN_FILE"
	ok "auth token sourced from $TOKEN_FILE (0600) — contents untouched"
fi

# 3. Restart the launchd service so it runs the new binary.
#    bootout is best-effort (service may not be loaded yet); bootstrap reloads
#    the plist, which always picks up the current binary at the copied path.
log "restarting launchd service ${LABEL}..."
launchctl bootout "gui/${UID_NUM}/${LABEL}" >/dev/null 2>&1 || true
sleep 0.5
launchctl bootstrap "gui/${UID_NUM}" "$PLIST" >/dev/null 2>&1 || \
  die "bootstrap failed -- run: launchctl bootstrap gui/${UID_NUM} $PLIST"

# 4. Verify the new process came up and is serving.
PID="$(pgrep -f "$DEST_BIN" | head -1 || true)"
[[ -n "$PID" ]] || die "service did not start -- check ~/Library/Logs/brain-server.err.log"
ok "running pid $PID from $DEST_BIN"

log "waiting for health endpoint..."
healthy=0
for _ in $(seq 1 15); do
  if curl -sf -m 2 -o /dev/null http://127.0.0.1:8765/health; then healthy=1; break; fi
  sleep 1
done
[[ "$healthy" = "1" ]] || die "service up but /health not responding -- check logs"
ok "/health OK"
printf '\nDone. Logs: ~/Library/Logs/brain-server.{log,err.log}\n'
