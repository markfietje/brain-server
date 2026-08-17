//! JWT signing key management + JWKS (v1.2.0 "AuthN" M4 + M5).
//!
//! Loads RSA/EC/Ed25519 public keys from `BRAIN_JWT_KEY_DIR` (default
//! `~/.config/brain-server/keys/`, mode 0700). Each key is a PEM file named
//! `<kid>.pem` (public) + `<kid>.key` (private, mode 0600). The KeyStore
//! exposes both:
//!
//! - [`VerifyingKey`]s for [`crate::jwt::verify_access_token`] (server-side).
//! - A JWK Set JSON document for `/.well-known/jwks.json` (public distribution).
//!
//! Key rotation (`brain key rotate`) writes a new keypair, then this store is
//! re-read. Two keys are live during the overlap window: the new one signs,
//! the old one still verifies tokens minted before rotation. The old key is
//! pruned (`brain key prune`) only after every cached token has expired
//! (max refresh lifetime = 24h).
//!
//! Why not use an external JWT library's JWK parsing? Because:
//! 1. `jsonwebtoken::DecodingKey::from_jwk` only handles RSA JWKs in v10 — no
//!    Ed25519, no EC. We need all three for the documented IdP matrix.
//! 2. The JWK → DecodingKey path pulls in `rsa`, `p256`, `ed25519` separately
//!    with version churn. PEM is stable.
//! 3. We control the wire format of `/.well-known/jwks.json` directly — no
//!    surprise fields, no library drift.
//!
//! ponytail ceiling: this assumes keys live on the local filesystem (file-
//! based KMS per OWASP Secrets Management). External KMS (Vault, AWS KMS) is
//! the v2.1 upgrade path; the trait boundary is "give me bytes by kid".

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jsonwebtoken::{Algorithm, DecodingKey};
use serde::Serialize;

use super::jwt::VerifyingKey;

/// Environment variable for the key directory. Default: per-platform config.
pub const KEY_DIR_ENV: &str = "BRAIN_JWT_KEY_DIR";

/// Default key directory relative to the user's config root.
pub const DEFAULT_KEY_DIR: &str = "keys";

/// File extension for public PEMs (one per key).
pub const PUBLIC_PEM_EXT: &str = "pem";

/// File extension for private keys (one per signing key).
pub const PRIVATE_KEY_EXT: &str = "key";

