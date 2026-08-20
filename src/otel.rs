//! feature-gated span-field helpers.
//!
//! Compiled ONLY under `--features otel` (the module declaration in `lib.rs`
//! is `#[cfg(feature = "otel")]`), so a default build compiles nothing here
//! and carries zero tracing overhead and zero new dependencies. The ingest /
//! recall / gate cores are instrumented with
//! `#[cfg_attr(feature = "otel", tracing::instrument(...))]`; every field this
//! module records is a *label* or a short hash — never the content body (the
//! PII rule in `SECURITY.md`).

use crate::screen::ScreenResult;

/// Opaque short hash of a recall query, so an operator can correlate a span to
/// the query it answered without exporting the query text. Delegates to the
/// codebase-wide audit hash (SHA-256 — a fingerprint, not a leak).
pub fn query_hash(query: &str) -> String {
    crate::audit::hash(query)
}

/// Span label for a screen verdict (`clean`/`quarantine`/`reject`).
pub fn screen_verdict_span(r: ScreenResult) -> &'static str {
    match r {
        ScreenResult::Clean => "clean",
        ScreenResult::Quarantine => "quarantine",
        ScreenResult::Reject => "reject",
    }
}

/// Span label for a gate decision outcome. `kind` names the operation
/// (`approved`); the result reports `ok` vs `error`. Never carries a body.
pub fn gate_outcome(
    kind: &'static str,
    res: &Result<serde_json::Value, crate::handlers::HandlerError>,
) -> &'static str {
    match res {
        Ok(_) => kind,
        Err(_) => "error",
    }
}

/// Build the OTLP/HTTP trace exporter + provider for `endpoint` and register it
/// as the process-global tracer provider (so `tracing_opentelemetry::layer()`
/// in `main.rs` can `.with_tracer(provider)`). `BatchSpanProcessor` spawns its
/// own worker thread internally, so there's no tokio-runtime coupling
/// (Jetson-core friendly, per the Cargo.toml note). A failed build is
/// best-effort — the caller logs and falls back to fmt-only logging; recall
/// stays the job.
pub fn init_otel(endpoint: &str) -> anyhow::Result<opentelemetry_sdk::trace::SdkTracerProvider> {
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::trace::SdkTracerProvider;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .build()?;
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();
    opentelemetry::global::set_tracer_provider(provider.clone());
    Ok(provider)
}
