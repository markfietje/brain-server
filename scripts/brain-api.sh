#!/usr/bin/env bash
# brain-api.sh — the one safe way to call the brain-server from a shell.
#
# The operator token file carries TWO lines by design (line 1 = operator,
# line 2 = agent; scripts/secrets-truth.sh pins that layout), so the obvious
# `curl -H "Authorization: Bearer $(cat …)"` glues both lines into one header
# and the server answers with a bare 400. Every call in this script reads the
# correct line for the role and never prints the secret — output that names
# the token carries its SHA-256 digest only, matching the secrets-truth law.
#
#   scripts/brain-api.sh health                     server liveness
#   scripts/brain-api.sh ready                      readiness posture
#   scripts/brain-api.sh stats                      store counts
#   scripts/brain-api.sh domains                    list registered domains
#   scripts/brain-api.sh recall "query text"        deterministic recall
#     [--domain NAME] [--limit N] [--role ROLE]
#   scripts/brain-api.sh search "terms"             legacy GET search
#     [--limit N] [--role ROLE]
#   scripts/brain-api.sh get CHUNK_ID               fetch one chunk
#   scripts/brain-api.sh ingest TITLE --domain NAME <<'EOF' … EOF
#                                                   structured ingest (goes
#                                                   through the review queue
#                                                   when write posture is
#                                                   `review` — the response
#                                                   names the proposal id)
#   scripts/brain-api.sh proposals                  the review queue
#   scripts/brain-api.sh approve PROPOSAL_ID [--supersedes CHUNK_ID]
#                                                   fetches the full
#                                                   content_digest itself, so
#                                                   the ReviewArmour 409
#                                                   cannot happen
#   scripts/brain-api.sh auth                       show WHICH line of the
#                                                   token file each role reads
#                                                   and the token's sha256
#                                                   digest (never the token)
#
# Roles: `agent` (default; line 2 of the token file) and `operator` (line 1).
# The base URL is BRAIN_API_URL (default http://127.0.0.1:8765).
#
# Exit codes: 0 ok; 1 usage; 2 network/HTTP failure; 3 token file problem.
set -euo pipefail

BRAIN_API_URL="${BRAIN_API_URL:-http://127.0.0.1:8765}"
TOKEN_FILE="${BRAIN_TOKEN_FILE:-$HOME/.config/brain-server/auth-token}"

usage() { sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'; exit 1; }

die() { echo "brain-api: $*" >&2; exit "${2:-1}"; }

# Read ONE line of the token file. Nothing else ever sees the raw value:
# it is consumed inside command substitution into the curl header.
token_for() {
  local role="$1" line
  [ -r "$TOKEN_FILE" ] || die "token file not readable: $TOKEN_FILE" 3
  case "$role" in
    operator) line=1 ;;
    agent)    line=2 ;;
    *)        die "unknown role '$role' (operator|agent)" ;;
  esac
  local value
  value="$(sed -n "${line}p" "$TOKEN_FILE")"
  [ -n "$value" ] || die "token file line $line ($role) is empty: $TOKEN_FILE" 3
  printf '%s' "$value"
}

api() { # api ROLE METHOD PATH [JSON_BODY]
  local role="$1" method="$2" path="$3" body="${4:-}"
  local token
  token="$(token_for "$role")"
  local args=(-sS -m 30 -w '\n%{http_code}' -X "$method" "$BRAIN_API_URL$path"
              -H "Authorization: Bearer $token")
  if [ -n "$body" ]; then
    args+=(-H "Content-Type: application/json" -d "$body")
  fi
  local out
  out="$(curl "${args[@]}")" || die "curl failed for $method $path" 2
  local code="${out##*$'\n'}"
  local payload="${out%$'\n'*}"
  if [ "$code" -ge 400 ] 2>/dev/null; then
    printf '%s\n' "$payload" >&2
    die "HTTP $code from $method $path" 2
  fi
  printf '%s\n' "$payload"
}

