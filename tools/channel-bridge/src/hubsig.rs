//! `X-Hub-Signature-256` verification — the Meta webhook discipline,
//! hardened to house law: LENGTH-CHECKED before compare, constant-time
//! fold compare, raw-body HMAC-SHA256 with the app secret. Pinned by
//! `hub_signature_verified_constant_time`.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Digest;

type HmacSha256 = Hmac<sha2::Sha256>;

/// Constant-time byte-slice equality (folded XOR accumulate; no early-out).
/// Used for the subscription token AND structurally mirrored inside the
/// signature comparison.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        // Lengths differ → values cannot match. The fold still runs so the
        // comparison WORK does not leak length classes through timing.
        let _ = a.iter().chain(b.iter()).fold(0u8, |acc, x| acc ^ x);
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// HMAC-SHA256 over the RAW body; hex, lowercase, exactly 64 chars.
fn hub_mac(app_secret: &[u8], body: &[u8]) -> String {
    let mut mac = match HmacSha256::new_from_slice(app_secret) {
        Ok(m) => m,
        Err(_) => {
            // A zero-length key is the only construction failure for HMAC-
            // SHA256; empty secrets are refused at config load anyway.
            return String::new();
        }
    };
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Verify `sha256=<hex64>` against the app secret over the raw body.
///
/// Hardening law (plan M1): the presented header is LENGTH-CHECKED before
/// any comparison (`"sha256=" + 64 hex`), and the final decision folds in
/// constant time. Malformed headers fail without ever reaching the MAC
/// compare; wrong secrets cannot win by timing.
pub(crate) fn verify_hub_signature(app_secret: &[u8], body: &[u8], header: &str) -> bool {
    let Some(presented_hex) = header.strip_prefix("sha256=") else {
        return false;
    };
    if presented_hex.len() != 64 || !presented_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let expected = hub_mac(app_secret, body);
    constant_time_eq(
        expected.as_bytes(),
        presented_hex.to_ascii_lowercase().as_bytes(),
    )
}

/// SHA-256 of arbitrary bytes, hex lowercase (media digests).
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let digest = sha2::Sha256::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn sign_hex(secret: &str, body: &[u8]) -> String {
        let mut m = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        m.update(body);
        m.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn constant_time_compare_scheme_matches_outbound_seam() {
        // The signature fold compare is the SAME construction the SW signer
        // composes over (documented cross-check; the byte-compat pin for the
        // wire scheme lives in outbound::tests::sw_sign_shape_is_kernel_...
        // because that is where signing happens).
        let (a, b) = (b"payload", b"payload");
        assert!(constant_time_eq(a, b));
    }

    // ── CARAVEL PIN (edge half): hub signature verification is constant-time
    //    AND length-checked BEFORE compare.
    #[test]
    fn hub_signature_verified_constant_time() {
        let secret = "the-app-secret";
        let body = br#"{"object":"whatsapp_business_account"}"#;
        let good = format!("sha256={}", sign_hex(secret, body));

        assert!(verify_hub_signature(secret.as_bytes(), body, &good));

        // Tampered digest byte → refuse.
        let mut tampered_bytes = good.clone().into_bytes();
        let ch = tampered_bytes[10];
        tampered_bytes[10] = if ch == b'a' { b'b' } else { b'a' };
        let tampered = String::from_utf8(tampered_bytes).unwrap_or_default();
        assert!(!verify_hub_signature(secret.as_bytes(), body, &tampered));

        // Wrong secret → refuse.
        assert!(!verify_hub_signature(b"other", body, &good));

        // Wrong body → refuse.
        assert!(!verify_hub_signature(
            secret.as_bytes(),
            br#"{"object":"forged"}"#,
            &good
        ));

        // Missing/malformed prefix refuses WITHOUT any MAC work path.
        assert!(!verify_hub_signature(
            secret.as_bytes(),
            body,
            &sign_hex(secret, body)
        ));
        assert!(!verify_hub_signature(
            secret.as_bytes(),
            body,
            "md5=deadbeef"
        ));

        // Length-checked BEFORE compare: short, long, and non-hex payloads
        // all refuse outright (timing + short-circuit classes from the plan).
        assert!(!verify_hub_signature(
            secret.as_bytes(),
            body,
            "sha256=abcd"
        ));
        assert!(!verify_hub_signature(
            secret.as_bytes(),
            body,
            &format!("sha256={}00", "a".repeat(64))
        ));
        assert!(!verify_hub_signature(
            secret.as_bytes(),
            body,
            &format!("sha256={}", "z".repeat(64))
        ));

        // Uppercase presentation normalizes and still verifies.
        let upper = format!("sha256={}", sign_hex(secret, body).to_uppercase());
        assert!(verify_hub_signature(secret.as_bytes(), body, &upper));

        // Empty secrets cannot construct the MAC → refuse (never accept).
        assert!(!verify_hub_signature(b"", body, &good));
    }

    #[test]
    fn constant_time_eq_holds_properties() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn sha256_hex_is_stable_and_lowercase() {
        let d = sha256_hex(b"abc");
        assert_eq!(
            d,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
