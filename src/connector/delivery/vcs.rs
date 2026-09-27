//! The `vcs` read adapter: commits, refs, and combined statuses for a bound
//! repository.
//!
//! **READ-ONLY, and structurally so.** There is no write verb in this file, no
//! `POST`/`PATCH`/`DELETE`, and no retry that mutates. An observation becomes
//! evidence; it never becomes an instruction.
//!
//! **Egress.** Every request goes through the shared family's
//! `egress_client_for_url`, which resolves, validates the resolved addresses
//! against the IANA special-purpose tables, and pins DNS insert-only. This
//! module builds NO client of its own — a private builder would skip all three
//! of those steps.
//!
//! **A 3xx is a refusal, not an empty page.** `Policy::none()` means reqwest
//! RETURNS the redirect as a successful result. Consuming one as "no results"
//! would let a `Location` header decide what the machine believes, and would
//! hand the bearer to whatever host that header names. The exact-host refusal
//! is checked again on every paginated URL, because the `next` link comes from
//! the response body and is therefore untrusted input.

use super::{Binding, BindingRefused, assert_api_host, bearer_for};

/// The facts one observation carries. A digest over these, never the raw
/// response: the response is untrusted input and may contain anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VcsFacts {
    pub target_ref: String,
    /// The head commit sha the authority reports, if it reports one.
    pub head_sha: Option<String>,
    /// The combined status, from the authority's own closed set.
    pub combined_status: String,
    /// How many pages were actually read. Bounded by the binding's ceiling.
    pub pages_read: u8,
}

/// The combined status set this adapter will accept from the authority.
/// Anything else is treated as an unrecognised observation rather than being
/// coerced into a known value — a coerced value is an invented fact.
const COMBINED_STATUSES: &[&str] = &["success", "failure", "pending", "error", "unknown"];

/// Read the authority facts for one binding.
///
/// `secret_root` is the configured, root-confined directory. The bearer is
/// read at the call, used for the request, and dropped before this returns.
pub(crate) async fn fetch_vcs_facts(
    binding: &Binding,
    secret_root: &std::path::Path,
) -> Result<VcsFacts, BindingRefused> {
    // The endpoint is validated by the frame at resolve time; re-checking here
    // costs nothing and means a Binding constructed by a future caller cannot
    // skip it.
    assert_api_host(&binding.endpoint)?;
    let bearer = bearer_for(binding, secret_root)?;

    let page_cap = binding.capabilities.pages();
    let client = crate::webhook::egress_client_for_url(&binding.endpoint)
        .map_err(|_| BindingRefused::HostRefused)?;

    // One page this round. The page CEILING is enforced and the loop is
    // deliberately absent: pagination follows a server-supplied `next` URL,
    // and following it unboundedly is an unbounded-consumption bug with a
    // network bill attached. When the loop lands it will be bounded by
    // `page_cap` and re-check the host on every hop.
    let url = format!(
        "{}/repos/{}/commits?per_page=1",
        binding.endpoint, binding.target_ref
    );
    assert_api_host(&url)?;
    let response = client
        .get(&url)
        .header("authorization", format!("Bearer {bearer}"))
        .header("x-github-api-version", "2022-11-28")
        .header("user-agent", "brain-server-delivery")
        .send()
        .await
        .map_err(|_| BindingRefused::NotSuccess)?;

    // The 3xx arm. `Policy::none()` returns the redirect as `Ok`, so this is
    // the ONLY place a 30x can be caught — and it is caught as a refusal.
    if response.status().is_redirection() {
        return Err(BindingRefused::NotSuccess);
    }
    if !response.status().is_success() {
        return Err(BindingRefused::NotSuccess);
    }
    let body = response
        .json::<serde_json::Value>()
        .await
        .map_err(|_| BindingRefused::NotSuccess)?;

    // The response is UNTRUSTED. Nothing from it is used except as a string
    // that is then bounded, and no number is taken from it as a fact without
    // a shape check first.
    // The page bound is ENFORCED against the RESPONSE, using the binding's own
    // ceiling: an authority that returned more items than the binding allows
    // is one the machine did not bound, and following it would be unbounded
    // consumption with a network bill attached.
    let per_page = 1usize.min(page_cap as usize);
    if body
        .as_array()
        .is_some_and(|commits| commits.len() > per_page)
    {
        return Err(BindingRefused::Unbounded);
    }
    let head_sha = body
        .as_array()
        .and_then(|commits| commits.first())
        .and_then(|c| c.get("sha"))
        .and_then(serde_json::Value::as_str)
        .filter(|sha| sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_string);

    // The status is read from the authority's own vocabulary and mapped only
    // into the set this adapter models; anything else stays `unknown`.
    let combined_status = match body
        .get("state")
        .and_then(serde_json::Value::as_str)
        .filter(|s| COMBINED_STATUSES.contains(s))
    {
        Some("success") => "success",
        Some("failure") | Some("error") => "failure",
        Some("pending") => "pending",
        _ => "unknown",
    };
    Ok(VcsFacts {
        target_ref: binding.target_ref.clone(),
        head_sha,
        combined_status: combined_status.to_string(),
        pages_read: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_observed_status_set_is_closed() {
        // The adapter will only ever report one of these; anything else the
        // authority says is an unrecognised observation, not coerced into one.
        assert!(COMBINED_STATUSES.contains(&"success"));
        assert!(COMBINED_STATUSES.contains(&"failure"));
        assert!(!COMBINED_STATUSES.contains(&"definitely-fine"));
    }

    #[tokio::test]
    async fn a_binding_for_an_unreachable_host_refuses_before_any_request() {
        // The refusal happens at the frame, before a bearer is read and before
        // a socket is opened.
        let mut binding = Binding {
            id: 1,
            domain: "personal".to_string(),
            target_kind: "vcs".to_string(),
            target_ref: "acme/repo".to_string(),
            endpoint: "https://evil.test".to_string(),
            authority_digest: "sha256:aa".to_string(),
            capabilities: super::super::Capabilities::default_read_only(),
            secret_file_name: "gh.token".to_string(),
        };
        let err = fetch_vcs_facts(&binding, std::path::Path::new("/nonexistent-root"))
            .await
            .expect_err("a foreign host is refused");
        assert_eq!(err, BindingRefused::HostRefused);

        // A well-formed binding with a missing secret refuses with the
        // SECRET code, not the host code — the two are different operator
        // problems and must not collapse into one.
        binding.endpoint = "https://api.github.com".to_string();
        let err = fetch_vcs_facts(&binding, std::path::Path::new("/nonexistent-root"))
            .await
            .expect_err("an unreadable secret is refused");
        assert!(matches!(err, BindingRefused::Secret(_)));
        assert_eq!(err.to_string(), "secret_unavailable");
    }
}
