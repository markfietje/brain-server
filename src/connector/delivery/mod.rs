//! The delivery adapter frame: binding resolution, the typed capability schema,
//! the exact-host refusal, and the shared shape both read adapters fill in.
//!
//! An AUTHORITY BINDING is the machine's standing permission to read one
//! external system on behalf of one tenant. Three properties follow from that
//! and are enforced here rather than at each call site:
//!
//! * **Domain-scoped.** `domain` is not decoration. A binding resolves only
//!   within the domain that was asked about, so a lookup cannot become a
//!   cross-tenant authority leak by forgetting a `WHERE`.
//! * **Read-only.** Both adapters read. There is no write verb, no retry that
//!   mutates, and no code path that turns an observation into an instruction.
//! * **Configured, never requested.** A binding names an endpoint and a secret
//!   FILE NAME, both of which come from server-owned config at boot. There is
//!   no write route and no request body that could widen an authority.
//!
//! **The secret never becomes a digest input.** [`authority_digest`] covers the
//! endpoint, the stable external ref, and the secret's file NAME — which
//! credential slot is bound. It never covers the secret (a low-entropy one is
//! brute-forceable out of a hash column; a high-entropy one is a bearer that
//! can never be rotated) and never the path (host layout is not authority).
//!
//! **The exact-host refusal is re-implemented here, not imported.** The shipped
//! `assert_api_host` in the GitHub connector is a private function behind
//! `#![cfg(feature = "connector-github")]`, so it is genuinely unreachable from
//! here. That makes this copy load-bearing rather than redundant: GitHub's
//! `Link` header can hand back an attacker-chosen `next` URL, and the shared
//! egress family validates an ADDRESS SET, not an identity. A 3xx arrives as
//! `Ok` under `Policy::none()`, so the adapters treat one as a refusal rather
//! than parsing it as an empty body.
//!
//! What this frame does not do: decide whether an observation may be acted on,
//! retain it, or publish it. A mismatch becomes typed evidence and a human
//! decides.

#![deny(dead_code)]
// The `allow(dead_code)` in the parent module is a LOAN to the connector
// library, not a hole: an `allow` is not a `forbid`, so this deeper `deny`
// wins for this subtree. It does not reach `pub` items in a library target
// (rustc never dead-code-lints those), which is exactly why the production-
// caller pins exist alongside it.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::secret_file::{ProviderSecretError, read_provider_secret};

pub mod ci;
pub mod vcs;

/// The only host a binding's bearer may ever be sent to.
pub(crate) const GITHUB_API_HOST: &str = "api.github.com";

/// The closed `target_kind` vocabulary. Two of these six have an adapter
/// behind them; the other four are declared and CONSUMER-LESS on purpose, so
/// that adding one is a deliberate act with code behind it rather than a
/// vocabulary that grows by accident. The table's CHECK enforces the same set
/// — this is the reader-side half, for a row that arrived from a future
/// writer or a hand-edited database.
pub(crate) const TARGET_KINDS: &[&str] = &["vcs", "ci", "registry", "deploy", "pm", "incident"];

/// The kinds this round can actually read. The rest are refused at the
/// adapter boundary with a named code, not silently treated as readable.
pub(crate) const ADAPTED_KINDS: &[&str] = &["vcs", "ci"];

/// Bounded caller/config text. A `target_ref` is an external identifier
/// (`owner/repo`); a `domain` is a validated identifier. Both are bounded so a
/// hostile configuration cannot turn a lookup key into an unbounded read.
const MAX_TARGET_REF: usize = 200;
const MAX_DOMAIN: usize = 100;
/// The page bound. An adapter that follows pagination without a ceiling is an
/// unbounded-consumption bug with a network bill attached.
const MAX_PAGES_DEFAULT: u8 = 4;
const MAX_PAGES_CEILING: u8 = 16;
/// The list cap. A domain with more bindings than this is an operator
/// configuration problem the response names, not a response that grows without
/// bound.
const MAX_LIST: usize = 64;

