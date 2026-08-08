//! Cross-platform secure token storage — v1.16.6 M2.
//!
//! The ONE thing the client persists is the auth token. On every non-web
//! platform it lives in the OS keyring (macOS/iOS Keychain, Windows Credential
//! Manager, Linux Secret Service). On web it is a NO-OP — the token stays
//! in-memory only (v1.16.1 posture; the browser's localStorage is not a secure
//! credential store and would violate the MASVS-STORAGE row in the plan).
//!
//! `#[cfg(target_arch = "wasm32")]` is the discriminator, not `target_os`: the
//! web build is the only wasm target. Desktop + iOS + Android all compile the
//! keyring path.

/// The keyring entry identity. "brain-client" service + "auth-token" account is
/// a single well-known slot the connect flow reads/writes.
#[cfg(not(target_arch = "wasm32"))]
const SERVICE: &str = "brain-client";
#[cfg(not(target_arch = "wasm32"))]
const ACCOUNT: &str = "auth-token";

/// Persist the token to the OS keyring. Call ONLY with a real token (see
/// `should_persist`); saving nothing would clobber a previously-saved remote
/// token on a loopback connect.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_token(token: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| e.to_string())?
        .set_password(token)
        .map_err(|e| e.to_string())
}

/// Read the previously-saved token from the OS keyring. `None` when none was
/// ever saved (or the store is unavailable — a silent no-op, matching the
/// v1.16.1 "reconnect manually" posture).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_token() -> Option<String> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .ok()?
        .get_password()
        .ok()
}

/// Remove the saved token (logout). Best-effort — a missing entry is fine.
#[cfg(not(target_arch = "wasm32"))]
pub fn delete_token() {
    let _ = keyring::Entry::new(SERVICE, ACCOUNT).and_then(|e| e.delete_credential());
}

// Web: no-op. The token is in-memory only (the `Signal<ApiClient>`).
#[cfg(target_arch = "wasm32")]
pub fn save_token(_token: &str) -> Result<(), String> {
    Ok(())
}
#[cfg(target_arch = "wasm32")]
pub fn load_token() -> Option<String> {
    None
}
#[cfg(target_arch = "wasm32")]
pub fn delete_token() {}

/// v1.16.6 M2: persist the token ONLY when one was actually provided. A loopback
/// connect (empty token) must not overwrite a previously-saved remote token with
/// nothing. Extracted pure for the connect-flow gate + a unit test.
pub fn should_persist(token: Option<&str>) -> bool {
    token.map(|t| !t.trim().is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The persist gate: only a real token is saved (never clobber a prior one).
    #[test]
    fn persist_gate_requires_a_real_token() {
        assert!(!should_persist(None));
        assert!(!should_persist(Some("")));
        assert!(!should_persist(Some("   ")));
        assert!(should_persist(Some("opaque-token")));
        assert!(should_persist(Some("e30.e30.e30"))); // a JWT-shaped token
    }
}
