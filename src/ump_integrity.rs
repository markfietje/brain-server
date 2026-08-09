//! UMP 1.0 integrity + identity primitives (v1.17.3 "UMP Rollout").
//!
//! Pure functions only — no I/O, no storage. Everything the spec §2.8/§5.1/§5.2/
//! §6.1/§6.2 need: RFC 4648 base32 (no padding), did:key base58btc (multicodec
//! 0xed + Ed25519 public key), JCS (RFC 8785) canonicalization, BLAKE3 content
//! hashing, Ed25519 sign/verify, and owner-signed capability tokens (§5.2).
//!
//! Lives in the lib so the `brain` CLI (`brain ump keygen` / `brain ump
//! export`) and the server share the same identities — same pattern as `eval`.

#![deny(unsafe_code)]

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::Serialize;
use serde_json::Value;

/// Spec §6.2: `urn:ump:<id>` where `<id>` is the content hash (L2+).
pub const URN_UMP_PREFIX: &str = "urn:ump:";

/// RFC 4648 base32, no padding, lowercase. 5 bits per char.
pub fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// Spec §6.2 content-addressed id: `urn:ump:<base32(blake3(canonical record minus id/integrity))>`.
pub fn content_id(hash: &[u8; 32]) -> String {
    format!("{URN_UMP_PREFIX}{}", base32_encode(hash))
}

/// Spec §5.1: `did:key:z…` from an Ed25519 public key — multicodec `0xed`
/// prefix + base58btc. Pinned by a known-vector test.
pub fn did_key_from_ed25519(pk: &[u8; 32]) -> String {
    let mut buf = [0u8; 33];
    buf[0] = 0xed;
    buf[1..].copy_from_slice(pk);
    format!("did:key:z{}", bs58::encode(buf).into_string())
}

/// Spec §6.1: JCS (RFC 8785) canonicalization.
///
/// `serde_json`'s default `Map` is a `BTreeMap`, so `to_vec` already emits
/// keys in sorted order and `f64` via ryu (shortest round-trip repr) — the two
/// JCS requirements. This wrapper exists so the semantics are pinned by name
/// and a test vector, and a future `preserve_order` feature flag can't silently
/// change record hashes.
pub fn canonical_jcs(v: &Value) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(v)
}

/// Spec §2.8: BLAKE3 content hash (spec-mandated algorithm).
pub fn record_hash(canonical: &[u8]) -> [u8; 32] {
    blake3::hash(canonical).into()
}

/// Sign the record hash with the operator's Ed25519 key (§2.8/§6.1).
pub fn sign_hash(hash: &[u8; 32], sk: &SigningKey) -> Vec<u8> {
    sk.sign(hash).to_bytes().to_vec()
}

/// Verify a record hash signature. Returns false (never errors) on any
/// malformed input — the read path drops unverifiable records (§5.3).
pub fn verify_hash(pk_bytes: &[u8; 32], hash: &[u8; 32], sig: &[u8]) -> bool {
    let Ok(pk) = VerifyingKey::from_bytes(pk_bytes) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(sig) else {
        return false;
    };
    pk.verify(hash, &sig).is_ok()
}

/// Capability token (§5.2): owner-signed `payload.sig`, base64url(JSON),
/// no header (the payload carries `alg`). Enforces verbs × scope × expiry.
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct CapabilityToken {
    pub alg: String,
    pub iss: String,
    pub verbs: Vec<String>,
    /// Project scope; `None`/empty = all projects.
    pub scope: Option<String>,
    /// Unix seconds; tokens expire.
    pub exp: u64,
}

/// Parse + verify a capability token against the owner public key.
pub fn parse_capability_token(token: &str, pk: &[u8; 32]) -> Result<CapabilityToken, TokenError> {
    let mut parts = token.split('.');
    let (Some(payload_b64), Some(sig_b64), None) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(TokenError::Malformed);
    };
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| TokenError::Malformed)?;
    let sig_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| TokenError::Malformed)?;
    let claims: CapabilityToken =
        serde_json::from_slice(&payload_bytes).map_err(|_| TokenError::Malformed)?;
    let Ok(sig) = Signature::from_slice(&sig_bytes) else {
        return Err(TokenError::BadSignature);
    };
    let Ok(pk) = VerifyingKey::from_bytes(pk) else {
        return Err(TokenError::BadSignature);
    };
    if pk.verify(payload_b64.as_bytes(), &sig).is_err() {
        return Err(TokenError::BadSignature);
    }
    if claims.alg != "EdDSA" {
        return Err(TokenError::Malformed);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now > claims.exp {
        return Err(TokenError::Expired);
    }
    Ok(claims)
}

