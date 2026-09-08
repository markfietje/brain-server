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

/// Parse a `did:key:z…` Ed25519 did back into its verifying key — the inverse
/// of [`did_key_from_ed25519`]. `None` on any malformed form (wrong prefix,
/// wrong multicodec, wrong length, undecodable base58).
pub fn verifying_key_from_did(did: &str) -> Option<VerifyingKey> {
    let b58 = did.strip_prefix("did:key:z")?;
    let buf = bs58::decode(b58).into_vec().ok()?;
    if buf.len() != 34 || buf[0] != 0xed || buf[1] != 0x01 {
        return None;
    }
    let mut pk = [0u8; 32];
    pk.copy_from_slice(&buf[2..]);
    VerifyingKey::from_bytes(&pk).ok()
}

/// Verify a [`sign_manifest_bytes`] signature over the exact bytes. False
/// (never errors) on any malformed did/signature — the standby status path
/// fails closed on this.
pub fn verify_manifest_bytes(did: &str, sig_hex: &str, bytes: &[u8]) -> bool {
    let Some(vk) = verifying_key_from_did(did) else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(sig_hex) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(&sig_bytes) else {
        return false;
    };
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    vk.verify(hex::encode(h.finalize()).as_bytes(), &sig)
        .is_ok()
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

/// Sign arbitrary manifest bytes under the parcels convention: Ed25519 over
/// the lowercase-hex SHA-256 digest STRING of the bytes — the 64-char hex text
/// is the signed message, not the raw digest. Extracted from
/// parcels' export path so the warm-standby cycle manifests sign under the
/// exact same scheme (`src/standby.rs`); parcels consumes this helper and its
/// bundle output is pinned byte-identical by
/// `parcel_signature_bytes_unchanged` (Ed25519 is deterministic, so equal
/// inputs must yield equal signatures — the pin recomputes the pre-extraction
/// formula inline with raw dalek calls).
/// Returns `(signature_hex, signed_by did:key)`.
pub fn sign_manifest_bytes(sk: &SigningKey, bytes: &[u8]) -> (String, String) {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let digest_hex = hex::encode(h.finalize());
    let sig = sk.sign(digest_hex.as_bytes());
    (
        hex::encode(sig.to_bytes()),
        did_key_from_ed25519(&sk.verifying_key().to_bytes()),
    )
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
    /// Per-token id. When present, a process-lifetime replay cache keyed on
    /// `(jti, method, path)` rejects reuse of the same token on any endpoint
    /// other than the one it was first seen on — mirroring the JWT jti
    /// denylist posture while keeping per-request-bearer retries valid.
    /// Absent on legacy tokens (they stay expiry-only; the documented
    /// ceiling until owners re-mint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
}

impl CapabilityToken {
    pub fn new(alg: &str, iss: &str, verbs: &[&str], scope: Option<&str>, exp: u64) -> Self {
        Self {
            alg: alg.to_string(),
            iss: iss.to_string(),
            verbs: verbs.iter().map(|s| s.to_string()).collect(),
            scope: scope.map(|s| s.to_string()),
            exp,
            jti: None,
        }
    }
}

/// Process-lifetime replay cache for capability tokens that carry a `jti`.
/// Bounded: entries prune lazily once expired; the count is capped at
/// [`CAP_REPLAY_CAP`] so a flood of unique tokens cannot grow memory
/// unboundedly. Poisoned lock = deny (fail-closed).
///
/// The cache pins each jti to the FIRST `(method, path)` it presented on:
/// capability tokens are presented as per-request bearers (nothing in-tree
/// mints one-time tokens), so keying on `jti` alone burned the single use on
/// the first request and refused every legitimate re-presentation — retries
/// and repeat calls on the same endpoint 401'd for the token's whole life.
/// Pinned this way, a token serves its declared verb×scope on that endpoint
/// repeatedly (retry-safe), while reuse on any DIFFERENT method/path is a
/// replay and is refused — lateral movement stays closed.
#[derive(Default)]
pub struct ReplayCache {
    /// Lock bounds (Headroom): the critical section is vec arithmetic —
    /// cap check + expiry retain (the extreme-flood `clear`), a linear scan
    /// pinning jti→(method, path), one push. No I/O, no nesting, no SQL.
    /// Poison: fail-CLOSED (`false` = replay = refused). Request-hot holder
    /// (the capability middleware on every cap-token-bearing request), so
    /// the acquire is wait-measured.
    seen: std::sync::Mutex<Vec<(ReplayKey, u64)>>,
}

/// (jti, method, path) — the endpoint-pinned replay key.
type ReplayKey = (String, String, String);

const CAP_REPLAY_CAP: usize = 4096;

impl ReplayCache {
    /// Returns `true` when this (jti, method, path) was NOT seen before and
    /// is now recorded (first presentation). `false` = replay or poisoned
    /// state.
    pub fn first_presentation(&self, jti: &str, method: &str, path: &str, exp: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let Ok(mut list) = crate::concurrency::mutex_guard_measured(&self.seen) else {
            // Poison ⇒ fail CLOSED (a replay-cache read failure must read as
            // "replay", never as "fresh") — unchanged.
            return false;
        };
        if list.len() >= CAP_REPLAY_CAP {
            list.retain(|(_, e)| *e > now);
            if list.len() >= CAP_REPLAY_CAP {
                // At the cap, evict the OLDEST quarter (the list is
                // insertion-ordered — drain the front). The old flood-clear
                // dropped EVERY pin, letting an attacker replay anything
                // older than the flood window; now recent pins survive a
                // flood and the documented trade-off shrinks to "the oldest
                // quarter of the window".
                let evict = list.len() / 4;
                list.drain(..evict);
            }
        }
        // Pin: a jti is bound to the FIRST (method, path) it presented on.
        // Retries / repeat calls on that exact endpoint stay valid;
        // presentation on ANY other endpoint is a replay.
        let pinned = list
            .iter_mut()
            .find_map(|((j, m, p), _)| (j == jti).then_some((m, p)));
        let fresh = pinned.is_none_or(|(m, p)| **m == *method && **p == *path);
        if fresh {
            list.push(((jti.to_string(), method.to_string(), path.to_string()), exp));
        }
        fresh
    }
}

/// The process-global cache consulted at every capability-token acceptance.
pub static CAP_REPLAY: std::sync::LazyLock<ReplayCache> =
    std::sync::LazyLock::new(ReplayCache::default);

/// Record-and-check a parsed token's `jti` at acceptance time. Tokens without
/// a `jti` are legacy and pass through (expiry-only ceiling). Keyed on
/// `(jti, method, path)` — see [`ReplayCache`] for why.
pub fn cap_replay_check(cap: &CapabilityToken, method: &str, path: &str) -> bool {
    cap.jti
        .as_ref()
        .is_none_or(|jti| CAP_REPLAY.first_presentation(jti, method, path, cap.exp))
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
    use rand::Rng;

    fn keypair() -> (SigningKey, [u8; 32]) {
        let mut seed = [0u8; 32];
        rand::rng().fill_bytes(&mut seed);
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

    /// The parcels-convention manifest signature: deterministic
    /// over the bytes, bound to the signer did, and equal to the
    /// pre-extraction formula (Ed25519 over the hex SHA-256 string)
    /// recomputed inline with raw dalek calls — the helper is a pure move.
    #[test]
    fn sign_manifest_bytes_matches_parcels_convention() {
        let (sk, pk) = keypair();
        let bytes = b"manifest bytes 123";
        let (sig_hex, signed_by) = sign_manifest_bytes(&sk, bytes);
        assert_eq!(signed_by, did_key_from_ed25519(&pk));
        // Deterministic: same key + same bytes → same signature.
        let (sig_hex_2, _) = sign_manifest_bytes(&sk, bytes);
        assert_eq!(sig_hex, sig_hex_2);
        // Recomputed pre-extraction formula, inline (not via the helper).
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(bytes);
        let expected = sk.sign(hex::encode(h.finalize()).as_bytes());
        assert_eq!(sig_hex, hex::encode(expected.to_bytes()));
        // A one-bit content change changes the signature.
        let (sig_hex_3, _) = sign_manifest_bytes(&sk, b"manifest bytes 124");
        assert_ne!(sig_hex, sig_hex_3);
    }

    /// verify_manifest_bytes is the fail-closed twin: true for a genuine
    /// sign_manifest_bytes signature over the same bytes, false for a
    /// tampered payload, a foreign did, or malformed input.
    #[test]
    fn verify_manifest_bytes_round_trip_and_tamper() {
        let (sk, pk) = keypair();
        let bytes = b"standby manifest json";
        let (sig_hex, did) = sign_manifest_bytes(&sk, bytes);
        assert!(did.starts_with("did:key:z"));
        assert!(verify_manifest_bytes(&did, &sig_hex, bytes));
        assert!(!verify_manifest_bytes(&did, &sig_hex, b"tampered bytes"));
        let (_, other_pk) = keypair();
        assert!(
            !verify_manifest_bytes(&did_key_from_ed25519(&other_pk), &sig_hex, bytes),
            "a different signer's did must not verify the signature"
        );
        assert!(!verify_manifest_bytes(&did, "zz", bytes));
        assert!(!verify_manifest_bytes("not-a-did", &sig_hex, bytes));
        assert!(verify_manifest_bytes(
            &did_key_from_ed25519(&pk),
            &sig_hex,
            bytes
        ));
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
            jti: None,
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

    /// A jti-bearing token is accepted once per (jti, method, path): the
    /// same endpoint may re-present it (per-request-bearer retries), but ANY
    /// other method/path is a replay and must be refused. Legacy (no-jti)
    /// tokens pass through expiry-only.
    #[test]
    fn capability_jti_replay_cache_rejects_second_presentation() {
        let cache = ReplayCache::default();
        assert!(cache.first_presentation("cap-1", "POST", "/ump/recall", u64::MAX));
        assert!(
            cache.first_presentation("cap-1", "POST", "/ump/recall", u64::MAX),
            "same endpoint re-presentation is retry, not replay"
        );
        assert!(
            !cache.first_presentation("cap-1", "GET", "/ump/memory/1", u64::MAX),
            "different method+path on same jti is a replay"
        );
        assert!(
            !cache.first_presentation("cap-1", "POST", "/ump/forget", u64::MAX),
            "different path, same jti is still a replay"
        );
        assert!(cache.first_presentation("cap-2", "POST", "/ump/recall", u64::MAX));

        // Legacy tokens without jti are not replay-tracked.
        let (_, pk) = keypair();
        let legacy = CapabilityToken {
            alg: "EdDSA".into(),
            iss: did_key_from_ed25519(&pk),
            verbs: vec!["read".into()],
            scope: None,
            exp: u64::MAX,
            jti: None,
        };
        assert!(cap_replay_check(&legacy, "POST", "/ump/recall"));
        assert!(cap_replay_check(&legacy, "GET", "/ump/memory/1"));

        let mut minted = legacy.clone();
        minted.jti = Some("cap-3".into());
        assert!(cap_replay_check(&minted, "POST", "/ump/recall"));
        assert!(
            cap_replay_check(&minted, "POST", "/ump/recall"),
            "retry on the same endpoint stays valid"
        );
        assert!(
            !cap_replay_check(&minted, "GET", "/ump/export"),
            "second endpoint refused as replay"
        );
    }

    /// At the cap, the OLDEST quarter is evicted (recent pins survive);
    /// the pinned recent jti still reads as a retry after the flood.
    #[test]
    fn flood_evicts_oldest_quarter_keeps_recent() {
        let cache = ReplayCache::default();
        assert!(cache.first_presentation("keep-me", "POST", "/ump/recall", u64::MAX));
        for i in 0..(CAP_REPLAY_CAP + 64) {
            let jti = format!("flood-{i}");
            let _ = cache.first_presentation(&jti, "POST", "/ump/flood", u64::MAX);
        }
        let list = cache.seen.lock().unwrap_or_else(|e| e.into_inner());
        assert!(list.len() < CAP_REPLAY_CAP, "bounded: {}", list.len());
        let last = list.last().map(|((j, _, _), _)| j.clone()).unwrap();
        assert_eq!(last, format!("flood-{}", CAP_REPLAY_CAP + 63));
    }

    /// The structural memory bound is pinned: the cache can never hold
    /// more than the cap under any input sequence.
    #[test]
    fn cache_bounded_memory_pinned() {
        let cache = ReplayCache::default();
        for i in 0..(CAP_REPLAY_CAP * 3) {
            let jti = format!("wave-{i}");
            let _ = cache.first_presentation(&jti, "GET", "/ump/x", u64::MAX);
        }
        let list = cache.seen.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            list.len() <= CAP_REPLAY_CAP,
            "the cache must stay <= cap: {}",
            list.len()
        );
    }
}
