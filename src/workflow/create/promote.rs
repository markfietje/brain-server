//! Promotion: a human, digest-bound, out-of-band, single-use act — and this
//! round it does not happen at all.
//!
//! ## The loop ships inert
//!
//! [`PROMOTION_ENABLED`] is a compile-time `false` and nothing can change it:
//! there is no environment variable, no flag, no header. The route exists, is
//! authorized, is audited, and returns a typed `promotion_disabled` refusal in
//! every configuration.
//!
//! ## Why that is the feature
//!
//! A deterministic gate's honesty is a MEASURED property, not an architectural
//! one. The literature's single relevant datapoint reads zero failures in
//! benchmark and one in a hundred and forty-six out of benchmark: a
//! deterministic gate is *not* perfect off-distribution, and **nobody has
//! published a long-run figure.** Shipping an unmeasured gate as a safety
//! property would be claiming something no one has demonstrated.
//!
//! So the measurement is the deliverable and the promotion is not. The
//! refusal stream accumulates from the moment the loop is deployed; a published
//! out-of-sample figure, with a named owner, is what turns this constant into
//! a runtime decision. Until then the honest state is "unmeasured", and the
//! docs say so in those words.
//!
//! ## The token, for when it is enabled
//!
//! Bound to the claim's evidence digest, single-use, expiring, and refused if
//! any byte of the claim or its evidence moved. The approval dialog belongs in
//! the client UI and NOT in the conversation: approvals are strict capabilities
//! bound to their target, and routing them through the conversation makes the
//! user's own approval socially engineerable by the thing being approved.
//! That distinction — a gate that covers actions and egress but not memory
//! writes — is the gap this loop is the first shipped control in.

use crate::auth::policy::PrincipalKind;
use crate::workflow::create::verify::ClaimUnderTest;

/// The switch. A constant, so there is nothing to configure.
///
/// If a future round flips this, that round owes a published out-of-sample
/// false-promotion figure and a named owner, and it owes a pin proving that no
/// OTHER round flipped it. A boolean that a build can carry but an operator
/// cannot reach is the only shape that keeps the invariant auditable.
pub(crate) const PROMOTION_ENABLED: bool = false;

/// How long a promotion token stays usable. Short, because the window between
/// "the reviewer looked" and "the reviewer clicked" is the window in which the
/// request under review can change underneath them.
pub(crate) const TOKEN_TTL_SECS: i64 = 900;

/// What the promotion attempt decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PromotionOutcome {
    /// Refused, always, in every configuration.
    Disabled { claim_id: String },
    /// Refused because the actor is not a human principal.
    NotHuman { claim_id: String },
    /// Refused because the token is absent, spent, or expired.
    TokenRejected { claim_id: String },
    /// The token is valid and the act is authorized. Unreachable while the
    /// switch is false, and the match arm keeps that unreachable.
    Authorized {
        claim_id: String,
        promoted_by: String,
    },
}

impl PromotionOutcome {
    /// The wire spelling. `promotion_disabled` is the typed refusal the loop
    /// returns in every configuration today.
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            PromotionOutcome::Disabled { .. } => "promotion_disabled",
            PromotionOutcome::NotHuman { .. } => "not_human",
            PromotionOutcome::TokenRejected { .. } => "token_rejected",
            PromotionOutcome::Authorized { .. } => "authorized",
        }
    }

    pub(crate) fn claim_id(&self) -> &str {
        match self {
            PromotionOutcome::Disabled { claim_id }
            | PromotionOutcome::NotHuman { claim_id }
            | PromotionOutcome::TokenRejected { claim_id }
            | PromotionOutcome::Authorized { claim_id, .. } => claim_id,
        }
    }
}

/// A single-use, expiring, digest-bound promotion token.
///
/// Deliberately not `Clone` and not `Copy`: a token that could be duplicated
/// would be usable twice, and single-use is the property that makes an
/// expiring window survivable.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PromotionToken {
    pub(crate) claim_id: String,
    /// Bound to the evidence digest at mint time. Refused if the evidence
    /// moved, so a token cannot promote a claim that was reviewed and then
    /// edited underneath the reviewer.
    pub(crate) evidence_digest: String,
    pub(crate) issued_at: i64,
    pub(crate) spent: bool,
}

