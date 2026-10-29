//! III-2 — the bearer's persistent home.
//!
//! READ ORDER, in one place: login keychain (service `com.stewardmem.shell/token`,
//! account `default`) → the shell's OWN `BRAIN_SHELL_TOKEN` → absent. The
//! kernel's `AUTH_TOKEN`/`BRAIN_TOKEN`/`BRAIN_TOKEN_FILE` are NEVER read here:
//! those name the operator's kernel credential, and copying one into the
//! webview would cross the boundary this shell keeps.
//!
//! The store is injected as `&CredentialStore`; every pin drives the mock.
//! No test constructs the Apple store or touches the login keychain.

use keyring_core::{CredentialStore, Error as KeyringError};

pub(crate) const SERVICE: &str = "com.stewardmem.shell/token";
pub(crate) const ACCOUNT: &str = "default";
pub(crate) const MAX_TOKEN_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenError {
    Empty,
    TooLong,
    Unavailable,
    ReadFailed,
    WriteFailed,
    ClearFailed,
}

impl TokenError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Empty => "token_empty",
            Self::TooLong => "token_too_long",
            Self::Unavailable => "keychain_unavailable",
            Self::ReadFailed => "keychain_read_failed",
            Self::WriteFailed => "keychain_write_failed",
            Self::ClearFailed => "keychain_clear_failed",
        }
    }
}

#[cfg(target_os = "macos")]
fn platform_store() -> Result<std::sync::Arc<CredentialStore>, KeyringError> {
    let store = apple_native_keyring_store::keychain::Store::new()?;
    Ok(store)
}

#[cfg(not(target_os = "macos"))]
fn platform_store() -> Result<std::sync::Arc<CredentialStore>, KeyringError> {
    Err(KeyringError::NotSupportedByStore(
        "the login keychain is macOS-only".to_string(),
    ))
}

pub(crate) fn read_from(store: &CredentialStore) -> Result<Option<String>, TokenError> {
    let entry = store
        .build(SERVICE, ACCOUNT, None)
        .map_err(|_| TokenError::ReadFailed)?;
    match entry.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err(TokenError::ReadFailed),
    }
}

pub(crate) fn write_to(store: &CredentialStore, token: &str) -> Result<(), TokenError> {
    let entry = store
        .build(SERVICE, ACCOUNT, None)
        .map_err(|_| TokenError::WriteFailed)?;
    entry
        .set_password(token)
        .map_err(|_| TokenError::WriteFailed)
}

pub(crate) fn clear_from(store: &CredentialStore) -> Result<(), TokenError> {
    let entry = store
        .build(SERVICE, ACCOUNT, None)
        .map_err(|_| TokenError::ClearFailed)?;
    // Clearing an ABSENT item is success (nothing to clear); every other
    // failure is the caller's to see.
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(KeyringError::NoEntry) => Ok(()),
        Err(_) => Err(TokenError::ClearFailed),
    }
}

pub(crate) fn normalize(token: String) -> Result<String, TokenError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(TokenError::Empty);
    }
    if token.chars().count() > MAX_TOKEN_CHARS {
        return Err(TokenError::TooLong);
    }
    Ok(token.to_string())
}

/// keychain → env → absent. Both sources pass the SAME trim/bound; `Err` is a
/// fall-through, never a hard stop.
pub(crate) fn resolve(
    stored: Result<Option<String>, TokenError>,
    env: Option<String>,
) -> Option<String> {
    stored
        .ok()
        .flatten()
        .and_then(|t| normalize(t).ok())
        .or_else(|| env.and_then(|t| normalize(t).ok()))
}

pub(crate) fn read_platform() -> Result<Option<String>, TokenError> {
    let store = platform_store().map_err(|_| TokenError::Unavailable)?;
    read_from(store.as_ref())
}

pub(crate) fn write_platform(token: &str) -> Result<(), TokenError> {
    let store = platform_store().map_err(|_| TokenError::Unavailable)?;
    write_to(store.as_ref(), token)
}

pub(crate) fn clear_platform() -> Result<(), TokenError> {
    let store = platform_store().map_err(|_| TokenError::Unavailable)?;
    clear_from(store.as_ref())
}

pub(crate) fn resolve_platform(env: Option<String>) -> Option<String> {
    resolve(read_platform(), env)
}
