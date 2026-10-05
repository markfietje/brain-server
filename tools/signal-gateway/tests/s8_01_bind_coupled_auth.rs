//! The bind decision and the auth decision are ONE decision, not two.
//!
//! `main.rs` historically read `SIGNAL_GATEWAY_ALLOW_REMOTE` to gate the bind
//! and read `server.auth_token` to gate the router — two independent `if`s, so
//! opting into a remote interface also served the whole unauthenticated surface
//! (`POST /v2/send`, `GET /api/v1/accounts`, the JSON-RPC endpoint) to anyone
//! who could route a packet to the port. The log line for that path read
//! "loopback-only posture", which was the one posture the process had already
//! been told, by its own config, not to hold.
//!
//! These tests drive the real decision function `main.rs` calls, not a copy.

// Test-only: the crate denies panic vectors in PRODUCTION code (`Cargo.toml`
// `[lints.clippy]`). A test that cannot fail loudly is not a test. Same scoping
// as the test modules in `src/`.
#![allow(clippy::expect_used, clippy::panic)]

use signal_gateway::resolve_api_auth;
use std::net::SocketAddr;

const TOKEN: &str = "an-operator-chosen-token";

fn addr(s: &str) -> SocketAddr {
    match s.parse() {
        Ok(a) => a,
        Err(e) => panic!("fixture {s} must parse: {e}"),
    }
}

fn refusal(a: &str, token: Option<&str>, allow_remote: bool) -> Result<Option<String>, String> {
    resolve_api_auth(addr(a), token.map(str::to_owned), allow_remote)
}

// ── The two loopback postures must not regress ──────────────────────────────

#[test]
fn loopback_with_token_is_authenticated() {
    assert_eq!(
        refusal("127.0.0.1:8765", Some(TOKEN), false),
        Ok(Some(TOKEN.to_owned())),
        "loopback + a configured token must install that token"
    );
    // The whole 127.0.0.0/8 range is loopback, not just 127.0.0.1.
    assert_eq!(
        refusal("127.0.0.5:8765", Some(TOKEN), false),
        Ok(Some(TOKEN.to_owned()))
    );
    assert_eq!(
        refusal("[::1]:8765", Some(TOKEN), false),
        Ok(Some(TOKEN.to_owned()))
    );
}

#[test]
fn loopback_without_token_is_allowed() {
    // The documented tokenless posture — unchanged by this fix.
    assert_eq!(
        refusal("127.0.0.1:8765", None, false),
        Ok(None),
        "loopback + no token stays allowed; this is the intended posture"
    );
    assert_eq!(refusal("127.0.0.5:8765", None, false), Ok(None));
    assert_eq!(refusal("[::1]:8765", None, false), Ok(None));
    assert_eq!(refusal("[::1]:8765", None, true), Ok(None));
}

// ── The defect: a wildcard bind must never be served unauthenticated ─────────

#[test]
fn wildcard_v4_bind_without_token_is_refused_even_when_remote_is_allowed() {
    let err = refusal("0.0.0.0:8080", None, true)
        .expect_err("0.0.0.0 with no token must be refused; it is reachable from off-host");
    assert!(
        err.contains("0.0.0.0:8080"),
        "the refusal must name the address, got: {err}"
    );
    assert!(
        err.contains("auth_token"),
        "the refusal must point at the config that would fix it, got: {err}"
    );
}

#[test]
fn wildcard_v6_bind_without_token_is_refused_even_when_remote_is_allowed() {
    let err = refusal("[::]:8080", None, true)
        .expect_err("[::] with no token must be refused; it is reachable from off-host");
    assert!(err.contains("[::]:8080"), "got: {err}");
}

