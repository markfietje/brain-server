#!/usr/bin/env bash
# secrets-truth.sh — the gateway-token truth lane.
#
# The openclaw gateway must never carry the brain-server OPERATOR token:
# the twokeys boundary (operator/agent split) only binds traffic to the
# revocable agent principal if the process actually authenticates with the
# agent token — an operator-token gateway silently re-merges the split and
# the /ops/agents/revoke kill-switch reaches nothing.
#
# DIGEST-ONLY OUTPUT (the standing law): this script never prints, logs,
# or writes token material — every comparison and every line of evidence
# is a sha256 digest truncated to 12 hex. A wrong posture is named loudly;
# the secret that proves it is not.
#
# The measured surface is the EFFECTIVE gateway environment, not the
# machine-generated env file: the plist execs the fork-owned wrapper,
# which sources that file and may then override it — so the truth is what
# the wrapper produces. The probe rides the wrapper itself (`exec "$@"`
# with a digest-printing command), which is the same environment the node
# gateway process would inherit.
#
# Usage:
#   scripts/secrets-truth.sh --selfcheck
#       Check this machine's live setup against the canonical paths.
#       SKIPS (exit 0, loudly) when no openclaw gateway env exists here —
#       this is a local lane, and absence of the setup is not a violation.
#   scripts/secrets-truth.sh --probe ENV_FILE WRAPPER AUTH_TOKEN_FILE [OPENCLAW_JSON]
#       The same assertions over explicit paths — the red-proof harness:
#       the hostile fixture (an env copy carrying the operator token) must
#       FAIL here, proving the selfcheck cannot pass vacuously.
set -uo pipefail

# ── digest helpers (12-hex, never the value) ────────────────────────────────
hasher() {
  if command -v shasum >/dev/null 2>&1; then
    printf '%s' "$1" | shasum -a 256 | cut -c1-12
  elif command -v sha256sum >/dev/null 2>&1; then
    printf '%s' "$1" | sha256sum | cut -c1-12
  elif command -v openssl >/dev/null 2>&1; then
    printf '%s' "$1" | openssl dgst -sha256 | awk '{print substr($2,1,12)}'
  else
    echo "NO_HASHER"
  fi
}

# ── the probe command run UNDER the wrapper (digests only) ─────────────────
# Runs with the effective gateway env; emits three labelled digests. The
# hasher branch is self-contained in the body — a pipe cannot ride a
# variable expansion (word-split, never re-parsed as an operator).
PROBE_BODY='
d() {
  if command -v shasum >/dev/null 2>&1; then
    printf "%s" "$1" | shasum -a 256 | cut -c1-12
  else
    printf "%s" "$1" | sha256sum | cut -c1-12
  fi
}
if [ -n "${BRAIN_TOKEN_FILE:-}" ] && [ -f "${BRAIN_TOKEN_FILE:-}" ]; then
  printf "token_file %s\n" "$(d "$(cat "$BRAIN_TOKEN_FILE")")"
else
  printf "token_file absent\n"
fi
if [ -n "${BRAIN_TOKEN:-}" ]; then
  printf "token_var %s\n" "$(d "$BRAIN_TOKEN")"
else
  printf "token_var absent\n"
fi
if [ -n "${BRAIN_SERVER_AUTH_TOKEN:-}" ]; then
  printf "server_auth %s\n" "$(d "$BRAIN_SERVER_AUTH_TOKEN")"
else
  printf "server_auth absent\n"
fi
'