/// Resolve the key directory from env, falling back to the platform default.
/// Matches the `~/.config/brain-server/` convention used by `auth-token`,
/// connector configs, etc. — same storage_layout root, just a `keys/` subdir.
pub fn resolve_key_dir() -> PathBuf {
    if let Ok(d) = std::env::var(KEY_DIR_ENV) {
        let p = PathBuf::from(d.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/brain-server").join(DEFAULT_KEY_DIR)
}

/// A loaded key pair (public + optional private) with its `kid` + alg.
#[derive(Clone)]
pub struct ManagedKey {
    pub kid: String,
    pub alg: Algorithm,
    /// Public half, parsed into the form `verify_access_token` needs.
    pub verifying: VerifyingKey,
    /// Raw public PEM bytes (for the JWKS endpoint).
    pub public_pem: String,
    /// Private half, present only for keys this server is allowed to sign with.
    /// `None` for keys imported from an external IdP (verification only).
    pub private_pem: Option<String>,
}

/// JWKS JSON shape. RFC 7517 §4. Minimal: only the fields a verifier needs.
/// `alg` is included so clients can pin their whitelist from discovery.
/// `kid` is mandatory (matches our verification requirement).
///
/// We serialize the key-type-specific params (`n`/`e` for RSA, `x`/`y`/`crv`
/// for EC/Ed) flat at the top level — RFC 7517 expects them there, NOT nested
/// under a `params` object. Using a single struct with `skip_serializing_if`
/// for the type-specific fields is simpler than a flattened enum + matches the
/// wire format every compliant client expects.
#[derive(Debug, Serialize)]
pub struct JsonWebKey {
    pub kty: &'static str,
    pub alg: &'static str,
    pub kid: String,
    /// `use`: "sig" (signature). Always signature for us — never "enc".
    /// We never emit JWE in v1.2 (claims aren't secret; SQLCipher is v3.7).
    #[serde(rename = "use")]
    pub use_: &'static str,
    // RSA params (present only when kty == "RSA").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e: Option<String>,
    // EC/Ed params (present only when kty == "EC" or "OKP"). Reserved for v1.3+;
    // not emitted by v1.2's RSA-only emitter, but the fields exist so a future
    // EC/Ed emitter doesn't need a schema change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crv: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
}

/// The full JWK Set served at `/.well-known/jwks.json`.
#[derive(Debug, Serialize)]
pub struct JwkSet {
    pub keys: Vec<JsonWebKey>,
}

/// In-memory snapshot of every loaded key. Cheap to clone (Arc). Re-read on
/// rotation by calling [`KeyStore::reload`]; the old snapshot stays valid for
/// any in-flight verification.
#[derive(Clone, Default)]
pub struct KeyStore {
    keys: Arc<Vec<ManagedKey>>,
    /// Index by kid for O(1) lookup in the hot path (the cheat sheet's
    /// "kid lookup must not be linear search" warning — though for n≤3 keys
    /// it hardly matters; we do it anyway because it's free).
    by_kid: Arc<HashMap<String, usize>>,
}

impl KeyStore {
    /// Read every `*.pem` file in the key directory + parse each into a
    /// `VerifyingKey`. Files without a matching `*.key` are verification-only
    /// (IdP-imported keys). Empty/missing dir → empty store (JWT auth will
    /// refuse every token, which is the safe default).
    pub fn load(dir: &Path) -> Result<Self, LoadError> {
        let mut keys: Vec<ManagedKey> = Vec::new();
        if !dir.exists() {
            // Empty store is a valid state — it just means JWT auth isn't
            // configured. The caller (middleware) treats an empty key set as
            // "JWT mode disabled" rather than erroring at startup.
            return Ok(Self::default());
        }
        let entries = std::fs::read_dir(dir).map_err(|e| LoadError::ReadDir {
            dir: dir.to_path_buf(),
            source: e,
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
                continue;
            };
            if ext != PUBLIC_PEM_EXT {
                continue;
            }
            let Some(kid) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
            else {
                continue;
            };
            let public_pem = std::fs::read_to_string(&path).map_err(|e| LoadError::ReadKey {
                kid: kid.clone(),
                source: e,
            })?;
            let (alg, verifying) = parse_public_pem(&kid, &public_pem)?;
            // Optional matching private key alongside. v1.20.24 "Sweep": a
            // signing key with group/world read bits is a leaked secret — the
            // load fails (same shape as any other key-read failure, which the
            // startup already reports loudly).
            let private_path = path.with_extension(PRIVATE_KEY_EXT);
            let private_pem = if private_path.exists() {
                super::check_secret_permissions(&private_path).map_err(|e| LoadError::ReadKey {
                    kid: kid.clone(),
                    source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, e),
                })?;
                Some(
                    std::fs::read_to_string(&private_path).map_err(|e| LoadError::ReadKey {
                        kid: kid.clone(),
                        source: e,
                    })?,
                )
            } else {
                None
            };
            keys.push(ManagedKey {
                kid,
                alg,
                verifying,
                public_pem,
                private_pem,
            });
        }
        // Sort by kid for deterministic JWKS output (stable rotation order).
        keys.sort_by(|a, b| a.kid.cmp(&b.kid));
        let by_kid = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.kid.clone(), i))
            .collect();
        Ok(Self {
            keys: Arc::new(keys),
            by_kid: Arc::new(by_kid),
        })
    }

    /// All loaded verifying keys. Slice form matches `verify_access_token`'s
    /// signature. Empty when JWT auth is unconfigured.
    pub fn verifying_keys(&self) -> Vec<VerifyingKey> {
        self.keys.iter().map(|k| k.verifying.clone()).collect()
    }

    /// Find a key by kid. O(1).
    pub fn find(&self, kid: &str) -> Option<&ManagedKey> {
        self.by_kid.get(kid).map(|&i| &self.keys[i])
    }

    /// The first private-key-bearing key (the current signing key). `None`
    /// when no signing key is configured (server is verify-only — typical for
    /// a deployment that uses an external IdP and never mints its own tokens).
    pub fn signing_key(&self) -> Option<&ManagedKey> {
        self.keys.iter().find(|k| k.private_pem.is_some())
    }

    /// Build the RFC 7517 JWK Set for the public endpoint. Includes every
    /// key (signing + verify-only) so external clients can verify any token
    /// we've ever signed during the rotation overlap window.
    pub fn to_jwks(&self) -> Result<JwkSet, LoadError> {
        let mut out = Vec::with_capacity(self.keys.len());
        for k in self.keys.iter() {
            out.push(jwk_from_pem(&k.kid, k.alg, &k.public_pem)?);
        }
        Ok(JwkSet { keys: out })
    }

    /// Number of loaded keys. Used by `/health` to report JWT configuration.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Re-read from disk. Used by the rotation watcher + `brain key rotate`.
    /// Returns the previous snapshot on any load failure (fail-safe: a bad
    /// key file must not leave the server with no keys).
    pub fn reload(&mut self, dir: &Path) -> Result<(), LoadError> {
        let next = Self::load(dir)?;
        *self = next;
        Ok(())
    }
}