#[test]
fn routable_addresses_without_token_are_refused() {
    // Wildcard binds are the realistic case, but a specific routable address is
    // the same exposure and must not become the way around the guard.
    for a in ["10.0.0.5:8080", "192.168.1.10:8080", "172.16.0.1:8080"] {
        assert!(
            refusal(a, None, true).is_err(),
            "{a} is routable and has no token — must be refused"
        );
    }
    assert!(refusal("8.8.8.8:8080", None, true).is_err());
    assert!(refusal("[2001:db8::1]:8080", None, true).is_err());
}

#[test]
fn non_loopback_without_token_is_refused_regardless_of_allow_remote() {
    // Both values, because the two decisions were independent: neither the
    // opt-in nor its absence may be what makes an unauthenticated bind legal.
    for allow_remote in [false, true] {
        assert!(
            refusal("0.0.0.0:8080", None, allow_remote).is_err(),
            "allow_remote={allow_remote} must not decide auth"
        );
        assert!(refusal("[::]:8080", None, allow_remote).is_err());
        assert!(refusal("192.168.1.10:8080", None, allow_remote).is_err());
    }
}

// ── The intended remote posture must keep working ───────────────────────────

#[test]
fn non_loopback_with_token_and_remote_allowed_is_served() {
    // The operator who set BOTH a token and the opt-in meant to serve a remote
    // interface, authenticated. Coupling bind to auth must not forbid that.
    assert_eq!(
        refusal("0.0.0.0:8080", Some(TOKEN), true),
        Ok(Some(TOKEN.to_owned())),
        "token + explicit remote opt-in is the intended remote posture"
    );
    assert_eq!(
        refusal("[::]:8080", Some(TOKEN), true),
        Ok(Some(TOKEN.to_owned()))
    );
    assert_eq!(
        refusal("192.168.1.10:8080", Some(TOKEN), true),
        Ok(Some(TOKEN.to_owned()))
    );
}

#[test]
fn non_loopback_with_token_but_no_opt_in_is_refused() {
    // Defence in depth: `main.rs` already refuses this bind before it reaches
    // the auth decision. The function must not be the weaker of the two, or a
    // future caller that reaches it first would serve a non-loopback bind the
    // operator never opted into.
    assert!(refusal("0.0.0.0:8080", Some(TOKEN), false).is_err());
    assert!(refusal("[::]:8080", Some(TOKEN), false).is_err());
}

// ── Anti-vacuity: the inputs must mean what the tests assume ─────────────────

#[test]
fn the_fixtures_really_exercise_loopback_and_non_loopback() {
    // If `is_loopback` ever grew an overload that returned true for a wildcard,
    // every refusal above would pass for the wrong reason. Pin the premises.
    assert!(addr("127.0.0.1:8765").ip().is_loopback());
    assert!(addr("127.0.0.5:8765").ip().is_loopback());
    assert!(addr("[::1]:8765").ip().is_loopback());

    for a in [
        "0.0.0.0:8080",
        "[::]:8080",
        "10.0.0.5:8080",
        "192.168.1.10:8080",
    ] {
        assert!(
            !addr(a).ip().is_loopback(),
            "{a} must be a non-loopback fixture"
        );
    }
}

#[test]
fn the_refusal_is_a_distinction_not_a_blunt_refusal() {
    // A function that refused everything non-loopback would satisfy the cases
    // above while breaking the intended remote posture, and a function that
    // refused everything would break loopback. The verdict must track the
    // (loopback, token) pair exactly.
    let cases: [(&str, bool, bool); 4] = [
        ("127.0.0.1:8765", true, true),  // loopback + token
        ("127.0.0.1:8765", true, false), // loopback, no token
        ("0.0.0.0:8080", true, false),   // non-loopback, no token → refuse
        ("0.0.0.0:8080", true, true),    // non-loopback + token + opt-in
    ];
    let verdicts: Vec<bool> = cases
        .iter()
        .map(|(a, has_token, allow)| refusal(a, has_token.then_some(TOKEN), *allow).is_ok())
        .collect();

    assert_eq!(
        verdicts,
        vec![true, true, false, true],
        "the verdict must be: authenticated, loopback-posture, refused, authenticated"
    );
}
