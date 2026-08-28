#!/bin/sh
# v1.28.50 "Aqueduct" live smoke — multi-db DB COPY, release binary.
# Recall end-to-end (vector + FTS + graph legs), trace replay, screened +
# quarantined ingest, dedup, /audit/verify — against a scratch copy of the
# live workspace DB (the live DB is never touched).
set -e
PORT="${1:-18485}"
SCRATCH="$(mktemp -d /tmp/aqueduct-smoke.XXXXXX)"
echo "scratch: $SCRATCH"
sqlite3 ~/.openclaw/workspace/brain.db ".backup '$SCRATCH/brain.db'"
echo "db copied: $(ls -la "$SCRATCH/brain.db" | awk '{print $5}') bytes"

BIND_PORT="$PORT" \
BRAIN_DB_PATH="$SCRATCH/brain.db" \
BRAIN_MULTI_DB=1 \
BRAIN_AUDIT_READ_EVENTS=true \
target/release/brain-server >"$SCRATCH/server.log" 2>&1 &
PID=$!
echo "$PID" > /tmp/aqueduct-smoke-pid
trap 'kill "$PID" 2>/dev/null || true' EXIT
i=0
while [ "$i" -lt 60 ]; do
  if curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then break; fi
  sleep 1
  i=$((i+1))
done
curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null && echo "health ok"

B="http://127.0.0.1:$PORT"

# 1. multi-db: register a second domain (its own brain-<domain>.db file).
curl -fsS -X POST "$B/domains" -H 'Content-Type: application/json' \
  -d '{"name":"smoke-aqua"}' | head -c 200; echo " <- domain create"
ls "$SCRATCH" | grep "brain-smoke-aqua" && echo "domain db file ok"

# 2. screened ingest (benign) into global.
R1=$(curl -fsS -X POST "$B/ingest" -H 'Content-Type: application/json' \
  -d '{"title":"aqueduct smoke","content":"The Aqueduct release moves the recall core into the service layer with RRF cross-domain fusion.","domain":"global"}')
echo "$R1" | head -c 220; echo " <- benign ingest"
echo "$R1" | grep -q '"status":"created"' && echo "screened ingest ok"

# 3. dedup: the SAME content again → duplicate receipt with the first id.
ID1=$(echo "$R1" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
R2=$(curl -fsS -X POST "$B/ingest" -H 'Content-Type: application/json' \
  -d '{"title":"aqueduct smoke","content":"The Aqueduct release moves the recall core into the service layer with RRF cross-domain fusion.","domain":"global"}')
echo "$R2" | head -c 220; echo " <- dedup ingest"
echo "$R2" | grep -q '"status":"duplicate"' && echo "dedup receipt ok"
ID2=$(echo "$R2" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
[ "$ID1" = "$ID2" ] && echo "dedup id match ok ($ID1)"

# 4. quarantined path: scrape without a documented lawful basis → stored + flagged.
R3=$(curl -fsS -X POST "$B/ingest" -H 'Content-Type: application/json' \
  -d '{"title":"scraped","content":"Scraped pricing table content for the smoke run.","domain":"global","source":"scrape"}')
echo "$R3" | head -c 220; echo " <- scraped ingest (no lawful basis)"
ID3=$(echo "$R3" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
FLAGGED=$(sqlite3 "$SCRATCH/brain.db" "SELECT flagged FROM knowledge WHERE id=$ID3")
[ "$FLAGGED" = "1" ] && echo "quarantine flag ok (stored + flagged)"

# 5. ingest into the second domain (multi-db leg).
R4=$(curl -fsS -X POST "$B/ingest" -H 'Content-Type: application/json' \
  -d '{"title":"domain note","content":"Smoke-aqua domain carries its own island of memory for the federation merge.","domain":"smoke-aqua"}')
echo "$R4" | head -c 220; echo " <- domain ingest"

# 6. recall end-to-end — vector + FTS + graph legs across domains.
curl -fsS -X POST "$B/recall" -H 'Content-Type: application/json' \
  -d '{"query":"Aqueduct recall core service layer","limit":5,"provenance":true}' \
  -o "$SCRATCH/recall.json"
python3 - "$SCRATCH/recall.json" <<'EOF'
import sys, json
r = json.load(open(sys.argv[1]))
hits = r["hits"]
assert r["decision"] in ("ok", "low_confidence"), r["decision"]
srcs = sorted({(h.get("source") or "none") for h in hits})
doms = sorted({(h.get("domain") or "?") for h in hits})
print("recall ok: decision=%s hits=%d sources=%s domains=%s" % (r["decision"], len(hits), srcs, doms))
assert any(h.get("domain") == "smoke-aqua" for h in hits), "cross-domain federation leg missing"
assert all(h.get("untrusted") is True for h in hits), "untrusted taint missing"
EOF
echo "recall federation ok"

# 7. trace replay: trace:true → trace_id → GET /recall/{id}/trace.
curl -fsS -X POST "$B/recall" -H 'Content-Type: application/json' \
  -d '{"query":"Aqueduct recall core service layer","limit":5,"trace":true}' \
  -o "$SCRATCH/recall_trace.json"
TID=$(python3 -c 'import sys,json;print(json.load(open(sys.argv[1])).get("trace_id") or "")' "$SCRATCH/recall_trace.json")
[ -n "$TID" ] && echo "trace id ok ($TID)"
curl -fsS "$B/recall/$TID/trace" -o "$SCRATCH/trace.json"
python3 - "$SCRATCH/trace.json" <<'EOF'
import sys, json
t = json.load(open(sys.argv[1]))
assert "query_hash" in t, "trace must carry the hash, never raw text"
raw = open(sys.argv[1]).read()
assert "Aqueduct recall core" not in raw, "raw query leaked into the trace"
print("trace replay ok (hash-only)")
EOF

# 8. audit chain verify on every chain.
V=$(curl -fsS "$B/audit/verify")
echo "$V" | head -c 200; echo " <- audit/verify"
echo "$V" | grep -q '"ok"' && echo "audit verify ok"

echo "SMOKE COMPLETE (scratch kept at $SCRATCH)"