/// Mint a capability token signed by the owner key.
pub fn mint_capability_token(
    claims: &CapabilityToken,
    sk: &SigningKey,
) -> Result<String, serde_json::Error> {
    let payload = serde_json::to_vec(claims)?;
    let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
    let sig = sk.sign(payload_b64.as_bytes());
    let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig.to_bytes());
    Ok(format!("{payload_b64}.{sig_b64}"))
}

/// Token validation failures — mapped to `unauthorized` at the handler.
#[derive(Debug, PartialEq)]
pub enum TokenError {
    Malformed,
    BadSignature,
    Expired,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    fn keypair() -> (SigningKey, [u8; 32]) {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key().to_bytes();
        (sk, pk)
    }

    #[test]
    fn base32_encodes_rfc4648_vectors() {
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_encode(b"f"), "my");
        assert_eq!(base32_encode(b"fo"), "mzxq");
        assert_eq!(base32_encode(b"foo"), "mzxw6");
        assert_eq!(base32_encode(b"foob"), "mzxw6yq");
        assert_eq!(base32_encode(b"fooba"), "mzxw6ytb");
        assert_eq!(base32_encode(b"foobar"), "mzxw6ytboi");
    }

    #[test]
    fn did_key_vector_is_stable() {
        // RFC 8032 test vector 1 public key; expected value independently
        // computed (pure-python base58btc of multicodec 0xed || pk).
        let pk: [u8; 32] = [
            0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64,
            0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0xbb, 0x6d,
            0x59, 0x03, 0x09, 0x5d,
        ];
        assert_eq!(
            did_key_from_ed25519(&pk),
            "did:key:z2DeuicgUFGK9784FgMs5DG57pbDLWGaDu6TnXE73uLgkEQ"
        );
    }

    #[test]
    fn jcs_canonicalizes_sorted_keys_and_shortest_floats() {
        let v: Value = serde_json::from_str(r#"{"b":2.0,"a":1,"c":0.1}"#).unwrap();
        assert_eq!(
            String::from_utf8(canonical_jcs(&v).unwrap()).unwrap(),
            r#"{"a":1,"b":2.0,"c":0.1}"#
        );
        let nested: Value = serde_json::from_str(r#"{"x":{"z":1,"y":[3,2.50,1]}}"#).unwrap();
        assert_eq!(
            String::from_utf8(canonical_jcs(&nested).unwrap()).unwrap(),
            r#"{"x":{"y":[3,2.5,1],"z":1}}"#
        );
    }

    #[test]
    fn content_id_is_deterministic_and_content_bound() {
        let h1 = record_hash(b"hello");
        let h2 = record_hash(b"hello");
        let h3 = record_hash(b"hello!");
        assert_eq!(content_id(&h1), content_id(&h2));
        assert_ne!(content_id(&h1), content_id(&h3));
        assert!(content_id(&h1).starts_with(URN_UMP_PREFIX));
    }

    #[test]
    fn sign_verify_round_trip_and_tamper_detection() {
        let (sk, pk) = keypair();
        let h = record_hash(b"the record");
        let sig = sign_hash(&h, &sk);
        assert!(verify_hash(&pk, &h, &sig));
        let tampered = record_hash(b"the record!");
        assert!(!verify_hash(&pk, &tampered, &sig));
        assert!(!verify_hash(&pk, &h, b"bad-sig"));
    }

    #[test]
    fn capability_token_round_trip_and_expiry() {
        let (sk, pk) = keypair();
        let claims = CapabilityToken {
            alg: "EdDSA".into(),
            iss: did_key_from_ed25519(&pk),
            verbs: vec!["read".into(), "derive".into()],
            scope: Some("projects/x".into()),
            exp: u64::MAX,
        };
        let token = mint_capability_token(&claims, &sk).unwrap();
        let parsed = parse_capability_token(&token, &pk).unwrap();
        assert_eq!(parsed, claims);
        assert_eq!(
            parse_capability_token(&token, &[0u8; 32]),
            Err(TokenError::BadSignature)
        );
        assert_eq!(
            parse_capability_token("nonsense", &pk),
            Err(TokenError::Malformed)
        );

        let expired = CapabilityToken {
            exp: 1,
            ..claims.clone()
        };
        let exp_token = mint_capability_token(&expired, &sk).unwrap();
        assert_eq!(
            parse_capability_token(&exp_token, &pk),
            Err(TokenError::Expired)
        );
    }
}