impl PromotionToken {
    /// Mint a token bound to the claim's CURRENT evidence digest.
    pub(crate) fn mint(claim: &ClaimUnderTest, issued_at: i64) -> PromotionToken {
        PromotionToken {
            claim_id: claim.claim_id.clone(),
            evidence_digest: evidence_digest(claim),
            issued_at,
            spent: false,
        }
    }

    /// Is this token still usable against this claim, at this instant?
    ///
    /// Three independent failures, all checked: spent, expired, and the
    /// evidence moved. Expiry is inclusive-bounded at the far end so a token
    /// cannot live for a nanosecond longer than promised by an off-by-one.
    pub(crate) fn is_usable(&self, claim: &ClaimUnderTest, now: i64) -> bool {
        if self.spent {
            return false;
        }
        if now < self.issued_at {
            return false;
        }
        if now - self.issued_at > TOKEN_TTL_SECS {
            return false;
        }
        self.claim_id == claim.claim_id && self.evidence_digest == evidence_digest(claim)
    }
}

/// The digest of everything a reviewer was shown: the claim's own fields plus
/// every citation's exact bytes and range.
///
/// It covers the citations' content, not merely their identities, so an
/// edit to a quoted byte range moves the digest and invalidates the token.
/// That is the whole point of binding: a reviewer approves what they read.
pub(crate) fn evidence_digest(claim: &ClaimUnderTest) -> String {
    let mut acc = String::with_capacity(256);
    acc.push_str(&claim.claim_id);
    acc.push('\u{1f}');
    acc.push_str(&claim.subject);
    acc.push('\u{1f}');
    acc.push_str(&claim.predicate);
    acc.push('\u{1f}');
    acc.push_str(&format!("{:?}", claim.value));
    for (citation, source) in claim.citations.iter().zip(claim.sources.iter()) {
        acc.push('\u{1f}');
        acc.push_str(&citation.source_cid);
        acc.push(':');
        acc.push_str(&citation.byte_start.to_string());
        acc.push(':');
        acc.push_str(&citation.byte_end.to_string());
        acc.push(':');
        // The QUOTE as well as the bytes it claims to come from. The gate
        // already refuses a quote that does not match its range, so the two
        // agree — but the reviewer is shown the quote, and a token must bind
        // what a human read rather than only what the bytes behind it say.
        acc.push_str(&crate::audit::hash(&citation.quote));
        acc.push(':');
        acc.push_str(&crate::audit::hash(&String::from_utf8_lossy(source)));
    }
    crate::audit::hash(&acc)
}