# ── the assertions ──────────────────────────────────────────────────────────
# check ENV_FILE WRAPPER AUTH_TOKEN_FILE [OPENCLAW_JSON]
# Prints digest-only evidence; returns non-zero on any violation.
check() {
  local env_file="$1" wrapper="$2" auth_file="$3" json_file="${4:-}"
  local fail=0

  if [ ! -f "$auth_file" ]; then
    echo "FAIL  auth token file absent: $auth_file (cannot derive the comparison digests)"
    return 1
  fi
  local line1 line2
  line1="$(sed -n '1p' "$auth_file" | tr -d '\r\n')"
  line2="$(sed -n '2p' "$auth_file" | tr -d '\r\n')"
  if [ -z "$line1" ] || [ -z "$line2" ]; then
    echo "FAIL  $auth_file does not carry the two-token form (operator line 1, agent line 2) — refusing rather than guessing which line is which"
    return 1
  fi
  local op_digest agent_digest
  op_digest="$(hasher "$line1")"
  agent_digest="$(hasher "$line2")"
  if [ "$op_digest" = "NO_HASHER" ] || [ "$agent_digest" = "NO_HASHER" ]; then
    echo "FAIL  no sha256 tool available (shasum/sha256sum/openssl) — the lane cannot measure, so it refuses rather than comparing placeholder digests"
    return 1
  fi
  if [ "$line1" = "$line2" ]; then
    echo "FAIL  $auth_file carries the same value on both lines — the operator/agent split does not exist, refusing rather than comparing identical digests"
    return 1
  fi
  echo "      auth line 1 (operator) digest $op_digest"
  echo "      auth line 2 (agent)    digest $agent_digest"

  # 1. openclaw.json must not carry an authToken for the brain-server
  #    plugin: ${ENV} placeholders resolve at config load, and that rung
  #    is the one a regenerated config reaches for first.
  if [ -n "$json_file" ] && [ -f "$json_file" ] && command -v python3 >/dev/null 2>&1; then
    if ! python3 - "$json_file" <<'PYEOF'
import json, sys
try:
    cfg = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(0)  # unparseable config is not this lane's finding
entry = (cfg.get("plugins", {}).get("entries", {}) or {}).get("brain-server", {})
if isinstance(entry, dict) and "authToken" in (entry.get("config") or {}):
    sys.exit(1)
sys.exit(0)
PYEOF
    then
      echo "FAIL  openclaw.json brain-server plugin config still carries authToken — the env-placeholder rung must not exist (delete it; the wrapper's BRAIN_TOKEN_FILE is the preferred rung)"
      fail=1
    else
      echo "PASS  openclaw.json brain-server plugin carries no authToken"
    fi
  fi

  # 2. The EFFECTIVE gateway env (through the wrapper) must authenticate
  #    with the agent token and must not carry the operator token.
  if [ ! -f "$env_file" ]; then
    echo "FAIL  gateway env file absent: $env_file"
    return 1
  fi
  if [ ! -f "$wrapper" ]; then
    echo "FAIL  gateway env wrapper absent: $wrapper"
    return 1
  fi
  local probe
  probe="$(/bin/sh "$wrapper" "$env_file" /bin/sh -c "$PROBE_BODY" 2>/dev/null)" || {
    echo "FAIL  the wrapper refused to run against $env_file (invalid env file?)"
    return 1
  }
  local tf tv sa
  tf="$(printf '%s\n' "$probe" | awk '/^token_file / {print $2}')"
  tv="$(printf '%s\n' "$probe" | awk '/^token_var / {print $2}')"
  sa="$(printf '%s\n' "$probe" | awk '/^server_auth / {print $2}')"
  echo "      effective gateway env: token_file=${tf:-?} token_var=${tv:-?} server_auth=${sa:-?}"

  if [ "$tf" != "$agent_digest" ]; then
    echo "FAIL  the gateway's BRAIN_TOKEN_FILE does not resolve to the AGENT token (line 2) — expected digest $agent_digest, got ${tf:-none}"
    fail=1
  else
    echo "PASS  the gateway authenticates with the agent token (BRAIN_TOKEN_FILE == auth line 2)"
  fi
  for label_digest in "token_var:$tv" "server_auth:$sa"; do
    local label="${label_digest%%:*}" value="${label_digest#*:}"
    if [ "$value" = "$op_digest" ]; then
      echo "FAIL  the effective gateway env carries the OPERATOR token in $label (digest match with auth line 1)"
      fail=1
    fi
  done
  if [ "$tf" != "$op_digest" ] && [ "$tv" != "$op_digest" ] && [ "$sa" != "$op_digest" ]; then
    echo "PASS  no rung of the effective gateway env carries the operator token"
  fi
  return "$fail"
}

case "${1:-}" in
  --selfcheck)
    AUTH="$HOME/.config/brain-server/auth-token"
    ENV="$HOME/.openclaw/service-env/ai.openclaw.gateway.env"
    WRAPPER="$HOME/.openclaw/service-env/ai.openclaw.gateway-env-wrapper.sh"
    JSON="$HOME/.openclaw/openclaw.json"
    if [ ! -f "$ENV" ] && [ ! -f "$WRAPPER" ]; then
      echo "SKIP  no openclaw gateway env on this machine ($ENV) — nothing to check"
      exit 0
    fi
    echo "== secrets-truth: the gateway never carries the operator token =="
    if check "$ENV" "$WRAPPER" "$AUTH" "$JSON"; then
      echo "== secrets-truth: PASS =="
      exit 0
    else
      echo "== secrets-truth: FAIL =="
      exit 1
    fi
    ;;
  --probe)
    shift
    if [ "$#" -lt 3 ]; then
      echo "usage: $0 --probe ENV_FILE WRAPPER AUTH_TOKEN_FILE [OPENCLAW_JSON]" >&2
      exit 2
    fi
    check "$@"
    ;;
  *)
    echo "usage: $0 [--selfcheck | --probe ENV_FILE WRAPPER AUTH_TOKEN_FILE [OPENCLAW_JSON]]" >&2
    echo "  --selfcheck  check this machine's live gateway setup (digest-only output)" >&2
    echo "  --probe      the red-proof harness over explicit paths" >&2
    exit 2
    ;;
esac
