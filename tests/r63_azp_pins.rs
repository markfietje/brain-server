//! R63 — `azp` (RFC 7519 §4.1.3) enforcement: the permanent pin suite.
//!
//! **What this round closes.** `azp` binds a token to an *application*. It was
//! absent from the entire tree — not in `Claims`, not in the env family, not
//! in any test. Where `BRAIN_JWT_AUDIENCE` is **tenant-wide** (the common case
//! for a shared IdP — Auth0, Okta, Entra ID and Keycloak all issue for many
//! client applications under one issuer) `azp` is the *only* per-application
//! binding, so a genuine token minted for a different application in the same
//! tenant was accepted. That is the token-intent / confused-deputy class.
//!
//! **The red-proofs.** `tests/r63_redproof_azp.rs` asserted the *vulnerable*
//! behavior against the pre-fix tree and passed (2/2), demonstrating the
//! finding before any code changed. Those assertions are inverted here:
//! `cross_application_azp_is_refused` and `absent_azp_is_refused_when_bound`
//! are the same inputs with the opposite verdict, so the pre-fix transcript
//! is the evidence these would have failed.
//!
//! **Two pins here CANNOT be red-first, and saying so is part of the record.**
//! `matching_azp_is_accepted` (R63.3) and `unset_azp_accepts_every_token`
//! (R63.4) are *over-refusal* and *non-breaking* guards: pre-fix every token
//! was accepted, so both pass trivially against the old code. A pin that has
//! never failed has not been tested — so both are proven by **mutation** in
//! the evidence file (make the gate refuse-everything / enforce-everything and
//! show the pin goes red). The mutation transcript is the red-proof for these
//! two, and that is a deliberate substitution, not an omission.
//!
//! Preregistrations: `R63_R63a_PREREGISTRATION_2026-09-29.md` (P63.1–P63.7).
//! Plan of record: `IMPL_R63_AZP_ENFORCEMENT_2026-09-28.md`.

