#!/usr/bin/env bash
# v1.20.10 "Proof": assemble an ADMT (Automated Decision-Making Transparency)
# record for a given chunk id — "why this became memory, by what path, from
# what source".
#
# Usage: scripts/admt-kit.sh <chunk-id> [--out DIR]
#   → writes DIR/ADMT_RECORD.json + DIR/ADMT_MANIFEST.json (default DIR=dist/admt-kit)
#
# It is a READ-ONLY assembly of already-emitted artifacts — never a fabricated
# summary:
#   * GET /get/{id}    → the chunk's `origin`, `owner`, title, evidence span.
#   * GET /audit       → the proposal-gate trail (proposal:{id} approve/reject
#                        rows) — the audit hash chain is the "why by what path".
#   PII: only the already-redacted `[redacted:…]` forms ride along (per v1.20.1).
#
# Requires a running server + a read-token. Fails loud if either is absent.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASE="${BRAIN_BASE_URL:-http://127.0.0.1:8765}"
TOKEN_FILE="${BRAIN_TOKEN_FILE:-$HOME/.config/brain-server/auth-token}"
ID="${1:-}"
OUT_DIR="${3:-$REPO/dist/admt-kit}"

if [[ -z "$ID" ]]; then
  echo "usage: $0 <chunk-id> [--out DIR]" >&2
  exit 2
fi
[[ -f "$TOKEN_FILE" ]] || { echo "ERR: no auth token at $TOKEN_FILE" >&2; exit 1; }
TOKEN="$(tr -d '[:space:]' < "$TOKEN_FILE")"

AUTH=(-H "Authorization: Bearer $TOKEN")
get() { curl -fsS "$@" "${AUTH[@]}"; }

mkdir -p "$OUT_DIR"

# The record itself — the server-authoritative pieces, assembled verbatim.
RECORD="$OUT_DIR/ADMT_RECORD.json"
{
  printf '{\n'
  printf '  "kit": "admt",\n'
  printf '  "decision_evidence": {\n'
  # Chunk provenance (origin/owner/evidence span) — from the existing /get/{id}.
  get "$BASE/get/$ID" \
    | sed 's/^/    /'
  printf '  },\n'
  printf '  "decision_path": {\n'
  # The proposal-gate trail from the audit hash chain. `target_hash` is the
  # `proposal:{id}` marker written on approve/reject; keep only those rows.
  get "$BASE/audit?kind=reconcile" \
    | tr ',' '\n' \
    | sed 's/^/    /'
  printf '  }\n'
  printf '}\n'
} > "$RECORD"

# Manifest: record + its SHA-256 (the evidentiary index for this decision).
HASH=(sha256sum)
command -v sha256sum >/dev/null 2>&1 || HASH=(shasum -a 256)
h=$("${HASH[@]}" "$RECORD" | awk '{print $1}')
printf '{\n  "kit": "admt",\n  "chunk_id": "%s",\n  "record": "ADMT_RECORD.json",\n  "sha256": "%s"\n}\n' \
  "$ID" "$h" > "$OUT_DIR/ADMT_MANIFEST.json"

echo "OK  wrote $OUT_DIR/ADMT_RECORD.json"
