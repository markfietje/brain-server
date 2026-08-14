# Quickstart

Five minutes from running server to a verified recall. Commands assume the
`brain` CLI from [Install](./install.md) is on `$PATH`.

> **Repository:** [github.com/markfietje/brain-server](https://github.com/markfietje/brain-server).
> Clone it (`git clone https://github.com/markfietje/brain-server.git`) or open
> the [releases](https://github.com/markfietje/brain-server/releases). Full
> install runbooks: [Deployment](../deployment.md) and [Docker](../docker.md).

## 1. Run the server

```sh
# Build + install the service (see Install).
scripts/install-service.sh
brain doctor     # health: config, DB, auth, schema
```

## 2. Store a memory

```sh
# A memory (manual). The write gate screens it; if the gate wants a human
# sign-off it holds it as a *proposal* (see step 4) instead of writing straight
# to memory.
curl -s -X POST http://localhost:8765/ingest/memory \
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

## 4. Review the gate (human-in-the-loop)

The server's injection screen runs on every write. If a write is flagged for a
human decision, it lands in the review queue as a **proposal** and only becomes
memory after approval:

```sh
# Find the pending proposal id (empty = the write passed the gate directly).
curl -s 'http://localhost:8765/proposals?status=pending'
# Approve it (one tx, optional ?supersedes=<old_chunk_id>).
curl -s -X POST 'http://localhost:8765/proposals/1/approve'
```

## 5. Verify the audit chain

```sh
curl -s http://localhost:8765/audit/verify          # {"ok":true} — chain intact
curl -s http://localhost:8765/health | jq .         # service + corpus + capacity
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
- [`docs/api.md`](../api.md) and [`openapi.yaml`](https://github.com/markfietje/brain-server/blob/main/openapi.yaml) — the contract.