use brain_server::auth::jwt::{
    AZP_UNBOUND_DISCLOSURE, AuthError, Claims, TokenType, VerifyingKey, azp_rejections,
    azp_unbound_disclosure, check_azp, verify_access_token,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rsa::RsaPrivateKey;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
use serde::Serialize;

const ISS: &str = "https://brain.test/";
const AUD: &str = "brain-server";
const KID: &str = "r63-kid-1";
/// The application this server is configured to accept.
const BOUND: &str = "brain-server";
/// A different application in the same tenant — the confused-deputy input.
const FOREIGN: &str = "some-other-application";

/// The token payload as an IdP would mint it. Mirrors `Claims` but keeps
/// `azp` under the test's control so a token can carry any authorized party,
/// including one this server has never heard of.
#[derive(Serialize)]
struct Payload {
    iss: String,
    aud: String,
    sub: String,
    jti: String,
    iat: u64,
    nbf: u64,
    exp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    azp: Option<String>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
}

fn payload(azp: Option<&str>) -> Payload {
    let t = now();
    Payload {
        iss: ISS.to_string(),
        aud: AUD.to_string(),
        sub: "user:alice".to_string(),
        jti: "r63-jti-001".to_string(),
        iat: t,
        nbf: t,
        exp: t + 600,
        azp: azp.map(str::to_string),
    }
}

/// Shared RSA fixture: one keypair, the verifying key under `KID`, and a
/// `Payload`-shaped minter so a test can put any `azp` on the wire.
struct Fixture {
    priv_key: RsaPrivateKey,
    keys: Vec<VerifyingKey>,
}

/// `azp_rejections()` is a PROCESS-GLOBAL static (the `concurrency::CONCURRENCY`
/// / `audit::BUSY_HITS` precedent — no `AppState` plumbing on purpose). That
/// makes it shared with every sibling test in this binary, so any test that
/// provokes a refusal moves it. **Every such test takes this lock**, which is
/// what lets the counter pin below measure a clean before/after.
///
/// Without it the pin is flaky — measured 1 failure in 6 full-binary runs
/// before this lock existed, because a sibling refusal landed inside the
/// measurement window. A flaky pin is a broken pin, and the fix is the lock,
/// not a loosened assertion.
fn counter_guard() -> std::sync::MutexGuard<'static, ()> {
    static COUNTER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    COUNTER_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

impl Fixture {
    fn new() -> Self {
        let mut rng = rand::rngs::ThreadRng::default();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA keypair for R63");
        let pub_key = rsa::RsaPublicKey::from(&priv_key);
        let pem = pub_key
            .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
            .expect("encode public PEM");
        let keys = vec![VerifyingKey {
            kid: KID.to_string(),
            alg: Algorithm::RS256,
            pinned_alg: Some(Algorithm::RS256),
            decoding_key: jsonwebtoken::DecodingKey::from_rsa_pem(pem.as_bytes())
                .expect("build decoding key from RSA PEM"),
        }];
        Self { priv_key, keys }
    }

    /// Mint a correctly-signed token carrying exactly the payload asked for.
    fn mint(&self, azp: Option<&str>) -> String {
        let pem = self
            .priv_key
            .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
            .expect("encode private PEM");
        let encoding = EncodingKey::from_rsa_pem(pem.as_bytes()).expect("build encoding key");
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(KID.to_string());
        encode(&header, &payload(azp), &encoding).expect("sign R63 test JWT")
    }

    /// Verify with azp enforcement BOUND to this server's application.
    fn verify_bound(&self, raw: &str) -> Result<Claims, AuthError> {
        verify_access_token(raw, &self.keys, ISS, AUD, Some(BOUND), TokenType::Access)
            .map(|(c, _)| c)
    }

    /// Verify in the historical UNBOUND posture.
    fn verify_unbound(&self, raw: &str) -> Result<Claims, AuthError> {
        verify_access_token(raw, &self.keys, ISS, AUD, None, TokenType::Access).map(|(c, _)| c)
    }
}

/// R63.1 — **the cross-application token is refused.** Valid signature, correct
/// `iss`, correct `aud`, but minted for a different application. Pre-fix this
/// exact input was ACCEPTED (`tests/r63_redproof_azp.rs`, pasted in evidence).
#[test]
fn cross_application_azp_is_refused() {
    let _counter = counter_guard();
    let fx = Fixture::new();
    let raw = fx.mint(Some(FOREIGN));
    match fx.verify_bound(&raw) {
        Err(AuthError::AzpBinding("mismatch")) => {}
        Err(other) => panic!("expected a typed azp mismatch refusal, got {other:?}"),
        Ok(claims) => panic!(
            "THE FINDING IS LIVE: a token minted for {FOREIGN} was accepted for \
             this server (sub={:?}). R63 has regressed.",
            claims.sub
        ),
    }
}

/// R63.2 — **absence red-proof.** With enforcement configured, a token with no
/// `azp` at all is refused. Per RFC 7519 §4.1.3 that token is *legal*; it is
/// the POLICY that refuses it, and the refusal is pinned so it can never
/// become an accident.
#[test]
fn absent_azp_is_refused_when_bound() {
    let _counter = counter_guard();
    let fx = Fixture::new();
    let raw = fx.mint(None);
    match fx.verify_bound(&raw) {
        Err(AuthError::AzpBinding("absent")) => {}
        Err(other) => panic!("expected a typed azp absence refusal, got {other:?}"),
        Ok(_) => panic!("an azp-less token was accepted while enforcement is configured"),
    }
}

/// A blank `azp` is not a binding. `azp: ""` is present-but-meaningless, and
/// fail-closed says it is treated exactly like absence rather than matching a
/// blank expectation.
#[test]
fn blank_azp_is_treated_as_absent() {
    let _counter = counter_guard();
    assert!(
        check_azp(Some(""), Some(BOUND)).is_err(),
        "a blank azp must not satisfy a bound expectation"
    );
    assert!(
        check_azp(Some("   "), Some(BOUND)).is_err(),
        "a whitespace azp must not satisfy a bound expectation"
    );
}

/// R63.3 — **match red-proof. A gate that rejects everything is not a gate.**
/// (Mutation-proven; see the module doc.)
#[test]
fn matching_azp_is_accepted() {
    let fx = Fixture::new();
    let raw = fx.mint(Some(BOUND));
    let claims = fx
        .verify_bound(&raw)
        .expect("a token whose azp matches MUST be accepted");
    assert_eq!(claims.sub, "user:alice");
    assert_eq!(claims.azp.as_deref(), Some(BOUND));
}

/// R63.4 — **unset is non-breaking.** With enforcement off, a token is accepted
/// exactly as it was before R63 existed — whatever its `azp`. This is the pin
/// that stops R63 becoming a breaking change for a live deployment.
/// (Mutation-proven; see the module doc.)
#[test]
fn unset_azp_accepts_every_token_exactly_as_before() {
    let fx = Fixture::new();
    // The three shapes an unbound deployment actually sees in the wild.
    for claimed in [None, Some(BOUND), Some(FOREIGN)] {
        let raw = fx.mint(claimed);
        fx.verify_unbound(&raw)
            .unwrap_or_else(|e| panic!("unbound posture must accept azp={claimed:?}, got {e:?}"));
    }
}

/// A whitespace-only *expectation* is unbound, not bound-and-matching. Guards
/// the `BRAIN_JWT_AZP="  "` case at the decision layer, complementing the
/// boot refusal in `blank_azp_env_refuses_boot`.
#[test]
fn blank_expectation_is_unbound_not_permissive() {
    assert!(
        check_azp(None, Some("  ")).is_ok(),
        "a blank expectation must not start refusing tokens"
    );
}

/// P63.6 / D63.6 — **ordering is a security invariant.** The `alg` whitelist
/// still fires before any key lookup, and the azp check still fires only after
/// the signature. Neither is an implementation detail; both are pinned.
///
/// The azp pin is the sharp one: a token with a **tampered signature** AND a
/// mismatching `azp` must be refused for the signature. If `check_azp` were
/// consulted before verification, this would report an azp failure — meaning
/// the server was reading a claim off an unverified token.
#[test]
fn azp_is_never_consulted_on_an_unverified_token() {
    let fx = Fixture::new();
    let raw = fx.mint(Some(FOREIGN));
    // Flip the first byte of the signature segment.
    let mut parts: Vec<&str> = raw.split('.').collect();
    let (first, rest) = parts[2].split_at(1);
    let flipped = format!("{}{rest}", if first == "A" { "B" } else { "A" });
    parts[2] = &flipped;
    let tampered = parts.join(".");
    match fx.verify_bound(&tampered) {
        Err(AuthError::AzpBinding(_)) => panic!(
            "ORDERING VIOLATION: an unverified token produced an azp verdict. The azp check \
             must run strictly after signature verification."
        ),
        Err(_) => {}
        Ok(_) => panic!("a tampered signature was accepted"),
    }
}

/// P63.6 / D63.6 — the pre-key-lookup `alg` whitelist still fires FIRST, ahead
/// of azp. With enforcement bound, an off-whitelist alg is refused as an
/// algorithm problem, never as an azp problem.
#[test]
fn alg_whitelist_fires_before_azp() {
    let fx = Fixture::new();
    let raw = fx.mint(Some(FOREIGN));
    // Re-header the token as an off-whitelist alg without re-signing: `alg` is
    // attacker controlled, and the whole point of the pre-key-lookup whitelist
    // is that this is refused before the attempt reaches a key.
    let mut segs: Vec<String> = raw.split('.').map(str::to_string).collect();
    use base64::Engine as _;
    let algs = [
        (
            "HS256",
            "{\"alg\":\"HS256\",\"typ\":\"JWT\",\"kid\":\"r63-kid-1\"}",
        ),
        (
            "PS256",
            "{\"alg\":\"PS256\",\"typ\":\"JWT\",\"kid\":\"r63-kid-1\"}",
        ),
    ];
    for (name, header_json) in algs {
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(header_json.as_bytes());
        segs[0] = b64;
        let forged = segs.join(".");
        match fx.verify_bound(&forged) {
            Err(AuthError::AzpBinding(_)) => panic!(
                "ORDERING VIOLATION: {name} produced an azp verdict. The alg whitelist must \
                 fire before any claim is consulted."
            ),
            Err(_) => {}
            Ok(_) => panic!("{name} was accepted"),
        }
    }
}

/// P63.2 / R63.5 — **the boot disclosure, as a decision.** JWT mode on + azp
/// unbound → one explicit line naming the gap. **Silence fails.**
#[test]
fn azp_boot_disclosure_names_the_unbound_state() {
    let line = azp_unbound_disclosure(true, None).expect(
        "an unbound JWT deployment MUST disclose it. Silence fails — an operator must never \
         have to guess whether token-intent enforcement is on.",
    );
    assert_eq!(line, AZP_UNBOUND_DISCLOSURE);
    // The disclosure has to actually SAY the thing. A generic string would
    // pass the `Some` check while telling the operator nothing.
    for needle in ["BRAIN_JWT_AZP", "azp", "not enforced", "aud"] {
        assert!(
            line.to_lowercase().contains(&needle.to_lowercase()),
            "the disclosure must name {needle:?}; it reads: {line}"
        );
    }
}

/// P63.2 / D63.4 — the disclosure is ABSENT when bound, and when JWT mode is
/// off. A deployment that enabled the control must not be warned about it.
#[test]
fn azp_boot_disclosure_is_silent_when_bound_or_not_jwt() {
    assert_eq!(
        azp_unbound_disclosure(true, Some(BOUND)),
        None,
        "a BOUND deployment must not be told the control is off"
    );
    assert_eq!(
        azp_unbound_disclosure(false, None),
        None,
        "opaque-token mode verifies no JWT; there is no azp to disclose"
    );
}

/// P63.5 / I63.5 — **the refusal is typed and says nothing about the token.**
/// Neither the wire code nor the `Display` may carry the presented `azp` or the
/// configured expectation: a refusal reason that echoes them is an oracle.
#[test]
fn azp_refusal_never_echoes_the_token_or_the_expectation() {
    let _counter = counter_guard();
    let err = check_azp(Some(FOREIGN), Some(BOUND)).expect_err("must refuse");
    let rendered = format!("{err} | {}", err.code());
    assert_eq!(err.code(), "azp_binding");
    for secret in [FOREIGN, BOUND, "user:alice"] {
        assert!(
            !rendered.contains(secret),
            "the refusal leaked {secret:?}: {rendered}"
        );
    }
    // The two categories must still be distinguishable to the operator.
    assert!(format!("{}", check_azp(None, Some(BOUND)).expect_err("absent")).contains("absent"));
    assert!(
        format!(
            "{}",
            check_azp(Some(FOREIGN), Some(BOUND)).expect_err("mismatch")
        )
        .contains("different application")
    );
}

/// P63.7 / I63.5 / D63.5 — **enforcement is observable.** The counter moves only
/// on a refusal, so a rising series is a positive statement that the control is
/// live. Every test in this binary that provokes a refusal holds
/// [`counter_guard`], so the measurement window is clean; `>=` on the refusal
/// side is still the race-tolerant form.
#[test]
fn azp_rejection_counter_moves_only_on_refusal() {
    let _counter = counter_guard();
    let before = azp_rejections();
    // An accepted token must not move it.
    let _ = check_azp(Some(BOUND), Some(BOUND));
    let _ = check_azp(None, None);
    let after_accepts = azp_rejections();
    assert_eq!(
        before, after_accepts,
        "an accepted token incremented the rejection counter"
    );
    // A refusal must.
    let _ = check_azp(Some(FOREIGN), Some(BOUND));
    assert!(
        azp_rejections() > after_accepts,
        "a refusal did not increment brain_jwt_azp_rejected_total"
    );
}

/// P63.4 — **a present-but-blank `BRAIN_JWT_AZP` refuses the boot.** It resolves
/// to the same posture as unset, so accepting it silently would hand an
/// operator who believes enforcement is on a deployment where it is not — the
/// false-sense-of-enforcement failure mode the ceilings name.
///
/// Env-mutating, so it takes the file-local ENV_LOCK (the
/// `config.rs` / `mcp.rs` posture). This test binary is its own process, so the
/// lock fully serializes env access here.
#[test]
fn blank_azp_env_refuses_boot() {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    const VAR: &str = brain_server::config::JWT_AZP_ENV;
    let restore = std::env::var(VAR).ok();

    // SAFETY: single-threaded under ENV_LOCK.
    unsafe {
        std::env::set_var(VAR, "");
        assert!(
            brain_server::config::validate_jwt_azp_env().is_err(),
            "an empty BRAIN_JWT_AZP must REFUSE the boot, not degrade to unbound"
        );
        std::env::set_var(VAR, "   ");
        assert!(
            brain_server::config::validate_jwt_azp_env().is_err(),
            "a whitespace-only BRAIN_JWT_AZP must REFUSE the boot"
        );

        // The total vocabulary: absent = unbound, present = bound.
        std::env::remove_var(VAR);
        assert_eq!(
            brain_server::config::resolve_jwt_azp().expect("unset is not an error"),
            None,
            "unset must resolve to the unbound posture (the non-breaking default)"
        );
        std::env::set_var(VAR, BOUND);
        assert_eq!(
            brain_server::config::resolve_jwt_azp().expect("a real value resolves"),
            Some(BOUND.to_string()),
            "a real value must resolve to the bound posture"
        );
        std::env::set_var(VAR, "  brain-server  ");
        assert_eq!(
            brain_server::config::resolve_jwt_azp().expect("a padded real value resolves"),
            Some(BOUND.to_string()),
            "a padded value must resolve trimmed"
        );
    }

    match restore {
        Some(v) => unsafe { std::env::set_var(VAR, v) },
        None => unsafe { std::env::remove_var(VAR) },
    }
}

/// The `Claims` field is OPTIONAL in the type, because it is optional in the
/// token. A token that omits `azp` must still PARSE — serde must not make the
/// field required, or every existing deployment's tokens would 400 at the
/// decode step instead of at the policy step.
#[test]
fn a_token_without_azp_still_decodes() {
    let fx = Fixture::new();
    let raw = fx.mint(None);
    let (claims, _) = verify_access_token(&raw, &fx.keys, ISS, AUD, None, TokenType::Access)
        .expect(
            "an azp-less token is RFC-legal and must still decode; making the field required \
             would fail it at parse time instead of at the policy layer",
        );
    assert_eq!(claims.azp, None);
}
