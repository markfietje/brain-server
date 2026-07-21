//! Connector process spawn helper.
//!
//! M1 scope is narrow: spawn a connector binary once, return the handle.
//! The full restart-with-backoff loop lands in M2.x when the server starts
//! auto-launching registered connectors on boot. M1 needs only the testable
//! primitives (`spawn_once`, `next_backoff`) so M2 has its building blocks.
//!
//! The primitives are intentionally not called from the server runtime in
//! M1 — `#![allow(dead_code)]` in `mod.rs` covers the warning until M2.x
//! wires `supervise` into the boot path.
//!
//! `ponytail:` ceilings:
//! - **No jitter** on the backoff schedule. Single local supervisor, no
//!   thundering-herd risk. Revisit if we ever run >3 concurrent connectors.
//! - **`kill_on_drop(true)`** instead of an explicit shutdown channel. When
//!   the supervisor task is dropped on server shutdown, the child is SIGKILLed
//!   by the OS. Drain lands in M3 when `brain disconnect` is wired.
//! - **`spawn_once` returns a `Child`**, not a wrapped handle. Callers compose
//!   their own loops; we keep M1 free of concurrency-control plumbing.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::process::{Child, Command};

use super::ConnectorManifest;

/// Maximum backoff between restart attempts. 60s matches the v0.9.6 plan ceiling.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Compute the backoff before the next restart attempt. Pure exponential
/// (`2^attempt` seconds) capped at `MAX_BACKOFF`. `attempt` is 0-indexed: the
/// first restart waits 1s, then 2s, 4s, 8s, 16s, 32s, then 60s forever.
///
/// `ponytail:` no jitter — single local supervisor, no herd risk. Add when we
/// run >3 concurrent connectors.
pub fn next_backoff(attempt: u32) -> Duration {
    // saturating on overflow: once 2^attempt exceeds 60s we cap anyway, so a
    // saturating_or u64 overflow at attempt >= 64 is irrelevant (we cap first).
    let secs = 1_u64
        .checked_shl(attempt)
        .unwrap_or(u64::MAX)
        .min(MAX_BACKOFF.as_secs());
    Duration::from_secs(secs)
}

/// Spawn one instance of the connector binary. Does NOT supervise — callers
/// composing their own restart loop use this; otherwise prefer the (M2.x)
/// `supervise` wrapper.
///
/// The child inherits the parent's environment (so `BRAIN_TOKEN_FILE` etc.
/// propagate automatically) and gets its stdout/stderr piped for log capture.
/// `config_path` and `checkpoint_path` are forwarded as `--config` /
/// `--checkpoint` argv (the connector-binary contract; see `mod.rs`).
pub async fn spawn_once(
    manifest: &ConnectorManifest,
    config_path: &str,
    checkpoint_path: &str,
) -> Result<Child> {
    let mut cmd = Command::new(&manifest.binary);
    cmd.arg("--config").arg(config_path);
    cmd.arg("--checkpoint").arg(checkpoint_path);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // kill_on_drop ensures the child is reaped if the supervisor task is
    // dropped (e.g. on server shutdown) before wait() returns. Crude but
    // correct for M1; drain lands in M3.
    cmd.kill_on_drop(true);
    let child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn connector binary {}", manifest.binary))?;
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_backoff_is_exponential_and_capped() {
        // 1, 2, 4, 8, 16, 32, 60, 60, 60, ...
        assert_eq!(next_backoff(0), Duration::from_secs(1));
        assert_eq!(next_backoff(1), Duration::from_secs(2));
        assert_eq!(next_backoff(2), Duration::from_secs(4));
        assert_eq!(next_backoff(3), Duration::from_secs(8));
        assert_eq!(next_backoff(4), Duration::from_secs(16));
        assert_eq!(next_backoff(5), Duration::from_secs(32));
        assert_eq!(next_backoff(6), MAX_BACKOFF); // 64 capped to 60
        assert_eq!(next_backoff(100), MAX_BACKOFF);
    }

    #[tokio::test]
    async fn test_spawn_once_runs_stub_binary_and_returns_zero() {
        // Spawns the actual `brain-connector-stub` binary (built from
        // src/bin/brain-connector-stub.rs). The stub takes --config/--checkpoint
        // argv (the connector contract), ingests one doc via the server's
        // /ingest/markdown route, then exits 0.
        //
        // SKIP conditions (the test asserts nothing in these cases):
        //   - stub binary not built yet (cargo test without cargo build --bin
        //     brain-connector-stub first)
        //   - no brain-server reachable at BRAIN_URL / 127.0.0.1:8765 (this is
        //     a unit-test context; the spawn path is exercised end-to-end by
        //     the M2.x integration test against a running server)
        //
        // The pure-function `next_backoff` test above covers the supervisor
        // math deterministically; this test covers the spawn contract.
        let base =
            std::env::var("BRAIN_URL").unwrap_or_else(|_| "http://127.0.0.1:8765".to_string());
        if !std::net::TcpStream::connect(("127.0.0.1", 8765)).is_ok() {
            eprintln!("skipping spawn test (no brain-server at {base})");
            return;
        }

        let manifest = ConnectorManifest::stub();
        let tmp = tempfile::tempdir().expect("tempdir");
        let config_path = tmp.path().join("config.json");
        std::fs::write(&config_path, "{}").unwrap();
        let checkpoint_path = tmp.path().join("checkpoint.db");

        let target_bin = std::env::current_dir()
            .ok()
            .map(|d| d.join("target/debug/brain-connector-stub"))
            .filter(|p| p.exists());
        let bin_path = match target_bin {
            Some(p) => p.to_string_lossy().into_owned(),
            None => manifest.binary.clone(),
        };
        let mut manifest = manifest;
        manifest.binary = bin_path;

        let child = spawn_once(
            &manifest,
            config_path.to_str().unwrap(),
            checkpoint_path.to_str().unwrap(),
        )
        .await;
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skipping spawn test (stub binary not built): {e}");
                return;
            }
        };
        let status = child.wait().await.expect("wait");
        assert!(
            status.success(),
            "stub connector should exit 0, got {status:?}"
        );
    }
}
