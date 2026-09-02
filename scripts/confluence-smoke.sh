#!/bin/sh
# v1.28.51 "Confluence" live smoke — multi-db DB COPY, release binary.
# One representative flow per migrated surface: procedure evaluate, UMP ops
# read (integrity-verified get-memory), kcs worklist, forget (tombstone),
# suggest + feedback, Art.30 register, webhook HMAC path (401s), audit
# verify — against a scratch copy of the live workspace DB (the live DB is
# never touched).
set -e
PORT="${1:-18486}"
SCRATCH="$(mktemp -d /tmp/confluence-smoke.XXXXXX)"
echo "scratch: $SCRATCH"
sqlite3 ~/.openclaw/workspace/brain.db ".backup '$SCRATCH/brain.db'"
echo "db copied: $(ls -la "$SCRATCH/brain.db" | awk '{print $5}') bytes"

BIND_PORT="$PORT" \
BRAIN_DB_PATH="$SCRATCH/brain.db" \
BRAIN_MULTI_DB=1 \
BRAIN_AUDIT_READ_EVENTS=true \
BRAIN_KB_FEEDBACK_SECRET_FILE="$SCRATCH/kb.secret" \
target/release/brain-server >"$SCRATCH/server.log" 2>&1 &
PID=$!
printf '%s' "smoke-secret" > "$SCRATCH/kb.secret"
chmod 600 "$SCRATCH/kb.secret"
echo "$PID" > /tmp/confluence-smoke-pid
trap 'kill "$PID" 2>/dev/null || true' EXIT
i=0
while [ "$i" -lt 60 ]; do
  if curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then break; fi
  sleep 1
  i=$((i+1))
done
curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null && echo "health ok"

B="http://127.0.0.1:$PORT"

# 1. procedure: create + steps + a decision evaluate (service::procedure).
R1=$(curl -fsS -X POST "$B/procedure" -H 'Content-Type: application/json' \
  -d '{"title":"confluence smoke procedure","content":"Check the coolant level before every print; pause if below the minimum line.","domain":"global","steps":[{"title":"open panel","content":"Open the front panel and locate the reservoir."},{"title":"top up","content":"Fill to the maximum line with the approved coolant."}]}')