/// The closed capability vocabulary. A capability the operator set that the
/// machine does not enforce is worse than one they did not set, so an unknown
/// one is refused rather than ignored.
pub(crate) const CAPABILITIES: &[&str] =
    &["commits", "refs", "statuses", "workflow_runs", "reconcile"];

/// The typed capability schema. `deny_unknown_fields` is the whole point: a
/// silently-ignored field is a permission the operator granted that nothing
/// enforces. This is deliberately NOT the `agent_cards` `is_object()`-only
/// check, which accepts any JSON object at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Capabilities {
    /// What the adapter may read. Closed set.
    pub read: Vec<String>,
    /// What the adapter may be asked to intend. The intents are QUEUED this
    /// round and never dispatched; the list is the declared surface.
    pub intents: Vec<String>,
    /// The page ceiling for this binding.
    pub max_pages: u8,
}

impl Capabilities {
    /// Parse-with-refusal. A malformed profile is a REFUSED binding, never an
    /// unconstrained one — so this returns a `Result` and never degrades to a
    /// default.
    pub(crate) fn parse(raw: &str) -> Result<Self, BindingRefused> {
        let parsed: Self =
            serde_json::from_str(raw).map_err(|_| BindingRefused::MalformedCapabilities)?;
        for value in parsed.read.iter().chain(parsed.intents.iter()) {
            if !CAPABILITIES.contains(&value.as_str()) {
                return Err(BindingRefused::UnknownCapability);
            }
        }
        if parsed.max_pages == 0 || parsed.max_pages > MAX_PAGES_CEILING {
            return Err(BindingRefused::MalformedCapabilities);
        }
        Ok(parsed)
    }

    /// The effective page bound, clamped to the hard ceiling. The column is
    /// operator input; the ceiling is not.
    pub(crate) fn pages(&self) -> u8 {
        self.max_pages.clamp(1, MAX_PAGES_CEILING)
    }

    /// The default for a freshly provisioned read-only binding.
    pub(crate) fn default_read_only() -> Self {
        Self {
            read: vec![
                "commits".to_string(),
                "refs".to_string(),
                "statuses".to_string(),
            ],
            intents: vec![],
            max_pages: MAX_PAGES_DEFAULT,
        }
    }
}

/// One resolved binding, as the adapters consume it. Secret-free by
/// construction: the bearer is read at the call and never stored here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub id: i64,
    pub domain: String,
    pub target_kind: String,
    pub target_ref: String,
    pub endpoint: String,
    pub authority_digest: String,
    pub capabilities: Capabilities,
    /// The secret's FILE NAME, resolved against the configured root. The path
    /// is never assembled into an error, a log line, or a digest.
    pub secret_file_name: String,
}

/// The closed refusal vocabulary. Every variant carries NO secret, NO path, and
/// NO response body — a stable code is what reaches a log line, an audit row,
/// and a gap proposal. A refusal that quoted its input would leak the bearer
/// it exists to protect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindingRefused {
    /// No active binding for this (domain, kind). Probe-blind: an absent
    /// binding and a foreign-domain binding are the same answer.
    NotFound,
    /// The kind is outside the closed vocabulary.
    UnknownKind,
    /// The kind is declared but has no adapter yet. Disclosed, not an error to
    /// paper over: four of the six are consumer-less this round.
    NotAdapted,
    /// The capabilities block did not parse, or parsed to something invalid.
    MalformedCapabilities,
    /// A capability outside the closed set was named.
    UnknownCapability,
    /// The endpoint is not the one host a binding may name.
    HostRefused,
    /// The response was not a success. A 3xx lands here too: under
    /// `Policy::none()` reqwest RETURNS the redirect as `Ok`.
    NotSuccess,
    /// The bearer could not be read. The cause is the reader's own closed code
    /// and never its path.
    Secret(ProviderSecretError),
    /// The store refused.
    Store,
    /// The response was larger or more paged than the binding's bound allows.
    Unbounded,
}

