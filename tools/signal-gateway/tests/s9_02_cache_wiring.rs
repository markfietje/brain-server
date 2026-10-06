//! Cache wiring + edge egress laws (ninth-pass remediation band).
//!
//! **The gap these close.** The bounded recipient cache (`src/cache.rs`,
//! cap 4096, oldest-quarter eviction) shipped as DEAD CODE while the
//! production worker ran an unbounded inline `HashMap` that logged
//! `phone -> uuid` pairs at INFO — the remedy existed in-tree and the
//! production path didn't use it. These pins hold the WIRING, not the
//! cache's own behaviour (that lives in `cache.rs`'s test module), plus
//! the redirect law on the brain seam.
//!
//! Source-structure pins are text-bound by nature; each carries an
//! anti-vacuity arm (the file and the symbol must both be found) so a
//! renamed or moved file fails loudly instead of passing over nothing.

// Test-only: the crate denies panic vectors in PRODUCTION code
// (`Cargo.toml` `[lints.clippy]`); a pin that cannot fail loudly is not a pin.
#![allow(clippy::panic)]

use std::path::PathBuf;

fn src(rel: &str) -> String {
    let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()))
}

/// The production worker must consume `crate::cache::RecipientCache` and
/// define no cache struct of its own — the twin that rots back into an
/// inline unbounded map is exactly the regression this exists to catch.
#[test]
fn the_bounded_cache_is_the_production_cache() {
    let worker = src("src/signal/worker.rs");
    assert!(
        worker.contains("pub use crate::cache::RecipientCache;"),
        "worker.rs must route its cache through crate::cache — an inline \
         struct is the unbounded regression this law refuses"
    );
    assert!(
        !worker.contains("struct RecipientCache"),
        "worker.rs must not define its own RecipientCache; the bounded twin \
         in cache.rs is the one production type"
    );
    let cache = src("src/cache.rs");
    assert!(
        !cache.contains("#![allow(dead_code)]"),
        "cache.rs must not carry a module-level dead_code allow — the module \
         is production code now, and that allow is how it rotted dead while \
         an unbounded twin ran live"
    );
}

/// The named PII leaks stay dead: no `phone -> uuid` mapping line, no
/// self-ACI operand, on ANY log lane — and the seed verb is audited with
/// sha256 digests, not raw operands.
#[test]
fn cache_pii_operands_stay_off_the_log_lane() {
    for rel in [
        "src/signal/worker.rs",
        "src/cache.rs",
        "src/api/mod.rs",
        "src/brain.rs",
    ] {
        let s = src(rel);
        assert!(
            !s.contains("[CACHE] Mapping"),
            "{rel} reintroduced the phone->uuid mapping log line"
        );
        assert!(
            !s.contains("Self ACI: {}"),
            "{rel} reintroduced the self-ACI operand log line"
        );
    }
    let api = src("src/api/mod.rs");
    assert!(
        api.contains("phone_sha256") && api.contains("uuid_sha256"),
        "the seed verb must audit with digests (phone_sha256/uuid_sha256), \
         never raw operands — a wrong mapping sends messages to the wrong \
         identity, so the act has to be loud AND PII-lawful"
    );
}

/// The brain seam never rides a redirect (the channel-bridge egress law,
/// mirrored): signed HMAC headers re-sent cross-origin are a credential
/// leak the kernel's webhook verification cannot forgive.
#[test]
fn brain_client_refuses_redirects() {
    let brain = src("src/brain.rs");
    assert!(
        brain.contains("redirect::Policy::none()"),
        "BrainClient must build with redirect::Policy::none() — the signed \
         webhook headers must never ride a redirect cross-origin"
    );
}