/// Attempt the promotion. Always refuses while the switch is false.
///
/// The order of the checks is deliberate and load-bearing: the disabled check
/// comes FIRST, so the refusal a caller receives in this round cannot vary with
/// the actor or the token. A route that returned `not_human` to an agent and
/// `promotion_disabled` to an operator would be reporting on the probe rather
/// than on the state of the loop.
pub(crate) fn promote(
    claim: &ClaimUnderTest,
    actor: &PrincipalKind,
    actor_sub: &str,
    token: Option<&PromotionToken>,
    now: i64,
) -> PromotionOutcome {
    if !PROMOTION_ENABLED {
        return PromotionOutcome::Disabled {
            claim_id: claim.claim_id.clone(),
        };
    }
    if *actor != PrincipalKind::Jwt {
        return PromotionOutcome::NotHuman {
            claim_id: claim.claim_id.clone(),
        };
    }
    match token {
        Some(t) if t.is_usable(claim, now) => PromotionOutcome::Authorized {
            claim_id: claim.claim_id.clone(),
            promoted_by: actor_sub.to_string(),
        },
        _ => PromotionOutcome::TokenRejected {
            claim_id: claim.claim_id.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::create::verify::{Citation, SlotType, SlotValue};

    fn claim() -> ClaimUnderTest {
        let source = b"the warranty runs for two years".to_vec();
        let cid = brain_evidence_core::cid_v1(&source);
        ClaimUnderTest {
            claim_id: "clm_1".into(),
            subject: "acme".into(),
            predicate: "warranty_months".into(),
            value: SlotValue::Integer(24),
            declared_type: SlotType::Integer,
            qualifiers: vec![],
            contradicts: vec![],
            citations: vec![Citation {
                source_cid: cid,
                byte_start: 0,
                byte_end: 21,
                quote: "the warranty runs for".into(),
            }],
            sources: vec![source],
            ratified: vec![],
        }
    }

    #[test]
    fn the_promotion_switch_is_a_constant_nothing_can_reach() {
        // Read through a black-boxable value so the assertion is about the
        // constant's VALUE rather than a literal in this test.
        let enabled: bool = std::hint::black_box(PROMOTION_ENABLED);
        assert!(
            !enabled,
            "the loop ships inert; a true here without a published out-of-sample \
             false-promotion figure and a named owner breaks the round's central invariant"
        );
        let source = include_str!("promote.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or_default();
        for token in ["env::var", "std::env", "BRAIN_", "var_os", "getenv"] {
            assert!(
                !production.contains(token),
                "the promotion module reaches `{token}`. There must be no configuration \
                 path that enables promotion."
            );
        }
    }

    #[test]
    fn every_configuration_receives_the_same_disabled_refusal() {
        // Actor, token presence and token validity must NOT change the
        // refusal: a route that answered differently per probe would be
        // reporting on the probe.
        for actor in [PrincipalKind::Jwt, PrincipalKind::AgentLoopback] {
            for token in [None, Some(PromotionToken::mint(&claim(), 0))] {
                let outcome = promote(&claim(), &actor, "whoever", token.as_ref(), 1);
                assert_eq!(
                    outcome,
                    PromotionOutcome::Disabled {
                        claim_id: "clm_1".into()
                    },
                    "the disabled refusal must not vary with the actor or the token"
                );
                assert_eq!(outcome.code(), "promotion_disabled");
            }
        }
    }

    #[test]
    fn a_token_is_single_use_and_expires() {
        let c = claim();
        let mut token = PromotionToken::mint(&c, 1_000);
        assert!(token.is_usable(&c, 1_000), "a fresh token is usable");
        assert!(
            token.is_usable(&c, 1_000 + TOKEN_TTL_SECS),
            "usable at the edge"
        );
        assert!(
            !token.is_usable(&c, 1_000 + TOKEN_TTL_SECS + 1),
            "a token one second past its life is dead"
        );
        token.spent = true;
        assert!(!token.is_usable(&c, 1_000), "a spent token is dead");
        // A token minted in the future is not usable in the past.
        assert!(
            !PromotionToken::mint(&c, 5_000).is_usable(&c, 1_000),
            "clock skew must not buy a token extra life"
        );
    }

    #[test]
    fn a_token_is_refused_when_any_evidence_byte_moves() {
        let c = claim();
        let token = PromotionToken::mint(&c, 0);
        let mut edited = c.clone();
        // A one-byte change to the cited range.
        edited.citations[0].byte_end = 20;
        assert!(
            !token.is_usable(&edited, 0),
            "the token must be refused when a cited range moved: a reviewer approves what \
             they read, so what they read is what the token binds"
        );
        let mut rewritten = c.clone();
        rewritten.sources[0][0] = b'x';
        assert!(
            !token.is_usable(&rewritten, 0),
            "the token must be refused when a cited byte changed"
        );
        let mut requoted = c.clone();
        requoted.citations[0].quote = "the warranty runs".into();
        assert!(
            !token.is_usable(&requoted, 0),
            "the token must be refused when the quote changed"
        );
    }

    #[test]
    fn the_evidence_digest_covers_the_claim_not_only_its_citations() {
        let c = claim();
        let base = evidence_digest(&c);
        let mut moved = c.clone();
        moved.value = SlotValue::Integer(36);
        assert_ne!(
            base,
            evidence_digest(&moved),
            "the object is part of what is signed"
        );
        let mut renamed = c.clone();
        renamed.subject = "other".into();
        assert_ne!(
            base,
            evidence_digest(&renamed),
            "the subject is part of what is signed"
        );
        assert_eq!(base.len(), 64);
    }
}