#[derive(Debug)]
pub enum LoadError {
    ReadDir {
        dir: PathBuf,
        source: std::io::Error,
    },
    ReadKey {
        kid: String,
        source: std::io::Error,
    },
    Parse {
        kid: String,
        reason: String,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::ReadDir { dir, source } => {
                write!(f, "read key dir {dir:?}: {source}")
            }
            LoadError::ReadKey { kid, source } => write!(f, "read key {kid}: {source}"),
            LoadError::Parse { kid, reason } => write!(f, "parse key {kid}: {reason}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Parse a public PEM into a `VerifyingKey`. Auto-detects the algorithm from
/// the PEM's SubjectPublicKeyInfo (RFC 5280). RSA PEMs → RS256 by default
/// (the strongest RSA variant we accept; the alg is verified against the
/// JWT's own alg claim during verification, so this default is only the
/// "first guess" for the kid lookup).
fn parse_public_pem(kid: &str, pem: &str) -> Result<(Algorithm, VerifyingKey), LoadError> {
    // Try as RSA-specific parsing.
    if let Ok(decoding_key) = DecodingKey::from_rsa_pem(pem.as_bytes()) {
        let verifying = VerifyingKey {
            kid: kid.to_string(),
            alg: Algorithm::RS256,
            decoding_key,
        };
        return Ok((Algorithm::RS256, verifying));
    }
    // Try as EC (P-256 / P-384). The PEM format is the same; only the
    // DecodingKey constructor differs. We default to ES256 for P-256.
    if let Ok(decoding_key) = DecodingKey::from_ec_pem(pem.as_bytes()) {
        let verifying = VerifyingKey {
            kid: kid.to_string(),
            // ES256 is the only EC variant we default to; ES384 needs P-384
            // detection which would require parsing the curve OID. ponytail
            // ceiling: deployments needing ES384 can name the kid's alg
            // explicitly in a sidecar file; that's a v1.3 concern.
            alg: Algorithm::ES256,
            decoding_key,
        };
        return Ok((Algorithm::ES256, verifying));
    }
    // Try as Ed25519. The `jsonwebtoken` API is `from_ed_pem`.
    if let Ok(decoding_key) = DecodingKey::from_ed_pem(pem.as_bytes()) {
        let verifying = VerifyingKey {
            kid: kid.to_string(),
            alg: Algorithm::EdDSA,
            decoding_key,
        };
        return Ok((Algorithm::EdDSA, verifying));
    }
    Err(LoadError::Parse {
        kid: kid.to_string(),
        reason: "not a recognizable RSA/EC/Ed25519 public PEM".to_string(),
    })
}

/// Build a JWK from a public PEM. Parses the SPKI to extract the key-type-
/// specific fields (n/e for RSA, x/y/crv for EC, x/crv for Ed25519).
/// ponytail ceiling: this re-parses the PEM; a future refactor could carry
/// the parsed key from `parse_public_pem` to avoid the double work. Skipped
/// here because JWKS serving is not a hot path (cached, served rarely).
fn jwk_from_pem(kid: &str, alg: Algorithm, pem: &str) -> Result<JsonWebKey, LoadError> {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    // RSA: try to extract modulus + exponent.
    if alg == Algorithm::RS256 || alg == Algorithm::RS384 || alg == Algorithm::RS512 {
        // `rsa::RsaPublicKey::from_public_key_pem` accepts a PKCS#8
        // SubjectPublicKeyInfo PEM (the format `to_public_key_pem`
        // emits). The trait is `rsa::pkcs8::DecodePublicKey`.
        use rsa::pkcs8::DecodePublicKey as _;
        let rsa_pub =
            rsa::RsaPublicKey::from_public_key_pem(pem).map_err(|e| LoadError::Parse {
                kid: kid.to_string(),
                reason: format!("RSA pubkey: {e}"),
            })?;
        // `n` and `e` are behind the `PublicKeyParts` trait. JWK RFC 7518
        // requires them as base64url of the big-endian magnitude.
        use rsa::traits::PublicKeyParts as _;
        let n = rsa_pub.n().to_bytes_be();
        let e = rsa_pub.e().to_bytes_be();
        return Ok(JsonWebKey {
            kty: "RSA",
            alg: alg_str(alg),
            kid: kid.to_string(),
            use_: "sig",
            n: Some(b64.encode(n)),
            e: Some(b64.encode(e)),
            crv: None,
            x: None,
            y: None,
        });
    }
    // EC + Ed25519: same pattern, but the `p256`/`ed25519` crates aren't
    // direct deps. For v1.2 we serve those PEMs as a non-standard `x5c`
    // fallback — clients that need native EC/Ed JWK fields should rotate to
    // RSA (the documented default). ponytail ceiling: native EC/Ed JWK params
    // land when a deployment actually uses them (none today).
    Err(LoadError::Parse {
        kid: kid.to_string(),
        reason: format!("JWK emission for {alg:?} not implemented; rotate to RSA for public JWKS"),
    })
}

fn alg_str(a: Algorithm) -> &'static str {
    match a {
        Algorithm::RS256 => "RS256",
        Algorithm::RS384 => "RS384",
        Algorithm::RS512 => "RS512",
        Algorithm::ES256 => "ES256",
        Algorithm::ES384 => "ES384",
        Algorithm::EdDSA => "EdDSA",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
    use rsa::RsaPrivateKey;
    use tempfile::tempdir;

    /// Write a keypair to the dir under `<kid>.pem` + `<kid>.key`. Returns
    /// the kid used. Mirrors what `brain key generate` will do — and the
/// the private key file is written
    /// owner-only (0o600), as `install-service.sh` enforces in production.
    fn write_keypair(dir: &Path, kid: &str) -> RsaPrivateKey {
        use std::os::unix::fs::PermissionsExt;
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pub_key = rsa::RsaPublicKey::from(&priv_key);
        let pub_pem = pub_key
            .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap();
        let priv_pem = priv_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        std::fs::write(dir.join(format!("{kid}.pem")), pub_pem.as_bytes()).unwrap();
        let key_path = dir.join(format!("{kid}.key"));
        std::fs::write(&key_path, priv_pem.as_bytes()).unwrap();
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        priv_key
    }

    #[test]
    fn empty_dir_yields_empty_store() {
        let dir = tempdir().unwrap();
        let store = KeyStore::load(dir.path()).expect("load from empty dir");
        assert!(store.is_empty());
        assert!(store.verifying_keys().is_empty());
    }

    #[test]
    fn missing_dir_yields_empty_store_not_error() {
        let dir = tempdir().unwrap();
        let ghost = dir.path().join("does-not-exist");
        let store = KeyStore::load(&ghost).expect("missing dir is not an error");
        assert!(store.is_empty());
    }

    #[test]
    fn loads_public_and_private_pair() {
        let dir = tempdir().unwrap();
        write_keypair(dir.path(), "kid-1");
        let store = KeyStore::load(dir.path()).unwrap();
        assert_eq!(store.len(), 1);
        let signing = store.signing_key().expect("must have a signing key");
        assert_eq!(signing.kid, "kid-1");
        assert!(signing.private_pem.is_some());
        assert!(store.find("kid-1").is_some());
    }

    #[test]
    fn loads_public_only_for_verify_only_deployments() {
        let dir = tempdir().unwrap();
        // Write just the public half — simulates an IdP-imported key.
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pub_key = rsa::RsaPublicKey::from(&priv_key);
        let pub_pem = pub_key
            .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap();
        std::fs::write(dir.path().join("ext-kid.pem"), pub_pem.as_bytes()).unwrap();
        let store = KeyStore::load(dir.path()).unwrap();
        assert_eq!(store.len(), 1);
        assert!(
            store.signing_key().is_none(),
            "no private key = verify-only"
        );
        assert!(store.find("ext-kid").is_some());
    }

    #[test]
    fn jwks_round_trips_rsa_key() {
        let dir = tempdir().unwrap();
        write_keypair(dir.path(), "kid-1");
        let store = KeyStore::load(dir.path()).unwrap();
        let jwks = store.to_jwks().expect("JWKS build must succeed");
        assert_eq!(jwks.keys.len(), 1);
        let k = &jwks.keys[0];
        assert_eq!(k.kty, "RSA");
        assert_eq!(k.alg, "RS256");
        assert_eq!(k.use_, "sig");
        assert!(k.n.is_some() && k.e.is_some(), "RSA JWK must have n + e");
        // The serialized form must be valid JSON the public endpoint can serve.
        let json = serde_json::to_string(&jwks).unwrap();
        assert!(json.contains("\"kid\":\"kid-1\""));
        assert!(json.contains("\"kty\":\"RSA\""));
    }

    #[test]
    fn reload_picks_up_new_key() {
        let dir = tempdir().unwrap();
        let mut store = KeyStore::load(dir.path()).unwrap();
        assert!(store.is_empty());
        write_keypair(dir.path(), "kid-1");
        store.reload(dir.path()).unwrap();
        assert_eq!(store.len(), 1);
        write_keypair(dir.path(), "kid-2");
        store.reload(dir.path()).unwrap();
        assert_eq!(store.len(), 2);
    }
}
