//! III-4 — the updater's pins: **where the bytes come from, and whose they are**.
//!
//! Every pin here drives the REAL function in `updater.rs`, and the real
//! `tauri-plugin-updater` behind it:
//!
//!   * the gate pins drive the pure `decide()` — no app, no network;
//!   * the signature pin drives the PLUGIN'S OWN minisign verification
//!     (`Update::verify`) inside a mock Tauri app, against the key the app
//!     actually ships, with a REAL signature over REAL bytes.
//!
//! The signature fixtures below are PUBLIC halves only: a minisign public key
//! and a minisign signature. No private key is in this tree, and none is
//! needed to check one — the fixtures were produced OUTSIDE the repository by
//! `tauri signer generate` + `tauri signer sign --app-version`, and only the
//! public halves were carried in.
//!
//! Nothing here starts the real kernel, binds the real port, reads the
//! operator's data or touches the login keychain.

use super::updater::{self, Decision, ENDPOINT_ENV};
use tauri_plugin_updater::UpdaterExt;

/// The bytes the fixtures sign. A signature is over BYTES, so the pin needs
/// the exact bytes — and it has them, because they are a constant here.
const PAYLOAD: &[u8] = b"brain-shell updater signature fixture\n";

/// The SECOND keypair — the one an attacker would sign with. Its PUBLIC half
/// and its signature are fixtures; its private half never existed in this
/// tree.
const SECOND_KEY_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDdDMjJEMzA5OTZGNTkzMjkKUldRcGsvV1dDZE1pZk1LSUZEcS9uT0pvOGtJcUJTc1ozNDdhL0FVaXp5dkloSGlBZjhEYlZaQzIK";

/// A REAL signature over `PAYLOAD`, made by the second keypair, bound to the
/// version `0.1.2` in its trusted comment.
const SECOND_KEY_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRcGsvV1dDZE1pZkhFZVlQUytFRjZXTUozT3BpTU1sRjg0OXNpSGhDMDRKY3ZIamVBaWdlQW9pWXZ2SVBqNjJGNkhTRjlTSVVyVEZpUDNhM2lJdTRwcE9RVmpteWxjNXdJPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyNDc3NTEzCWZpbGU6cGF5bG9hZC13cm9uZy5iaW4JdmVyc2lvbjowLjEuMgp4c2d3RWxUbUpuMGRBZVE3TG11Z2pJSDBRVHl0eHdINVlkTHRscFNWSi9nNjZza3dqVGR1d1YrUUxCb0Rld1pPcE1kMHgrcEhFc3JmKzF6d0V5N05Ddz09Cg==";

/// The manifest the endpoint would answer with: one release, offered for
/// every target the plugin might ask for on any CI platform, announcing
/// `version`.
///
/// The announced version and the SIGNED version are two facts, and the
/// plugin's `require_signed_version` law is the comparison between them — so
/// a fixture that cannot announce a different version cannot exercise it.
fn manifest_versioned(signature: &str, version: &str) -> serde_json::Value {
    let platform = serde_json::json!({
        "url": "https://example.invalid/Brain%20Shell.app.tar.gz",
        "signature": signature,
    });
    let mut platforms = serde_json::Map::new();
    for target in [
        "darwin-aarch64",
        "darwin-x86_64",
        "linux-x86_64",
        "linux-aarch64",
    ] {
        platforms.insert(target.to_string(), platform.clone());
    }
    serde_json::json!({
        "version": version,
        "notes": "a fixture release",
        "pub_date": "2026-10-20T00:00:00Z",
        "platforms": platforms,
    })
}

/// The app's own configuration, as the tree ships it.
fn shipped_config() -> serde_json::Value {
    serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json parses")
}

/// The public key the app SHIPS, read from the app's own configuration.
///
/// The endpoint may be overridden at runtime; the key may not. So this is
/// read from the configuration and from nowhere else — and the pin that
/// proves there is no other path is `the_updater_module_reads_exactly_one_
/// environment_variable_and_it_is_the_endpoint`.
fn shipped_pubkey() -> String {
    let config = shipped_config();
    let plugin = config
        .get("plugins")
        .and_then(|value| value.get("updater"))
        .unwrap_or_else(|| {
            panic!(
                "the app's configuration carries no plugins.updater block (the key is read from \
                 the configuration and from nowhere else)"
            )
        });
    updater::pinned_pubkey(plugin).unwrap_or_else(|| {
        panic!(
            "the app's configuration carries no updater public key (the key is read from the \
             configuration and from nowhere else)"
        )
    })
}

/// A mock Tauri app with the updater plugin installed and the given public
/// key, and an `Update` restored from the manifest — WITHOUT a network
/// request (`Updater::restore_update`).
fn restore(pubkey: &str, signature: &str) -> tauri_plugin_updater::Update {
    restore_versioned(pubkey, signature, "0.1.2")
}

