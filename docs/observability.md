# Observability — metrics, audit, traces, health

Brain Server ships a small but honest observability surface: a Prometheus-format
`/metrics` endpoint, an append-only SHA-256 audit chain, optional recall decision
traces, an optional OpenTelemetry trace export, and health/stats/version
endpoints. Everything is **local-first**: metrics and audit are on-device, and
OpenTelemetry is opt-in (off by default, no data egress unless configured).

This page is verified against `src/main.rs` (`metrics`, `list_audit`,
`verify_audit_chain`, `health`, `stats`, `version`), `src/audit.rs`, and
`src/otel.rs`.

## Metrics (`GET /metrics`)

Prometheus text exposition, **auth-gated** (a `Read` principal is required —
a `403` with the reason keeps the non-JSON contract). The gauges, verified from
source:

| Gauge | Meaning |
|---|---|
| `brain_rss_mib` | **This process's** RSS in MiB (not host-wide). Matches the capacity envelope `/health` reports. |
| `brain_pool_connections{state="idle"}` / `{state="busy"}` | SQLite connection-pool idle/busy counts. |
| `brain_capacity_status` | `1`=ok, `2`=warning, `3`=exceeded (mirrors the capacity envelope). |
| `brain_audit_chain_ok` | `1` = audit chain verifies, `0` = tamper detected. |

The audit-chain gauge uses a short-TTL cache so a scrape doesn't trigger a full
O(n) chain scan; `/audit/verify` (below) always gives the authoritative answer.

## Audit chain

An append-only, hash-chained audit ledger records ingest, approvals, denials,
auth failures, read events (opt-in), purges, and DSARs. Content is never stored
in the chain — only hashes (SHA-256 since v1.20.25).

- **`GET /audit`** — recent audit rows (Admin; `?since=` and `?principal=`
  filters are URL-addressable).
- **`GET /audit/verify`** — fresh, authoritative full-chain integrity check
  (Admin). Returns `{ ok: bool }`.
- **`GET /ump/audit`** / **`GET /ump/audit/verify`** — the UMP reference audit
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

## OpenTelemetry (opt-in, feature-gated)

A `src/otel.rs` module is compiled **only** under `--features otel` (a default
build compiles nothing here — zero tracing overhead, zero new dependencies). The
ingest / recall / gate cores are instrumented with `#[cfg_attr(feature = "otel",
tracing::instrument(...))]`.

- Enable with `BRAIN_OTEL_ENABLED` + `BRAIN_OTEL_ENDPOINT` (see
  **[Configuration](./configuration.md)**); the exporter is an OTLP/HTTP
  span exporter (`opentelemetry-otlp`).
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
- OpenTelemetry is **opt-in and feature-gated**; the default build has no trace
  export, by design.
- The audit gauge is cached for scrape safety; `/audit/verify` is authoritative.

## Next steps

- **[Configuration](./configuration.md)** — `BRAIN_AUDIT_*`, `BRAIN_OTEL_*`, `BRAIN_ALERT_WEBHOOK_*`.
- **[Security](./security.md)** — the audit chain and egress posture.
- **[Retrieval & Recall](./retrieval-and-recall.md)** — recall decision traces.