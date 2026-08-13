# Quickstart

Five minutes from running server to a verified recall. Commands assume the
`brain` CLI from [Install](./install.md) is on `$PATH`.

## 1. Run the server

```sh
# Build + install the service (see Install).
scripts/install-service.sh
brain doctor     # health: config, DB, auth, schema
```

## 2. Store a memory

```sh
# A memory (manual). The write gate proposes it; it becomes memory only on
# approval — see step 4.
curl -s -X POST localhost:8765/ingest/memory \
  -H 'content-type: application/json' \
  -d '{"items":[{"content":"the acme project ships on the first of every month"}]}'
```

(The `brain` CLI ingests whole **directories** — `brain ingest-dir ~/notes` — not single
snippets; for one memory use the HTTP endpoint above.)

## 3. Recall it

```sh
brain query "when does acme ship" --k 3
```

Every hit carries per-retriever provenance; add `--explain` to see the fused
score and decision path.

## 4. Approve the proposal (human-in-the-loop)

Memory candidates are *proposed*, never auto-promoted. Approve from the
console (`/ops` or Review) or the API:

```sh
# Find the pending proposal id.
curl -s 'localhost:8765/proposals?status=pending'
# Approve it (one tx, optional ?supersedes=<old_chunk_id>).
curl -s -X POST 'localhost:8765/proposals/1/approve'
```

## 5. Verify the audit chain

```sh
curl -s localhost:8765/audit/verify          # {"ok":true} — chain intact
curl -s localhost:8765/health | jq .         # service + corpus + capacity
```

## What just happened

A write hit the **injection screen** (blocklist + optional local classifier),
a candidate was **proposed** with deterministic novelty/conflict/salience
scores, a human **approved** it inside one transaction, and every step was
recorded in the **SHA-256 audit hash chain**. That's the whole posture: recall
that never thinks, writes a human can audit, and a chain a reviewer can verify.

## Next

- [`docs/overview.md`](../overview.md) — the full design.
- [`docs/architecture.md`](../architecture.md) — components + data flow.
- [`docs/api.md`](../api.md) and [`GET /openapi.yaml`](../../openapi.yaml) — the contract.