impl std::fmt::Display for BindingRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately the variant name and nothing else. No URL, no path, no
        // body, no status code that could carry a `Location` header.
        let code = match self {
            Self::NotFound => "not_found",
            Self::UnknownKind => "unknown_kind",
            Self::NotAdapted => "not_adapted",
            Self::MalformedCapabilities => "malformed_capabilities",
            Self::UnknownCapability => "unknown_capability",
            Self::HostRefused => "host_refused",
            Self::NotSuccess => "not_success",
            Self::Secret(_) => "secret_unavailable",
            Self::Store => "store",
            Self::Unbounded => "unbounded",
        };
        f.write_str(code)
    }
}

/// The authority digest: sha256 over endpoint + target_ref + the secret's FILE
/// NAME.
///
/// It records WHICH authority is bound and WHICH credential slot signs for it,
/// so a change to either is visible as a changed binding. It deliberately does
/// NOT record the secret — see the module header — and not the path, which is
/// host layout rather than authority.
pub(crate) fn authority_digest(endpoint: &str, target_ref: &str, secret_file_name: &str) -> String {
    let mut hasher = Sha256::new();
    // Length-prefixed so no two different triples can produce the same byte
    // string: without it, ("ab", "c") and ("a", "bc") collide.
    for part in [endpoint, target_ref, secret_file_name] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("sha256:{}", crate::audit::hex_encode(&hasher.finalize()))
}

/// The exact-host refusal, re-implemented (see the module header). Scheme AND
/// host must match: a substring test would admit `api.github.com.evil.test`.
pub(crate) fn assert_api_host(url: &str) -> Result<(), BindingRefused> {
    let Ok(parsed) = url::Url::parse(url) else {
        return Err(BindingRefused::HostRefused);
    };
    if parsed.scheme() == "https" && parsed.host_str() == Some(GITHUB_API_HOST) {
        return Ok(());
    }
    Err(BindingRefused::HostRefused)
}

/// Resolve the active binding for ONE domain and kind. The domain is a
/// parameter, never an optional filter: an unscoped resolve is a cross-tenant
/// authority leak, and making it structurally impossible to omit is cheaper
/// than auditing every call site for the omission.
pub(crate) fn resolve_binding(
    conn: &Connection,
    domain: &str,
    target_kind: &str,
) -> Result<Binding, BindingRefused> {
    if !TARGET_KINDS.contains(&target_kind) {
        return Err(BindingRefused::UnknownKind);
    }
    if !ADAPTED_KINDS.contains(&target_kind) {
        return Err(BindingRefused::NotAdapted);
    }
    if domain.is_empty() || domain.chars().count() > MAX_DOMAIN {
        return Err(BindingRefused::NotFound);
    }
    let row = conn
        .query_row(
            "SELECT id, domain, target_kind, target_ref, endpoint, authority_digest, \
                    capabilities_json, secret_file_name \
               FROM delivery_bindings \
              WHERE domain = ?1 AND target_kind = ?2 AND active = 1 \
              ORDER BY id LIMIT 1",
            params![domain, target_kind],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .map_err(|_| BindingRefused::Store)?;
    let Some((id, domain, target_kind, target_ref, endpoint, digest, caps, file)) = row else {
        return Err(BindingRefused::NotFound);
    };
    if target_ref.is_empty() || target_ref.chars().count() > MAX_TARGET_REF {
        return Err(BindingRefused::MalformedCapabilities);
    }
    if assert_api_host(&endpoint).is_err() {
        return Err(BindingRefused::HostRefused);
    }
    let capabilities = Capabilities::parse(&caps)?;
    Ok(Binding {
        id,
        domain,
        target_kind,
        target_ref,
        endpoint,
        authority_digest: digest,
        capabilities,
        secret_file_name: file,
    })
}

/// The emitted shape of one binding. It carries NO secret file name: the
/// operator can see which authority is bound, and cannot use this surface to
/// learn where its credential lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct BindingView {
    pub id: i64,
    pub domain: String,
    pub target_kind: String,
    pub target_ref: String,
    pub endpoint: String,
    pub authority_digest: String,
    pub capabilities: String,
    pub active: i64,
    pub updated_at: i64,
}