/// The same restore, with a manifest that announces a different version.
fn restore_versioned(pubkey: &str, signature: &str, version: &str) -> tauri_plugin_updater::Update {
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().plugins.0.insert(
        "updater".to_string(),
        serde_json::json!({
            "pubkey": pubkey,
            "endpoints": ["https://example.invalid/latest.json"],
        }),
    );
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .expect("the mock app builds with the updater plugin");
    let updater = app
        .handle()
        .updater_builder()
        .version_comparator(|_, _| true)
        .build()
        .expect("the updater builds");
    updater
        .restore_update(manifest_versioned(signature, version))
        .expect("the manifest restores")
        .expect("the manifest offers an update")
}

// ── P1 · the signature ─────────────────────────────────────────────────────

/// P1 — an update signed by a key that is NOT the key the app pins is
/// refused.
///
/// The mutation this catches is the one that matters: swapping the pinned
/// public key (in the app's configuration) for another key's. That swap
/// changes WHO the app trusts, and it is invisible in the manifest — the
/// manifest is unsigned. So the pin verifies the second keypair's REAL
/// signature against the key the app SHIPS, and refuses it.
///
/// The anti-vacuity arm matters as much as the refusal: the same signature
/// verifies under the key that MADE it. Without that arm, a fixture that was
/// simply broken would satisfy the pin.
#[test]
fn an_update_signed_by_a_key_that_is_not_the_pinned_key_is_refused() {
    let pinned = shipped_pubkey();

    let update = restore(&pinned, SECOND_KEY_SIGNATURE);
    let refused = updater::verify_artifact(&update, PAYLOAD);
    assert!(
        refused.is_err(),
        "an update signed by a key that is not the pinned key must be refused, got {refused:?}"
    );

    // Anti-vacuity: the fixture is a REAL signature; only the key differs.
    let update = restore(SECOND_KEY_PUBKEY, SECOND_KEY_SIGNATURE);
    assert_eq!(
        updater::verify_artifact(&update, PAYLOAD),
        Ok(()),
        "the second key's own signature must verify under the second key, or the refusal above \
         proves nothing"
    );
}

// ── P2 · the kernel the release does not know ─────────────────────────────

/// P2 — a build whose OWN kernel version is unknown to the release refuses
/// to update, and the refusal names BOTH versions.
///
/// The update replaces the whole `.app`, including the bundled kernel
/// sidecar. A release that does not vouch for the kernel this app carries
/// must not be installed over it, and the refusal must say which kernel was
/// expected and which was found.
#[test]
fn a_build_whose_embedded_kernel_version_is_unknown_to_the_release_refuses_to_update() {
    let manifest = serde_json::json!({
        "version": "0.1.1",
        "kernel_version": "1.29.23",
        "kernel_versions": ["1.29.23"],
    });

    match updater::decide(Some("1.29.22"), &manifest) {
        Decision::Refused {
            kind,
            expected,
            observed,
        } => {
            assert_eq!(kind, "unknown_local_kernel", "the refusal names its law");
            assert_eq!(
                observed.as_deref(),
                Some("1.29.22"),
                "the refusal names the kernel this app carries"
            );
            let expected =
                expected.expect("the refusal names the versions the release vouches for");
            assert!(
                expected.contains("1.29.23"),
                "the refusal names the vouched kernel version, got {expected:?}"
            );
        }
        other => panic!(
            "a build whose kernel version the release does not know must be refused, got {other:?}"
        ),
    }
}

// ── P3 · a manifest that says nothing about the kernel ────────────────────

/// P3 — a manifest without a kernel version is refused, fail-closed.
///
/// Two shapes, and an empty list is the same refusal as an absent one: a
/// release that does not say which kernel it carries does not get to
/// replace the one this app carries.
#[test]
fn a_manifest_without_a_kernel_version_is_refused() {
    let no_kernel_version = serde_json::json!({
        "version": "0.1.1",
        "kernel_versions": ["1.29.23"],
    });
    match updater::decide(Some("1.29.23"), &no_kernel_version) {
        Decision::Refused { kind, .. } => assert_eq!(kind, "no_kernel_version"),
        other => panic!("a manifest without a kernel version must be refused, got {other:?}"),
    }

    let no_known_versions = serde_json::json!({
        "version": "0.1.1",
        "kernel_version": "1.29.23",
    });
    match updater::decide(Some("1.29.23"), &no_known_versions) {
        Decision::Refused { kind, .. } => assert_eq!(kind, "no_known_kernel_versions"),
        other => panic!(
            "a manifest without the versions a release vouches for must be refused, got {other:?}"
        ),
    }

    let empty_known_versions = serde_json::json!({
        "version": "0.1.1",
        "kernel_version": "1.29.23",
        "kernel_versions": [],
    });
    match updater::decide(Some("1.29.23"), &empty_known_versions) {
        Decision::Refused { kind, .. } => assert_eq!(kind, "no_known_kernel_versions"),
        other => panic!("an empty list of vouched versions must be refused, got {other:?}"),
    }
}

