//! Boot-time guards: the argv gate, worker-thread resolution, and the
//! fail-closed loopback-bind check beside the constant-time token compare.
//! Pure std + config — `main` calls in from `main.rs`.

use anyhow::Result;
use std::net::SocketAddr;

use crate::auth;
use crate::config;
use crate::config::SERVER_VERSION;

// replaced a hand-rolled fold with `subtle::ConstantTimeEq`, which
// is backed by asm/black_box primitives that the optimizer cannot short-
// circuit. `subtle` is already a transitive dep (sha2/hmac/aes-gcm), so this
// adds zero build surface. The length check below is inherently leaky, but
// token length is not secret for a fixed-format random token.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq as _;
    a.len() == b.len() && a.ct_eq(b).unwrap_u8() == 1
}

/// Handle CLI flags before any side effect. Prints version/usage and exits;
/// rejects unknown `-`-prefixed flags so the server never starts silently on
/// a typo (e.g. `brain-server --version` previously launched the server).
/// Positional args are allowed through (back-compat for any wrapper script).
pub(crate) fn handle_cli_args() {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-V" | "--version" => {
                println!("brain-server {}", SERVER_VERSION);
                std::process::exit(0);
            }
            "-h" | "--help" => {
                println!(
                    "brain-server {} — HTTP memory/recall server",
                    SERVER_VERSION
                );
                println!();
                println!("Run as a launchd service (see scripts/install-service.sh) or directly:");
                println!("  brain-server                start server on $BIND_HOST:$BIND_PORT");
                println!("  brain-server --version      print version and exit");
                println!("  brain-server --help         print this help and exit");
                println!(
                    "  brain-server --re-embed <profile>  rebuild the vector store at a profile's dim, then exit"
                );
                println!(
                    "  brain-server --re-audit     re-anchor the audit chain under hmac256 (v1.27.31), then exit"
                );
                println!();
                println!("Env: BIND_HOST, BIND_PORT, BRAIN_DB_PATH, AUTH_TOKEN_FILE, RUST_LOG");
                println!(
                    "      BRAIN_AUDIT_CHAIN_KEY / BRAIN_AUDIT_CHAIN_KEY_FILE (audit chain HMAC key)"
                );
                std::process::exit(0);
            }
            // Offline one-shot modes handled later in main_inner — passthrough.
            "--re-embed" | "--re-audit" => {}
            other if other.starts_with('-') => {
                eprintln!("brain-server: unknown flag '{other}'");
                eprintln!("  pass --help for usage, or run with no args to start the server");
                std::process::exit(2);
            }
            _ => {}
        }
    }
}

pub(crate) fn worker_threads() -> Option<usize> {
    std::env::var("BRAIN_WORKER_THREADS")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|&n| n > 0)
}

// ── startup bind fail-closed ────────────────
// `handlers/mod.rs` treats a `None` principal as superuser (by-design
// loopback). The symmetric gap: a non-loopback bind with no AUTH_TOKEN/JWT is
// an open superuser API. Fail-closed file-perms checks already exist; this is the
// matching posture on the bind side. Two pure predicates + one guard, all
// unit-testable without a live socket.
//
// ponytail: startup-only enforcement — once running, a rebind is not re-checked
// (the OS socket is already bound). Does NOT add per-principal rate limiting
// (v2.1, needs Redis) or change the in-memory per-IP limiter.

/// True when the resolved bind address is loopback (`127.0.0.0/8` or `::1`).
/// `SocketAddr` always carries a resolved IP, so a hostname like `localhost`
/// never reaches here as a string — it either resolved to 127.0.0.1 (loopback)
/// or the startup path already exited in the parse-failure branch above.
pub(crate) fn bind_is_loopback(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}

/// True when SOME auth gate is configured: a non-empty opaque-token set OR JWT
/// mode. Reuses `config::auth_tokens()` + `AuthMode` — does not duplicate token
/// resolution. A non-loopback bind with this false is an open superuser API.
pub(crate) fn auth_configured(auth_mode: auth::AuthMode) -> bool {
    auth_mode.is_jwt() || !config::auth_tokens().is_empty()
}

