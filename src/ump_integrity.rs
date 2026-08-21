//! UMP 1.0 integrity + identity primitives.
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
    let mut acc = 0u32;
    let mut bits = 0u32;
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

/// Spec §5.1: `did:key:z…` from an Ed25519 public key — the `0xed 0x01`
/// multicodec varint (Ed25519 pubkey) + base58btc. The varint is TWO bytes:
/// the reference `didKeyFromPublicKey` prefixes `[0xed, 0x01]` and
/// `publicKeyFromDidKey` rejects any other codec (a bare `0xed` 33-byte form
/// yields a valid base58 string that is NOT a did:key). Pinned by a
/// known-vector test computed against the reference base58btc.
pub fn did_key_from_ed25519(pk: &[u8; 32]) -> String {
    let mut buf = [0u8; 34];
    buf[0] = 0xed;
    buf[1] = 0x01;
    buf[2..].copy_from_slice(pk);
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

/// RFC 8785 canonicalization with the reference implementation's exact
/// number/string rules (the suite's `canonicalize`, `canonical.ts`): integral
/// floats serialize without a trailing `.0` (JS `String(n)` — `serde_json`/
/// ryu would emit `1.0`), strings escape U+2028/U+2029, and null is kept.
/// Keys sort by UTF-16 code unit, identical to byte order for the ASCII keys
/// UMP records carry. This is the ONLY flavor that reproduces the reference
/// `contentHash`, so `emit_record`/`verify_record` (and any peer-verified
/// signing) must use it, not `canonical_jcs`.
/// `ponytail:` number formatting follows ECMAScript `Number.toString` for the
/// range UMP records carry (small integers + simple decimals): Rust's shortest
/// round-trip display matches JS except at extreme magnitudes (|n| >= 1e21,
/// or < 1e-6 where JS switches to exponent notation) and for -0.0 — none of
/// which a stored record can produce.
pub fn canonical_ump(v: &Value) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    write_ump_value(&mut out, v)?;
    Ok(out)
}

fn write_ump_value(out: &mut Vec<u8>, v: &Value) -> Result<(), String> {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => {
            let f = n
                .as_f64()
                .ok_or_else(|| "canonical_ump: non-finite number".to_string())?;
            if !f.is_finite() {
                return Err("canonical_ump: non-finite number".into());
            }
            out.extend_from_slice(format!("{f}").as_bytes());
        }
        Value::String(s) => write_ump_string(out, s),
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_ump_value(out, item)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            out.push(b'{');
            // serde_json's default Map is a BTreeMap — keys already sorted
            // (byte order = UTF-16 order for the ASCII keys records carry).
            for (i, (k, val)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_ump_string(out, k);
                out.push(b':');
                write_ump_value(out, val)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

/// JS `JSON.stringify` string escaping: the JSON short forms, `\u00XX` for
/// other control chars, and the ES2019 U+2028/U+2029 escapes (serde_json
/// leaves those two raw — byte-exactness with the reference requires them).
fn write_ump_string(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\u{0c}' => out.extend_from_slice(b"\\f"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\u{2028}' => out.extend_from_slice(b"\\u2028"),
            '\u{2029}' => out.extend_from_slice(b"\\u2029"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

/// Spec §2.8: BLAKE3 content hash (spec-mandated algorithm).
pub fn record_hash(canonical: &[u8]) -> [u8; 32] {
    blake3::hash(canonical).into()
}

/// The reference `contentHash` string: `blake3:` + lowercase base32 (no
/// padding) over the canonical record bytes — the exact string a record's
/// `integrity.content_hash` must equal and what the signature signs.
pub fn content_hash_string(canonical: &[u8]) -> String {
    format!("blake3:{}", base32_encode(&record_hash(canonical)))
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

/// Reference `signHash`: Ed25519 over BLAKE3(hash-string) — the signed message
/// is the digest of the `blake3:…` content-hash STRING, not the raw record
/// hash. The suite's `verify()` recomputes exactly this.
pub fn sign_hash_string(hash_string: &str, sk: &SigningKey) -> Vec<u8> {
    sk.sign(&record_hash(hash_string.as_bytes()))
        .to_bytes()
        .to_vec()
}

/// Verify a `sign_hash_string` signature (reference `verifyHash`). False on
/// any malformed input.
pub fn verify_hash_string(hash_string: &str, pk_bytes: &[u8; 32], sig: &[u8]) -> bool {
    verify_hash(pk_bytes, &record_hash(hash_string.as_bytes()), sig)
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
        // computed (pure-python base58btc of the 34-byte multicodec
        // 0xed 0x01 || pk — the reference `didKeyFromPublicKey` form).
        let pk: [u8; 32] = [
            0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64,
            0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0xbb, 0x6d,
            0x59, 0x03, 0x09, 0x5d,
        ];
        assert_eq!(
            did_key_from_ed25519(&pk),
            "did:key:z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN5x1fTDDgQ"
        );
    }

    #[test]
    fn canonical_ump_matches_reference_js_flavor() {
        // JS String(2.0) = "2", not "2.0" — the byte that makes the suite's
        // contentHash agree with ours.
        let v: Value = serde_json::from_str(r#"{"b":2.0,"a":1,"c":0.1}"#).unwrap();
        assert_eq!(
            String::from_utf8(canonical_ump(&v).unwrap()).unwrap(),
            r#"{"a":1,"b":2,"c":0.1}"#
        );
        // Nulls survive; keys stay sorted; strings escape like JSON.stringify
        // (including U+2028/U+2029, which serde_json leaves raw).
        let v: Value =
            serde_json::from_str(r#"{"x":{"z":null,"y":[3,2.50,1]},"s":"a\u2028b\tc"}"#).unwrap();
        assert_eq!(
            String::from_utf8(canonical_ump(&v).unwrap()).unwrap(),
            r#"{"s":"a\u2028b\tc","x":{"y":[3,2.5,1],"z":null}}"#
        );
        // Integral f64s emitted by the record engine (confidence 1.0) collide
        // byte-identically with the JS canonicalizer.
        let v: Value = serde_json::from_str(r#"{"confidence":1.0,"kind":"semantic"}"#).unwrap();
        assert_eq!(
            String::from_utf8(canonical_ump(&v).unwrap()).unwrap(),
            r#"{"confidence":1,"kind":"semantic"}"#
        );
    }

    #[test]
    fn content_hash_string_is_reference_shaped() {
        let h = content_hash_string(b"{\"a\":1}");
        assert!(h.starts_with("blake3:"));
        assert_eq!(
            h,
            format!("blake3:{}", base32_encode(&record_hash(b"{\"a\":1}")))
        );
        assert!(
            h.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == ':')
        );
    }

    #[test]
    fn sign_hash_string_round_trip_and_tamper_detection() {
        let (sk, pk) = keypair();
        let h = "blake3:abc123".to_string();
        let sig = sign_hash_string(&h, &sk);
        assert!(verify_hash_string(&h, &pk, &sig));
        assert!(!verify_hash_string(&h, &pk, b"bad-sig"));
        assert!(!verify_hash_string("blake3:different", &pk, &sig));
        // The signed message is BLAKE3 of the hash STRING — a signature over
        // the raw string bytes (no digest) must not verify under the scheme.
        use ed25519_dalek::Signer;
        let raw_bytes_sig = sk.sign(b"blake3:abc123").to_bytes().to_vec();
        assert!(!verify_hash_string(&h, &pk, &raw_bytes_sig));
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
