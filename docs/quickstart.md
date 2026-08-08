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

## 4. Recall

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

## 5. Use the CLI

The `brain` binary gives you the same surface from a terminal:

```bash
./target/release/brain status          # health + stats
./target/release/brain query "blueberry alternative" --k 3
./target/release/brain explain "blueberry alternative"
./target/release/brain ingest-dir ./vault
```

---

## 6. Run as a service (macOS)

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
