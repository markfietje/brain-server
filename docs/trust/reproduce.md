# Reproduce — verify the whole posture in one pass

> **What this is:** a scripted, read-only walk-through of every claim in the
> [proof map](./proof-map.md), against a **fresh throwaway instance** so you
> can reproduce the security/compliance posture without touching production
> data. This is the artifact that turns "trust us" into "verify it" in a SOC 2
> / vendor-assessment conversation.
>
> Requirements: the `brain-server` binary, the `brain` CLI, `jq`, `curl`, and a
> throwaway DB path. Runs ~3 minutes.

## 0. Fresh throwaway instance

```sh
DB=/tmp/brain-repro-$$.db
PORT=18799
BRAIN_DB_PATH=$DB BIND_PORT=$PORT BRAIN_WORKER_THREADS=2 \
  ./target/release/brain-server &      # or via the installed binary
SVC=$!
sleep 2
B="localhost:$PORT"
```

## 1. Tamper-evident audit chain

```sh
curl -s "$B/audit/verify"                       # {"ok":true}
curl -s "$B/audit?limit=3" | jq '.[0].prev_hash'  # non-null backref
```

## 2. Human-in-the-loop write gate (nothing auto-promotes)

```sh
curl -s -X POST "$B/ingest/proposal" -H 'content-type: application/json' \
  -d '{"content":"acme ships monthly","title":"t"}'
# → a proposal id, NOT a knowledge row.
curl -s "$B/proposals?status=pending" | jq 'length'   # ≥ 1
D=$(curl -s "$B/proposals?status=pending" | jq -r '.[0].content_digest')
curl -s -X POST "$B/proposals/1/approve?digest=$D"    # promote → chunk_id (digest binds to displayed bytes)
curl -s "$B/search?q=acme" | jq '.hits[0].content'    # now recallable
```

## 3. DSAR → chain-verifiable deletion certificate

```sh
curl -s -X POST "$B/dsar" -H 'content-type: application/json' \
  -d '{"owner":"repro-user"}' | jq '.certificate_id'
CERT=$(curl -s "$B/dsar" ... | jq -r '.certificate_id')
curl -s "$B/dsar/$CERT/certificate" | jq '.chain_verifies'   # true
curl -s "$B/tombstones" | jq 'length'                          # ≥ 1
```

## 4. OIDC + JWKS + UMP L3 + capability tokens

```sh
curl -s "$B/.well-known/jwks.json" | jq '.keys | length'   # ≥ 1
curl -s "$B/ump/capabilities" | jq '.conformance'          # "UMP 1.0 / L3"
brain ump keygen --dir /tmp/brain-ump-repro                  # mint a token
# read-only token on a write → 401 (see proof-map row)
```

## 5. Health + hardening + capacity

```sh
curl -s "$B/health" | jq '{hardening, capacity}'
curl -s "$B/.well-known/ai-notice" | jq '.origin_metadata'
```

## 6. Injection screen quarantines, it doesn't delete

```sh
curl -s -X POST "$B/ingest" -H 'content-type: application/json' \
  -d '{"content":"normal content"}'
# a screen-flagged payload → stored flagged (read-only probe in the docs)
curl -s "$B/health" | jq '.injection_classifier_loaded'
```

## 7. Tear down

```sh
kill $SVC
rm -f "$DB" "$DB"-* /tmp/brain-ump-repro 2>/dev/null || true
echo "repro complete: every row of the proof map verified live"
```

## Notes / honest caveats

- The commands above are a **skeleton** — the exact request bodies for DSAR and
  the injection-screen probe are pinned by the repo's integration tests
  (`cargo test --features bench`, `test_observe_dsar_locate_and_purge_semantics`
  + the screen tests). Follow those for byte-exact payloads.
- OTel/SSE/SOC-2-kit rows **shipped** (v1.20.7 / v1.20.8 / v1.20.10) — the
  proof map marks them so; they are claimed there, not re-proven here.
- AuthN rows need `BRAIN_JWT_ISSUER` + a key dir to fully exercise; the opaque-
  token default covers the audit/gate/DSAR/UMP rows unauthenticated.