/// Refuse to start if the bind is beyond loopback AND no auth is configured.
/// The same posture applied to the bind side (fail-closed, clear message, exit).
pub(crate) fn enforce_loopback_bind_guard(
    addr: &SocketAddr,
    auth_mode: auth::AuthMode,
) -> Result<()> {
    if !bind_is_loopback(addr) && !auth_configured(auth_mode) {
        return Err(anyhow::anyhow!(
            "refusing to start: non-loopback bind ({}) with no AUTH_TOKEN/JWT — \
             this would expose an unauthenticated superuser API. \
             Set AUTH_TOKEN_FILE, configure JWT, or bind to 127.0.0.1/::1.",
            addr
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ct_eq() {
        // Equal, same length.
        assert!(ct_eq(b"abcdef", b"abcdef"));
        // Differ in one byte, same length → false (no early exit path).
        assert!(!ct_eq(b"abcdef", b"abcXef"));
        // Differ in last byte → false.
        assert!(!ct_eq(b"abcdef", b"abcdeX"));
        // Different length → false.
        assert!(!ct_eq(b"abcdef", b"abc"));
        assert!(!ct_eq(b"abc", b"abcdef"));
        // Empty slices compare equal.
        assert!(ct_eq(b"", b""));
    }

    /// every non-public route's handler must
    /// `None`-principal-is-superuser behavior above — a non-loopback bind with
    /// no auth must refuse startup. Pure predicates + guard, no live socket.
    #[test]
    fn bind_is_loopback_and_auth_configured_predicates() {
        use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

        let loopback_v4 = SocketAddr::from((Ipv4Addr::new(127, 0, 0, 1), 8765));
        let loopback_v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, 8765));
        let any_v4 = SocketAddr::from((Ipv4Addr::new(0, 0, 0, 0), 8765));
        let site = SocketAddr::from((Ipv4Addr::new(192, 168, 1, 10), 8765));

        // bind_is_loopback: 127.0.0.0/8 + ::1 are loopback; 0.0.0.0 / site IPs
        // are not. (`localhost` as a hostname never reaches here as a SocketAddr
        // — it resolves to 127.0.0.1 upstream or exits in the parse-fail branch.)
        assert!(bind_is_loopback(&loopback_v4));
        assert!(bind_is_loopback(&loopback_v6));
        assert!(!bind_is_loopback(&any_v4));
        assert!(!bind_is_loopback(&site));

        // auth_configured: opaque tokens OR JWT. Empty tokens + Opaque => false.
        // SAFETY for env mutation: this process has no AUTH_TOKEN_FILE set during
        // the normal test run, so auth_tokens() is empty here. We assert both
        // arms of AuthMode against the same (empty-token) environment.
        let no_tokens_no_jwt = auth_configured(auth::AuthMode::Opaque);
        let jwt_mode = auth_configured(auth::AuthMode::Jwt);
        assert!(
            !no_tokens_no_jwt,
            "opaque mode with no tokens must be unauthenticated"
        );
        assert!(
            jwt_mode,
            "JWT mode counts as configured even with no opaque token"
        );

        // The guard: back-compat preserved (loopback + no-auth => Ok), and the
        // gap closed (non-loopback + no-auth => Err). Non-loopback + JWT => Ok.
        assert!(enforce_loopback_bind_guard(&loopback_v4, auth::AuthMode::Opaque).is_ok());
        assert!(enforce_loopback_bind_guard(&loopback_v6, auth::AuthMode::Opaque).is_ok());
        assert!(
            enforce_loopback_bind_guard(&any_v4, auth::AuthMode::Opaque).is_err(),
            "0.0.0.0 with no auth must refuse startup"
        );
        assert!(
            enforce_loopback_bind_guard(&site, auth::AuthMode::Opaque).is_err(),
            "site IP with no auth must refuse startup"
        );
        assert!(
            enforce_loopback_bind_guard(&site, auth::AuthMode::Jwt).is_ok(),
            "site IP with JWT configured is a valid (authenticated) public bind"
        );
    }
}