main() {
  [ $# -ge 1 ] || usage
  local role="agent" domain="" limit="" supersedes=""
  local cmd="" positional=()

  # one pass: the first non-flag token is the command, the rest are its
  # positionals; --flags may appear anywhere
  while [ $# -ge 1 ]; do
    case "$1" in
      --role)       [ $# -ge 2 ] || usage; role="$2"; shift 2 ;;
      --domain)     [ $# -ge 2 ] || usage; domain="$2"; shift 2 ;;
      --limit)      [ $# -ge 2 ] || usage; limit="$2"; shift 2 ;;
      --supersedes) [ $# -ge 2 ] || usage; supersedes="$2"; shift 2 ;;
      --*)          usage ;;
      *)
        if [ -z "$cmd" ]; then cmd="$1"; else positional+=("$1"); fi
        shift
        ;;
    esac
  done

  [ -n "$cmd" ] || usage
  local a1="${positional[0]:-}" a2="${positional[1]:-}"

  case "$cmd" in
    health) api "$role" GET /health ;;
    ready)  api "$role" GET /ready ;;
    stats)  api "$role" GET /stats ;;
    domains) api "$role" GET /domains ;;
    get)
      [ -n "$a1" ] || die "get needs a chunk id"
      api "$role" GET "/get/$a1"
      ;;
    recall)
      [ -n "$a1" ] || die "recall needs a query string"
      local body
      body="$(RECALL_QUERY="$a1" RECALL_DOMAIN="$domain" RECALL_LIMIT="${limit:-5}" python3 -c '
import json, os
body = {"query": os.environ["RECALL_QUERY"], "limit": int(os.environ["RECALL_LIMIT"])}
if os.environ["RECALL_DOMAIN"]:
    body["domain"] = os.environ["RECALL_DOMAIN"]
print(json.dumps(body))
')"
      api "$role" POST /recall "$body"
      ;;
    search)
      [ -n "$a1" ] || die "search needs a query string"
      local q
      q="$(python3 -c 'import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))' "$a1")"
      api "$role" GET "/search?q=$q&k=${limit:-5}"
      ;;
    ingest)
      [ -n "$a1" ] || die "ingest needs a title, then content on stdin"
      [ -n "$domain" ] || die "ingest needs --domain (refusing the global default)"
      local body
      body="$(INGEST_TITLE="$a1" INGEST_DOMAIN="$domain" python3 -c '
import json, os, sys
print(json.dumps({
    "title": os.environ["INGEST_TITLE"],
    "domain": os.environ["INGEST_DOMAIN"],
    "content": sys.stdin.read(),
}))
')"
      api "$role" POST /ingest "$body"
      ;;
    proposals)
      api "$role" GET "/proposals?limit=${limit:-50}"
      ;;
    approve)
      [ -n "$a1" ] || die "approve needs a proposal id"
      local id="$a1" digest
      digest="$(api "$role" GET "/proposals?limit=200" \
        | python3 -c "import json,sys; print(next((p['content_digest'] for p in json.load(sys.stdin) if p['id']==int(sys.argv[1])), ''))" "$id")"
      [ -n "$digest" ] || die "proposal $id not found in the pending queue"
      local qs="digest=$digest"
      [ -n "$supersedes" ] && qs="$qs&supersedes=$supersedes"
      api "$role" POST "/proposals/$id/approve?$qs"
      ;;
    auth)
      for r in operator agent; do
        local t
        t="$(token_for "$r")"
        printf '%s: line %s of %s, sha256 %s\n' "$r" \
          "$([ "$r" = operator ] && echo 1 || echo 2)" "$TOKEN_FILE" \
          "$(printf '%s' "$t" | shasum -a 256 | cut -c1-12)"
      done
      ;;
    help|--help|-h) usage ;;
    *) die "unknown command '$cmd' (try: help)" ;;
  esac
}

main "$@"
