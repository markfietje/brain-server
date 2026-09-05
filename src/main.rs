//! Brain Server — version derived from Cargo.toml.
//!
//! THE THIN BINARY — the Spire Line's capstone. This file is
//! WIRING ONLY: argv/pool/model bootstrap (`server::bootstrap`) → router
//! composition (`server::router::app`) → serve + graceful shutdown. The law
//! it wires by, each clause machine-checked in `src/spire_inventory.rs`:
//!   * routes register ONLY under `src/server/router/**` — a route
//!     registration anywhere else under `src/` fails CI;
//!   * `server::bootstrap` stays protocol-free — no axum types;
//!   * this file stays ≤ 300 lines, with no `#[cfg(test)]` region — the
//!     test mass lives in `tests/` (`tests/main_suite.rs` + the family
//!     suites), and the crate-wide `#[test]` floor never decreases.
//!
//! Architecture Law: `docs/architecture.md` · the Spire Line's measured
//! before/after: `docs/SPIRE_AUDIT.md`.

use anyhow::Result;
use std::net::SocketAddr;
use tokio::signal;

// The secret-file mode-check seam, re-exported so shared modules compiled in
// this tree (connector/crm) reach it via the same `brain_server::secret_file` path
// as the lib tree.
#[allow(unused_imports)]
pub(crate) use brain_server::secret_file;

/// Entry point. The runtime is configurable via BRAIN_WORKER_THREADS
/// (default = cores; Jetson target = 2). Built here instead of `#[tokio::main]`
/// so the env var is read before the runtime starts.
fn main() {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if let Some(n) = brain_server::server::bootstrap::worker_threads() {
        builder.worker_threads(n);
    }
    let runtime = builder
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    if let Err(e) = runtime.block_on(main_inner()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

async fn main_inner() -> Result<()> {
    // ── bootstrap: everything up to the composed router ───────────────
    // Offline `--re-embed`/`--re-audit` modes run INSTEAD of serving and
    // exit Ok here (the bootstrap doc-comment freezes the order).
    let boot = brain_server::server::bootstrap::bootstrap()?;
    let brain_server::server::bootstrap::BootOutcome::Serve(boot) = boot else {
        return Ok(());
    };
    let brain_server::server::bootstrap::Bootstrap {
        state,
        addr,
        shutdown_pool,
        ..
    } = boot;

    let app = brain_server::server::router::app(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // the `timeout(drain_cap, axum::serve(...))`
    // was wrapping the ENTIRE serve lifetime, causing a 30s crash-loop on
    // systemd-managed deployments (the server would run for exactly
    // SHUTDOWN_DRAIN_SECS then exit). The timeout was intended to cap only
    // the drain phase, not the serving phase. Fixed: let the server run
    // indefinitely until SIGTERM, then axum's built-in drain handles the
    // rest. If a request hangs forever after SIGTERM, systemd's
    // TimeoutStopSec (default 90s) will kill the process — that's the
    // outer cap, not the application.
    //
    // `into_make_service_with_connect_info`
    // injects the peer `SocketAddr` extension on every request. Previously
    // the plain `serve` never provided it, so `rate_limit_middleware`'s
    // `req.extensions().get::<SocketAddr>()` was always `None` and every
    // client shared ONE "unknown" bucket — the per-IP limiter was a global
    // limiter in practice. With the extension present, the middleware keys
    // by remote address (XFF still honored only under `BRAIN_TRUST_PROXY=1`).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // checkpoint WAL on shutdown so a kill -9 or power loss
    // can't leave the live DB with un-replayed WAL frames. Best-effort: a
    // failure here is logged, not fatal (the OS will replay WAL on next open
    // anyway). `TRUNCATE` zeros the WAL file back to its minimum size.
    println!("📦 Checkpointing WAL...");
    if let Ok(conn) = shutdown_pool.get() {
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }

    Ok(())
}

/// Wait for SIGINT or SIGTERM (Unix) / Ctrl+C (Windows). Returns when either
/// fires; the caller uses this as axum's graceful-shutdown trigger.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => println!("\n🔔 Received SIGINT (Ctrl+C)"),
        _ = terminate => println!("\n🔔 Received SIGTERM"),
    }

    println!("\n🛑 Initiating graceful shutdown...");
}