/// The read surface: every binding in ONE domain, with no secret material in
/// the projection. The read route serves exactly this, which is why a
/// secret-file reference can never ride a request — there is no request to
/// ride.
pub(crate) fn list_bindings(
    conn: &Connection,
    domain: &str,
) -> Result<Vec<BindingView>, BindingRefused> {
    let mut stmt = conn
        .prepare(
            "SELECT id, domain, target_kind, target_ref, endpoint, authority_digest, \
                    capabilities_json, active, updated_at \
               FROM delivery_bindings WHERE domain = ?1 ORDER BY target_kind, target_ref",
        )
        .map_err(|_| BindingRefused::Store)?;
    let rows = stmt
        .query_map(params![domain], |r| {
            let raw: String = r.get(6)?;
            Ok(BindingView {
                id: r.get(0)?,
                domain: r.get(1)?,
                target_kind: r.get(2)?,
                target_ref: r.get(3)?,
                endpoint: r.get(4)?,
                authority_digest: r.get(5)?,
                // A malformed capabilities block renders as an explicit marker
                // rather than as a default that reads as "unconstrained".
                capabilities: Capabilities::parse(&raw)
                    .ok()
                    .and_then(|c| serde_json::to_string(&c).ok())
                    .unwrap_or_else(|| "\"unparseable\"".to_string()),
                active: r.get::<_, i64>(7)?,
                updated_at: r.get(8)?,
            })
        })
        .map_err(|_| BindingRefused::Store)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|_| BindingRefused::Store)?);
    }
    out.truncate(MAX_LIST);
    Ok(out)
}

/// The undrained-intent census for `/metrics`: how many `delivery/%` rows sit
/// `pending` with no reader, split by what the READ SIDE makes of them.
///
/// The split is the point. An intent that was LOST and an intent that has not
/// yet been promoted are otherwise indistinguishable, and a FORGED row — one
/// that carries a delivery topic but not a kernel-minted key — is
/// indistinguishable from a real one. So the gauge classifies through
/// [`intent_kind`], which means the degradation is visible at the ops surface
/// rather than being a property only a test can see.
pub(crate) fn pending_intent_census(
    conn: &Connection,
    domain: &str,
) -> Result<PendingIntents, BindingRefused> {
    let mut stmt = conn
        .prepare(
            "SELECT o.topic, o.idempotency_key FROM outbox o \
               JOIN workflow_runs r ON r.id = o.run_id \
              WHERE o.status = 'pending' AND o.topic LIKE 'delivery/%' AND r.domain = ?1",
        )
        .map_err(|_| BindingRefused::Store)?;
    let rows = stmt
        .query_map(params![domain], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| BindingRefused::Store)?;
    let mut census = PendingIntents::default();
    for row in rows {
        let (topic, key) = row.map_err(|_| BindingRefused::Store)?;
        match crate::workflow::delivery_intents::intent_kind(&topic, &key) {
            crate::workflow::delivery_intents::IntentKind::Intent => census.intents += 1,
            crate::workflow::delivery_intents::IntentKind::Observed => census.observed += 1,
            crate::workflow::delivery_intents::IntentKind::Untrusted => census.untrusted += 1,
            crate::workflow::delivery_intents::IntentKind::NotDelivery => {}
        }
    }
    Ok(census)
}

/// The three counters. All three are EXPECTED to be non-zero in normal
/// operation this release; the gauge exists to make a lost row distinguishable
/// from an un-promoted one, not to alarm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PendingIntents {
    pub intents: i64,
    pub observed: i64,
    /// Rows carrying a delivery topic whose key is NOT a kernel mint. Any
    /// non-zero value is worth an operator's attention: it means something
    /// wrote the reserved root without going through the minter.
    pub untrusted: i64,
}