// ── III-7 P6 · the reader refuses what the writer would never produce ────

/// III-7 P6 — the updater REFUSES a manifest it cannot verify.
///
/// The round that built the updater proved that a signature from another key
/// is refused. This pin closes the loop from the OTHER end: the reader must
/// refuse the three things the writer would never produce, and each refusal
/// must be a refusal rather than an "unreachable" — an endpoint that cannot
/// be reached and an artefact that cannot be verified are different faults,
/// and the app says which.
///
/// The arms:
///
/// 1. **NO signature.** The manifest names an artefact and carries no proof
///    of who produced it. There is nothing to verify against.
/// 2. **A signature from ANOTHER key.** The bytes are fine; the PROOF is
///    not ours.
/// 3. **A version the app does not carry.** The signature is genuinely the
///    pinned key's, and the manifest announces a version the signature was
///    not made for — the relabel, caught at the reader. The refusal names
///    BOTH versions, because a refusal that names one of them leaves the
///    reader to guess about the other.
#[test]
fn the_updater_refuses_a_manifest_it_cannot_verify() {
    let pinned = shipped_pubkey();

    // Arm 1: no signature at all.
    let unsigned = restore_versioned(&pinned, "", "0.1.2");
    let refused = updater::verify_artifact(&unsigned, PAYLOAD);
    assert!(
        refused.is_err(),
        "a manifest with no signature must be refused, got {refused:?}"
    );

    // Arm 2: a signature from another key.
    let other_key = restore_versioned(&pinned, SECOND_KEY_SIGNATURE, "0.1.2");
    let refused = updater::verify_artifact(&other_key, PAYLOAD);
    assert!(
        refused.is_err(),
        "a signature from another key must be refused, got {refused:?}"
    );

    // Arm 3: a version the app does not carry. The signature is REAL and
    // verifies under the key that made it, so the ONLY thing left to fail
    // is the version binding — which is exactly what this arm is about.
    let relabelled = restore_versioned(SECOND_KEY_PUBKEY, SECOND_KEY_SIGNATURE, "0.1.3");
    let error = updater::verify_artifact(&relabelled, PAYLOAD)
        .expect_err("a version the app does not carry must be refused");
    assert!(
        error.contains("0.1.2") && error.contains("0.1.3"),
        "the refusal must name BOTH versions — the signed one and the announced one — got {error:?}"
    );

    // Anti-vacuity: the fixture really is a good signature under the key
    // that made it, and the good case really does verify. Without this arm
    // a broken fixture would satisfy all three refusals above.
    let honest = restore_versioned(SECOND_KEY_PUBKEY, SECOND_KEY_SIGNATURE, "0.1.2");
    assert_eq!(
        updater::verify_artifact(&honest, PAYLOAD),
        Ok(()),
        "the second key's own signature must verify under the second key, or the refusals above \
         prove nothing"
    );
}

// ── P10 · the module's environment ────────────────────────────────────────

/// P10 — the updater module reads EXACTLY ONE environment variable, and it
/// is the endpoint override.
///
/// The exhaustive enumeration is the point: not "there is no env read of the
/// public key" but "there is exactly one env read, and here it is". A key
/// that could be overridden by an environment variable would move WHO the
/// app trusts, and that is the one thing this round's design forbids.
#[test]
fn the_updater_module_reads_exactly_one_environment_variable_and_it_is_the_endpoint() {
    let source = include_str!("updater.rs");

    let mut reads: Vec<String> = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("env::var") {
        let after = rest[at + "env::var".len()..].trim_start();
        let (call, tail) = match after.strip_prefix("_os(") {
            Some(tail) => ("env::var_os", tail),
            None => match after.strip_prefix('(') {
                Some(tail) => ("env::var", tail),
                None => panic!("the module reads the environment in a form the scan cannot parse"),
            },
        };
        let close = tail.find(')').expect("a closed environment read");
        reads.push(format!("{call}({})", tail[..close].trim()));
        rest = &tail[close..];
    }

    assert_eq!(
        reads.len(),
        1,
        "the updater module must read exactly one environment variable, it reads {reads:?}"
    );
    assert_eq!(
        reads[0], "env::var(ENDPOINT_ENV)",
        "the one environment read is the endpoint override"
    );
    assert_eq!(
        ENDPOINT_ENV, "BRAIN_SHELL_UPDATE_ENDPOINT",
        "the endpoint override has one name"
    );
}
