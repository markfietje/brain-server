//! III-2 — the mock-store pins for the bearer's persistent home.
//!
//! Every pin drives the REAL functions in `token.rs` against
//! `keyring_core::mock::Store`. No pin constructs the Apple store and none
//! touches the login keychain; the exhaustive walk at the bottom is the guard
//! that keeps it that way.

use std::sync::Arc;

use keyring_core::Error as KeyringError;
use keyring_core::api::CredentialStoreApi;
use keyring_core::mock;

use super::token::{self, ACCOUNT, MAX_TOKEN_CHARS, SERVICE, TokenError};

fn mock_store() -> Arc<mock::Store> {
    mock::Store::new().expect("the mock store builds")
}

/// Plant an error on the mock credential the store will hand out. The mock
/// clears it after one call, which is what lets the fall-through pin observe
/// both an error and the absence that follows it.
fn inject_error(store: &mock::Store, message: &str) {
    let entry = store
        .build(SERVICE, ACCOUNT, None)
        .expect("the mock store builds an entry");
    let credential = entry
        .as_any()
        .downcast_ref::<mock::Cred>()
        .expect("the mock store's credential downcasts");
    credential.set_error(KeyringError::PlatformFailure(Box::new(
        std::io::Error::other(message.to_string()),
    )));
}

#[test]
fn the_token_is_keychain_first_env_second_and_absent_last() {
    // Both sources present: the keychain wins.
    let store = mock_store();
    token::write_to(store.as_ref(), "keychain-token").expect("the write lands");
    assert_eq!(
        token::resolve(
            token::read_from(store.as_ref()),
            Some("env-token".to_string())
        ),
        Some("keychain-token".to_string())
    );

    // Empty keychain, env present: the env is the fallback.
    let empty = mock_store();
    assert_eq!(
        token::resolve(
            token::read_from(empty.as_ref()),
            Some("env-token".to_string())
        ),
        Some("env-token".to_string())
    );

    // Empty keychain, no env: absent.
    assert_eq!(token::resolve(token::read_from(empty.as_ref()), None), None);
}

#[test]
fn the_token_store_round_trips_and_clear_is_idempotent() {
    let store = mock_store();
    token::write_to(store.as_ref(), "round-trip-token").expect("the write lands");
    assert_eq!(
        token::read_from(store.as_ref()),
        Ok(Some("round-trip-token".to_string()))
    );

    token::clear_from(store.as_ref()).expect("the delete lands");
    assert_eq!(token::read_from(store.as_ref()), Ok(None));

    // Clearing an absent item is success, not an error.
    token::clear_from(store.as_ref()).expect("the second clear is idempotent");
}

#[test]
fn a_keychain_error_falls_through_to_the_environment() {
    let store = mock_store();
    token::write_to(store.as_ref(), "unreachable-stored-token").expect("the write lands");

    inject_error(&store, "locked keychain");
    assert_eq!(
        token::resolve(
            token::read_from(store.as_ref()),
            Some("env-token".to_string())
        ),
        Some("env-token".to_string())
    );

    // The mock cleared the error on the failed call; plant it again and show
    // that with no env the read is absent rather than an error surfaced.
    inject_error(&store, "locked keychain");
    assert_eq!(token::resolve(token::read_from(store.as_ref()), None), None);
}

#[test]
fn the_token_bounds_refuse_empty_and_oversized_values() {
    assert_eq!(
        token::normalize("  tok\n".to_string()),
        Ok("tok".to_string())
    );
    assert_eq!(
        token::normalize(" \n\t ".to_string()),
        Err(TokenError::Empty)
    );

    let at_max = "x".repeat(MAX_TOKEN_CHARS);
    assert_eq!(token::normalize(at_max.clone()), Ok(at_max));
    assert_eq!(
        token::normalize("x".repeat(MAX_TOKEN_CHARS + 1)),
        Err(TokenError::TooLong)
    );

    // A stored value that fails the bound does not beat the env.
    let store = mock_store();
    token::write_to(store.as_ref(), "   ").expect("a blank value can be written");
    assert_eq!(
        token::resolve(
            token::read_from(store.as_ref()),
            Some("env-token".to_string())
        ),
        Some("env-token".to_string())
    );
    token::write_to(store.as_ref(), &"x".repeat(MAX_TOKEN_CHARS + 1))
        .expect("an oversized value can be written");
    assert_eq!(
        token::resolve(
            token::read_from(store.as_ref()),
            Some("env-token".to_string())
        ),
        Some("env-token".to_string())
    );
    assert_eq!(
        token::resolve(
            token::read_from(store.as_ref()),
            Some("x".repeat(MAX_TOKEN_CHARS + 1))
        ),
        None
    );

    // An env value with surrounding whitespace is trimmed by the same rule.
    assert_eq!(
        token::resolve(Ok(None), Some("  env-token \n".to_string())),
        Some("env-token".to_string())
    );
}

#[test]
fn the_token_module_carries_no_log_or_print_macro() {
    let source = include_str!("token.rs");
    for needle in ["println!", "eprintln!", "dbg!", "log::", "tracing::"] {
        assert!(
            !source.contains(needle),
            "token.rs carries `{needle}` — the module has no sink and must not grow one"
        );
    }
}

#[test]
fn the_token_errors_carry_no_payload() {
    // Fieldless by CONSTRUCTION: a payloaded variant would make these
    // constructions fail to compile, which is the assertion.
    let variants = [
        (TokenError::Empty, "token_empty"),
        (TokenError::TooLong, "token_too_long"),
        (TokenError::Unavailable, "keychain_unavailable"),
        (TokenError::ReadFailed, "keychain_read_failed"),
        (TokenError::WriteFailed, "keychain_write_failed"),
        (TokenError::ClearFailed, "keychain_clear_failed"),
    ];
    for (variant, code) in variants {
        assert_eq!(variant.code(), code);
    }

    // A failed write must not echo the value it was asked to store.
    let store = mock_store();
    let sentinel = "brain-shell-token-error-sentinel-DO-NOT-LEAK-7c41";
    inject_error(&store, "locked keychain");
    let error =
        token::write_to(store.as_ref(), sentinel).expect_err("the planted error fails the write");
    let rendered = format!("{error:?}{}", error.code());
    assert!(
        !rendered.contains(sentinel),
        "the error rendered the token it failed to store"
    );
}

#[test]
fn the_test_suite_never_constructs_the_platform_keychain() {
    fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("the crate's src tree is readable") {
            let path = entry.expect("directory entries are readable").path();
            if path.is_dir() {
                walk(&path, files);
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_tests.rs"))
            {
                files.push(path);
            }
        }
    }

    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    assert!(
        files.len() >= 4,
        "the walk must see the test modules, saw {}",
        files.len()
    );

    // The needle is assembled at compile time so this test's own source does
    // not contain it: a scan that matches itself would red on every run.
    let needle = concat!("apple_native_keyring_store", "::keychain::Store::new");
    for path in &files {
        let text = std::fs::read_to_string(path).expect("a test module is readable");
        assert!(
            !text.contains(needle),
            "{} constructs the platform keychain — the tests drive the mock store only",
            path.display()
        );
    }
}