/// Read one binding's bearer, at the call, through the root-confined reader.
/// The value lives in this binding's scope and is dropped when the adapter
/// returns — it is never stored on a struct, cached, or logged.
pub(crate) fn bearer_for(
    binding: &Binding,
    secret_root: &std::path::Path,
) -> Result<String, BindingRefused> {
    read_provider_secret(secret_root, std::path::Path::new(&binding.secret_file_name))
        .map_err(BindingRefused::Secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_covers_the_descriptor_and_never_the_secret() {
        let a = authority_digest("https://api.github.com", "acme/repo", "gh.token");
        let b = authority_digest("https://api.github.com", "acme/repo", "gh.token");
        assert_eq!(a, b, "the same descriptor digests the same");
        // A different endpoint, ref, or credential SLOT is a different authority.
        assert_ne!(
            a,
            authority_digest("https://api.github.com", "acme/other", "gh.token")
        );
        assert_ne!(
            a,
            authority_digest("https://api.github.com", "acme/repo", "gh.other")
        );
        assert!(a.starts_with("sha256:"));
    }

    #[test]
    fn the_digest_is_length_prefixed_so_triples_cannot_collide() {
        assert_ne!(
            authority_digest("ab", "c", "d"),
            authority_digest("a", "bc", "d"),
            "without length framing, concatenating the parts makes different authorities \
             digest identically"
        );
    }

    #[test]
    fn the_exact_host_refusal_admits_only_the_api_host() {
        assert!(assert_api_host("https://api.github.com/repos/acme/repo").is_ok());
        // The suffix and prefix attacks a substring test would admit.
        assert!(assert_api_host("https://api.github.com.evil.test/x").is_err());
        assert!(assert_api_host("https://evil.test/api.github.com").is_err());
        assert!(assert_api_host("http://api.github.com/x").is_err());
        assert!(assert_api_host("not a url").is_err());
    }

    #[test]
    fn capabilities_refuse_an_unknown_field_and_an_unknown_value() {
        let ok = Capabilities::parse(r#"{"read":["commits"],"intents":[],"max_pages":2}"#);
        assert!(ok.is_ok(), "a well-formed profile parses: {ok:?}");
        // deny_unknown_fields: an operator-set field nothing enforces.
        assert!(
            Capabilities::parse(r#"{"read":[],"intents":[],"max_pages":1,"admin":true}"#).is_err()
        );
        // An unknown capability VALUE is refused too, not ignored.
        assert!(
            Capabilities::parse(r#"{"read":["delete_everything"],"intents":[],"max_pages":1}"#)
                .is_err()
        );
        // A zero or unbounded page count is refused.
        assert!(Capabilities::parse(r#"{"read":[],"intents":[],"max_pages":0}"#).is_err());
        assert!(Capabilities::parse(r#"{"read":[],"intents":[],"max_pages":200}"#).is_err());
        assert!(Capabilities::parse("not json").is_err());
    }

    #[test]
    fn every_refusal_code_is_stable_and_carries_nothing() {
        // A refusal is what reaches a log line and an audit row. If any of
        // these grew a payload, a bearer or a path would ride along with it.
        let codes = [
            (BindingRefused::NotFound, "not_found"),
            (BindingRefused::UnknownKind, "unknown_kind"),
            (BindingRefused::NotAdapted, "not_adapted"),
            (
                BindingRefused::MalformedCapabilities,
                "malformed_capabilities",
            ),
            (BindingRefused::UnknownCapability, "unknown_capability"),
            (BindingRefused::HostRefused, "host_refused"),
            (BindingRefused::NotSuccess, "not_success"),
            (BindingRefused::Store, "store"),
            (BindingRefused::Unbounded, "unbounded"),
        ];
        for (refusal, code) in codes {
            assert_eq!(refusal.to_string(), code);
        }
    }
}
