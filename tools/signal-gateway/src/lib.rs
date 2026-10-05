//! Signal Gateway — the decision seams that govern how the process is exposed.
//!
//! The rest of the crate (API, Signal edge, brain adapter) is a binary-only tree
//! declared from `main.rs`. This library target exists for the postures that
//! must be provable from a test rather than read off a log line: what the
//! process is allowed to serve, and on which interface.
//!
//! Memory safety: the package forbids `unsafe_code` crate-wide (`Cargo.toml`
//! `[lints.rust]`), so this target is covered by the same compile-time bar
//! `main.rs` carries.

#![forbid(unsafe_code)]

/// Whether the operator explicitly opted into serving a non-loopback interface.
///
/// The env read lives here so the bind guard and the auth posture cannot drift
/// apart: both consume the value this function returns, read once per process.
pub fn remote_bind_allowed() -> bool {
    std::env::var("SIGNAL_GATEWAY_ALLOW_REMOTE").as_deref() == Ok("1")
}

/// The auth posture this process may serve on `addr`.
///
/// Returns the token to install on the router (`Ok(Some(token))`), no auth
/// (`Ok(None)`), or a refusal (`Err`). The caller must not serve anything, and
/// must not open a socket, until this returns `Ok`.
///
/// An empty `server.auth_token` counts as no credential: it would install an
/// empty secret, and any client could then satisfy it with a bare
/// `Authorization: Bearer ` header.
///
/// Coupling: the interface and the credential are ONE decision. `ALLOW_REMOTE`
/// is an operator's statement about *reaching* the process, never a waiver of
/// its authentication — so a non-loopback bind demands a token, and the
/// loopback posture is the only one that may run unauthenticated.
///
/// `main.rs` keeps its own bind guard, which refuses a non-loopback bind
/// without the opt-in before this is ever consulted. This function is the
/// second, independent half: it refuses to serve *unauthenticated* on anything
/// routable. A caller that reached only one of the two still cannot open the
/// exposure.
pub fn resolve_api_auth(
    addr: std::net::SocketAddr,
    token: Option<String>,
    allow_remote: bool,
) -> Result<Option<String>, String> {
    if addr.ip().is_loopback() {
        return Ok(token.filter(|t| !t.is_empty()));
    }

    // Non-loopback from here. The opt-in is required to bind at all; the
    // credential is required to serve. Both, or nothing is served.
    if !allow_remote {
        return Err(format!(
            "{addr} is not loopback and SIGNAL_GATEWAY_ALLOW_REMOTE=1 is not set"
        ));
    }
    match token.filter(|t| !t.is_empty()) {
        Some(t) => Ok(Some(t)),
        None => Err(format!(
            "{addr} is not loopback, so the API cannot be served unauthenticated: \
             set server.auth_token in the config, or bind a loopback address"
        )),
    }
}

#[cfg(test)]
mod tests {
    // Test-only: the crate denies panic vectors in PRODUCTION code
    // (`Cargo.toml` `[lints.clippy]`). A test that cannot fail loudly is not a
    // test. Mirrors the same scoping used by the other test modules here.
    #![allow(clippy::expect_used, clippy::panic)]
    use super::*;

    const TOKEN: &str = "an-operator-chosen-token";

    fn verdict(a: &str, token: Option<&str>, allow_remote: bool) -> Result<Option<String>, String> {
        match a.parse() {
            Ok(addr) => resolve_api_auth(addr, token.map(str::to_owned), allow_remote),
            Err(e) => panic!("fixture {a} must parse: {e}"),
        }
    }

    #[test]
    fn loopback_serves_with_or_without_a_token() {
        assert_eq!(
            verdict("127.0.0.1:8765", Some(TOKEN), false),
            Ok(Some(TOKEN.to_owned()))
        );
        assert_eq!(verdict("127.0.0.1:8765", None, false), Ok(None));
        assert_eq!(verdict("[::1]:8765", None, true), Ok(None));
    }

    #[test]
    fn non_loopback_without_a_token_is_refused_however_it_was_reached() {
        for allow_remote in [false, true] {
            assert!(
                verdict("0.0.0.0:8080", None, allow_remote).is_err(),
                "allow_remote={allow_remote} must not decide auth"
            );
            assert!(verdict("[::]:8080", None, allow_remote).is_err());
        }
    }

    #[test]
    fn non_loopback_with_a_token_serves_only_under_the_opt_in() {
        assert_eq!(
            verdict("0.0.0.0:8080", Some(TOKEN), true),
            Ok(Some(TOKEN.to_owned()))
        );
        assert!(verdict("0.0.0.0:8080", Some(TOKEN), false).is_err());
    }

    #[test]
    fn an_empty_token_is_not_a_credential() {
        // A configured-but-empty `auth_token` would otherwise install an empty
        // secret, and `Authorization: Bearer ` would then satisfy it.
        assert!(
            verdict("0.0.0.0:8080", Some(""), true).is_err(),
            "an empty token must not pass as a credential on a routable bind"
        );
    }

    #[test]
    fn the_refusal_never_echoes_the_token() {
        let secret = "s3cret-value";
        let err = match verdict("0.0.0.0:8080", Some(secret), false) {
            Ok(_) => panic!("a non-loopback bind without the opt-in must be refused"),
            Err(e) => e,
        };
        assert!(
            !err.contains(secret),
            "a refusal must not carry the credential: {err}"
        );
    }
}
