# Quickstart

Get Brain Server running on your machine and make your first recall in minutes.
It builds from source with the Rust toolchain; there are no external services.

---

## Prerequisites

- **Rust** (stable) with `cargo`. Get it at [rustup.rs](https://rustup.rs).
- macOS or Linux (any architecture Rust compiles to; ARM/Linux recommended for edge).

---

## 1. Build

```bash
# Build the server and the operator CLIs
cargo build --release --features bench

# Optionally include the GitHub connector binary
cargo build --release --features bench,connector-github
```

The release profile is size-optimized (`opt-level="z"`, `lto="fat"`,
`codegen-units=1`, `strip`, `panic="abort"`).

---

## 2. Run

```bash
./target/release/brain-server
```

The server binds to `127.0.0.1:8765` by default and creates a SQLite database at
the configured path (default `brain.db` in the current directory, or
`BRAIN_DB_PATH`).

```bash
# Liveness + stats
curl http://localhost:8765/health
curl http://localhost:8765/stats
```

> The server refuses to bind `0.0.0.0` unless `BIND_PUBLIC=1`. Loopback-safe by
> default.

---

## 3. Ingest

Ingest a markdown document. `[[relation::entity]]` links build the knowledge graph:

```bash
curl -X POST http://localhost:8765/ingest/markdown \
  -H 'Content-Type: application/json' \
  -d '{"title":"Bignay","content":"Bignay is [[alternative_to::blueberry]]. It has [[has_property::antioxidants]]."}'
```

For structured data, `POST /ingest` accepts explicit entities and relations.

---

## 4. Review — the human-in-the-loop gate

Write-back is human-gated by default. A candidate is **scored, not stored** — it becomes
memory only when a human approves it:

```bash
# Propose a fragment (scored; creates NO knowledge row)
curl -X POST http://localhost:8765/ingest/proposal \
  -H 'Content-Type: application/json' \
  -d '{"content":"Bignay is an antioxidant-rich alternative to blueberry."}'

# List the pending queue
curl http://localhost:8765/proposals?status=pending

# The human decides — approve into memory (optionally superseding a conflicting chunk)
curl -X POST http://localhost:8765/proposals/1/approve

# …or reject, audited, never deleted (note: the server records the rejection,
# not a free-text reason — any ?reason= is accepted but not persisted)
curl -X POST http://localhost:8765/proposals/1/reject
```

The web client at `/app` puts this in a control room: the **Review** panel (scoring
breakdown + sourcing prompt + screen verdict + raw evidence), the **Memory Operations**
panel (live SLA clocks + flagged inventory + gate health), and the **Agent Memory
Register** (a read-only provenance ledger). See
[**Human in the loop**](./human-in-the-loop.md) for how to evaluate a proposal well —
not just clear the queue.

---

## 5. Recall

Structured recall returns ranked evidence with provenance:

```bash
curl -X POST http://localhost:8765/recall \
  -H 'Content-Type: application/json' \
  -d '{"query":"blueberry alternative","provenance":true}'
```

Explore the knowledge graph:

```bash
curl http://localhost:8765/graph/entity/bignay
curl 'http://localhost:8765/graph/traverse?start=bignay&max_depth=2'
```

---

## 6. Use the CLI

The `brain` binary gives you the same surface from a terminal:

```bash
./target/release/brain status          # health + stats
./target/release/brain query "blueberry alternative" --k 3
./target/release/brain explain "blueberry alternative"
./target/release/brain ingest-dir ./vault
```

---

## 7. Run as a service (macOS)

For a persistent install managed by launchd:

```bash
scripts/install-service.sh
```

This builds the release binaries, installs them to `~/.local/bin`, relocates the
auth token to a 0600 file, restarts the service, and waits for `/health`. See
[Deployment](./deployment.md) for details and the client GUI.

---

## Next steps

- Configure authentication and other tunables in [Deployment](./deployment.md).
- Understand the retrieval pipeline in [Architecture](./architecture.md).
- Review the security posture in [Security](./security.md).
- Learn the write-back review job in [Human in the loop](./human-in-the-loop.md).
