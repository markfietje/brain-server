# Observability — metrics, audit, traces, health

Brain Server ships a small but honest observability surface: a Prometheus-format
`/metrics` endpoint, an append-only SHA-256 audit chain, optional recall decision
traces, an OpenTelemetry trace export, and health/stats/version
endpoints. Everything is **local-first**: metrics and audit are on-device.
OpenTelemetry is feature-gated — a build without `--features otel` compiles no
exporter at all; on otel builds export is **enabled by default** and
`BRAIN_OTEL_ENABLED` (`0`/`false`/`no`/`off`) is the kill switch.

This page is verified against `src/server/router/core.rs` (the `/metrics`,
`/health/*`, `/audit*` surfaces), `src/audit/mod.rs`, and `src/otel.rs`. The
`/metrics` series list is machine-pinned to the metrics dictionary
(`src/docs_truth.rs` — a series cannot ship without a dictionary row).

## Metrics (`GET /metrics`)

Prometheus text exposition, **auth-gated** (a `Read` principal is required —
a `403` with the reason keeps the non-JSON contract). All twelve series,
verified from source:

| Series | Kind | Meaning |
|---|---|---|
| `brain_rss_mib` | gauge | **This process's** RSS in MiB (not host-wide). Matches the capacity envelope `/health` reports. |
| `brain_pool_connections{state="idle"}` / `{state="busy"}` | gauge | SQLite connection-pool idle/busy counts. |
| `brain_pool_in_use{domain}` | gauge | Connections currently checked out, per domain DB. |
| `brain_pool_idle{domain}` | gauge | Connections parked in the pool, per domain DB. |
| `brain_pool_timeouts_total` | counter | Acquire attempts that hit the pool timeout (visible contention). |
| `brain_busy_errors_total` | counter | SQLite `SQLITE_BUSY` errors returned to callers. |
| `brain_wal_pages_pending{domain}` | gauge | WAL frames not yet checkpointed, per domain DB — the write-pressure gauge. |
| `brain_lock_wait_micros_p50` | gauge | p50 of contended mutex/RwLock acquire waits (Headroom telemetry; try_lock fast paths read zero clock). |
| `brain_lock_wait_micros_p95` | gauge | p95 of the same histogram (fixed-bucket edges, no histograms crate). |
| `brain_db_busy_total` | counter | Busy-handler sleeps on the write path. |
| `brain_capacity_status` | gauge | `1`=ok, `2`=warning, `3`=exceeded (mirrors the capacity envelope). |
| `brain_audit_chain_ok` | gauge | `1` = audit chain verifies, `0` = tamper detected. |

Formulas, sources, and citations for every series live in the metrics
dictionary (`docs/metrics.md`, the "Server telemetry series" section).

The audit-chain gauge uses a short-TTL cache so a scrape doesn't trigger a full
O(n) chain scan; `/audit/verify` (below) always gives the authoritative answer.

### `/health/db` — the operator's detail read (Read-gated)

Beyond reachability, `/health/db` echoes the operating posture: the capacity
block, the hardening/concurrency block (`pool_timeouts_total`,
`busy_errors_total`, per-domain `wal_pages_pending`), the static boot-time
**durability echo** (`synchronous`, `wal_autocheckpoint_pages`,
`capacity_target` — what the write-posture envelope resolved to), and the
**loom boot decision** (whether the opt-in CPU-parallelism tier engaged, and
why or why not). Use it alongside `/metrics`: gauges are the trend,
`/health/db` is the configuration truth.

## Audit chain

An append-only, hash-chained audit ledger records ingest, approvals, denials,
auth failures, read events (opt-in), purges, and DSARs. Content is never stored
in the chain — only hashes (SHA-256 since v1.20.25).

- **`GET /audit`** — recent audit rows (Admin; `?since=` and `?principal=`
  filters are URL-addressable).
- **`GET /audit/verify`** — fresh, authoritative full-chain integrity check
  (Admin). Returns `{ ok: bool }`.
- **`POST /ump/audit`** / **`GET /ump/audit/verify`** — the UMP reference audit
  facility over the same chain.

Read-event auditing is controlled by `BRAIN_AUDIT_READ_EVENTS` (default `on` in
JWT mode, `off` on loopback) and `BRAIN_AUDIT_READ_SAMPLE_RATE` (default `1.0`).
See **[Configuration](./configuration.md)**.

## Recall decision traces

Read events may be recorded; when a recall runs with `trace: true` (or the
server's read-event audit is on), the response includes a `trace_id` (the audit
row id) that **`GET /recall/{trace_id}/trace`** replays — a step-by-step view of
the decision path (per-retriever ranks, fused score, applied scope). Trace
records store the **query hash**, never the raw query (a recall query can be
personal data). See **[Retrieval & Recall](./retrieval-and-recall.md)**.

## OpenTelemetry (feature-gated; on by default under `--features otel`)

A `src/otel.rs` module is compiled **only** under `--features otel` (a default
build compiles nothing here — zero tracing overhead, zero new dependencies). The
ingest / recall / gate cores are instrumented with `#[cfg_attr(feature = "otel",
tracing::instrument(...))]`; additional decision spans (`gate.edit`,
`compliance.export`) exist alongside the core spans.

- On otel builds export runs unless disabled: set `BRAIN_OTEL_ENABLED=0|false|no|off`
  to kill it; `BRAIN_OTEL_ENDPOINT` selects the collector (default
  `http://127.0.0.1:4318/v1/traces`). The exporter is OTLP/HTTP
  (`opentelemetry-otlp`).
- Every recorded span field is a **label or a short hash — never the content
  body** (the PII rule). Recall queries are recorded as `query_hash` (SHA-256
  fingerprint via the codebase-wide audit hash), screen verdicts as
  `clean`/`quarantine`/`reject`, and gate outcomes as `ok`/`error`.
- A failed exporter build is **best-effort** — the server logs and falls back
  to fmt-only logging; recall stays the job.

## Health, readiness, stats, version

| Endpoint | Purpose |
|---|---|
| `GET /health` | Liveness (always auth-exempt). |
| `GET /health/db` | Database reachability. |
| `GET /ready` | Readiness. |
| `GET /stats` | Operational counters. |
| `GET /version` | Server version. |

## Alerting

There is also an in-process **alert feed** (`GET /events`, Server-Sent Events)
and an opt-in outbound **system-alert webhook** (`BRAIN_ALERT_WEBHOOK_URL` /
`BRAIN_ALERT_WEBHOOK_SECRET`, Standard Webhooks signed, redirect-refusing). See
**[Security](./security.md)** for the egress posture.

## Honest ceiling

- `/metrics` is a compact, purpose-built set of gauges — it is not a full
  runtime-profiling endpoint (no pprof, no per-request histograms).
- OpenTelemetry is **feature-gated**; a build without `--features otel` has no
  trace export, by design. On otel builds it is on unless the kill switch
  (`BRAIN_OTEL_ENABLED=0|false|no|off`) is thrown.
- The audit gauge is cached for scrape safety; `/audit/verify` is authoritative.

## Next steps

- **[Configuration](./configuration.md)** — `BRAIN_AUDIT_*`, `BRAIN_OTEL_*`, `BRAIN_ALERT_WEBHOOK_*`.
- **[Security](./security.md)** — the audit chain and egress posture.
- **[Retrieval & Recall](./retrieval-and-recall.md)** — recall decision traces.