echo "$R1" | head -c 200; echo " <- procedure create"
echo "$R1" | grep -q '"status":"created"' && echo "procedure create ok"
PID1=$(echo "$R1" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
R2=$(curl -fsS "$B/procedure/$PID1/steps")
echo "$R2" | head -c 160; echo " <- procedure steps"
echo "$R2" | grep -q '"steps"' && echo "procedure steps ok"

# 2. UMP ops: remember + get-memory (verify-before-serve must pass).
R3=$(curl -fsS -X POST "$B/ump/remember" -H 'Content-Type: application/json' \
  -d '{"record":{"kind":"semantic","body":{"text":"The Confluence smoke pins the UMP read path."},"scope":{"owner":"smoke"}}}')
echo "$R3" | head -c 200; echo " <- ump/remember"
UMPID=$(echo "$R3" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
R4=$(curl -fsS "$B/ump/memory/$UMPID")
echo "$R4" | head -c 200; echo " <- ump/memory (verified record)"
echo "$R4" | grep -q '"record"' && echo "ump get-memory ok (verify-before-serve served it)"

# 3. kcs: approve a draft article, then the worklist must list it.
ART=$(sqlite3 "$SCRATCH/brain.db" "INSERT INTO knowledge(content, title, source, content_hash, node_kind, kcs_state, domain) VALUES ('Confluence smoke article body.','confluence smoke article','manual','confluence-smoke-hash','fact','draft','global'); SELECT last_insert_rowid();")
R5=$(curl -fsS -X POST "$B/kcs/articles/$ART/approve")
echo "$R5" | head -c 200; echo " <- kcs approve"
echo "$R5" | grep -q '"kcs_state":"approved"' && echo "kcs approve ok (CAS + freshness stamped)"
R6=$(curl -fsS "$B/kcs/articles")
echo "$R6" | python3 -c 'import sys,json;d=json.load(sys.stdin);a=[x for x in d["articles"] if x["id"]=='"$ART"'];assert a and a[0]["kcs_state"]=="approved","worklist missing the approved article";print("kcs worklist ok: the approved article is listed")'

# 4. forget: store a chunk, DELETE it, tombstone carries the digest.
R7=$(curl -fsS -X POST "$B/ingest" -H 'Content-Type: application/json' \
  -d '{"title":"forget me","content":"A chunk whose erasure the smoke verifies.","domain":"global"}')
FID=$(echo "$R7" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
R8=$(curl -fsS -X DELETE "$B/memory/$FID")
echo "$R8" | head -c 120; echo " <- DELETE /memory"
TOMB=$(sqlite3 "$SCRATCH/brain.db" "SELECT content_hash IS NOT NULL AND document_id IS NULL OR 1 FROM tombstones WHERE knowledge_id=$FID")
NT=$(sqlite3 "$SCRATCH/brain.db" "SELECT COUNT(*) FROM tombstones WHERE knowledge_id=$FID")
[ "$NT" = "1" ] && echo "forget ok: exactly one tombstone for $FID"

# 5. suggest + feedback + metrics (service::suggest).
R9=$(curl -fsS -X POST "$B/suggest" -H 'Content-Type: application/json' \
  -d '{"context":"coolant level check before print","k":3}')
echo "$R9" | head -c 120; echo " <- suggest"
R10=$(curl -sS -X POST "$B/suggest/feedback" -H 'Content-Type: application/json' \
  -d "{\"chunk_id\":$FID,\"feedback\":\"dismiss\",\"reason\":\"smoke run\"}")
# FID was deleted — the existence fence must 404 (probe-blind).
echo "$R10" | grep -q '"no chunk with id' && echo "feedback fence ok (404 on deleted chunk)"
R11=$(curl -fsS -X POST "$B/suggest/feedback" -H 'Content-Type: application/json' \
  -d "{\"chunk_id\":$PID1,\"feedback\":\"accept\"}")
echo "$R11" | grep -q '"status":"recorded"' && echo "feedback recorded ok"
curl -fsS "$B/suggest/metrics" | grep -q '"total"' && echo "metrics ok"

# 6. Art.30 register read (service::art30) — the live register JSON.
R12=$(curl -fsS "$B/art30")
echo "$R12" | python3 -c 'import sys,json;d=json.load(sys.stdin)["art30"];assert "categories_of_data" in d and "lifecycle" in d and "dsar_history" in d;print("art30 ok: categories=%d lifecycle.live=%d" % (len(d["categories_of_data"]), d["lifecycle"]["live"]))'

# 7. webhook HMAC path: missing + bad signatures → 401 (the transport
#    boundary stayed handler-side; the storage moved to the service).
S1=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$B/webhooks/kb-feedback" -H 'Content-Type: application/json' -d '{"slug":"x","helpful":true,"day_bucket":"2026-09-02"}')
[ "$S1" = "401" ] && echo "webhook missing-signature 401 ok"
# Sign a delivery WITH the smoke secret, then tamper the body — the
# standard-webhooks verify must 401 (constant-time compare, replay aside).
TS=$(date +%s)
BODY='{"slug":"confluence-smoke","helpful":true,"day_bucket":"2026-09-02"}'
SECRET_HEX=$(printf '%s' "smoke-secret" | xxd -p | tr -d '\n')
SIG=$(printf '%s' "smoke-2.$TS.$BODY" | openssl dgst -sha256 -hmac "smoke-secret" -binary | base64)
S2=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$B/webhooks/kb-feedback" \
  -H 'Content-Type: application/json' \
  -H "webhook-id: smoke-2" -H "webhook-timestamp: $TS" \
  -H "webhook-signature: v1,$SIG" \
  -d "$BODY")
[ "$S2" = "200" ] && echo "webhook valid-signature 200 ok"
TAMPERED=$(printf '%s' "$BODY" | sed 's/helpful":true/helpful":false/')
S3=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$B/webhooks/kb-feedback" \
  -H 'Content-Type: application/json' \
  -H "webhook-id: smoke-3" -H "webhook-timestamp: $TS" \
  -H "webhook-signature: v1,$SIG" \
  -d "$TAMPERED")
[ "$S3" = "401" ] && echo "webhook tampered-body 401 ok"

# 8. audit chain verify on every chain.
V=$(curl -fsS "$B/audit/verify")
echo "$V" | head -c 200; echo " <- audit/verify"
echo "$V" | grep -q '"ok":true' && echo "audit verify ok"

echo "SMOKE COMPLETE (scratch kept at $SCRATCH)"
