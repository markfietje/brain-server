//! The `ci` read adapter: GitHub Actions workflow runs for a bound repository.
//!
//! **READ-ONLY, and structurally so.** The only verb used is `GET`. A CI run's
//! conclusion is an OBSERVATION about the world, never an instruction to the
//! world, and this file has no path that could turn one into the other.
//!
//! **A run's name, branch, and conclusion are all untrusted input.** They come
//! from a third-party system that anyone with push access can influence, so
//! they are treated the way the read seam treats stored text: bounded,
//! stripped, and never trusted ahead of reconciliation. A conclusion this
//! adapter does not recognise is reported as `unrecognised`, never coerced
//! into a known value — a coerced conclusion is an invented fact, and this
//! loop's entire premise is that the ledger believes only what an authority
//! actually said.
//!
//! Egress, the exact-host refusal, and the 3xx-as-refusal law are the `vcs`
//! adapter's, for the same reasons and by the same seam.

use super::{Binding, BindingRefused, assert_api_host, bearer_for};

/// What one CI observation carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CiFacts {
    pub target_ref: String,
    /// The workflow run's own id, as a string. It is an EXTERNAL identifier:
    /// never parsed as a number and never used as an ordering key.
    pub run_id: Option<String>,
    /// The conclusion, from the closed set below, or `unrecognised`.
    pub conclusion: String,
    pub pages_read: u8,
}

/// The conclusions this adapter will report. GitHub adds to this set over
/// time; an unknown value is reported as `unrecognised` rather than being
/// mapped onto a known one, because "we saw a conclusion we do not model" and
/// "the run failed" are different facts and only one of them is actionable.
const CONCLUSIONS: &[&str] = &["success", "failure", "cancelled", "skipped", "timed_out"];

/// The reported value for a conclusion outside the modelled set.
const UNRECOGNISED: &str = "unrecognised";

/// Read the CI facts for one binding.
pub(crate) async fn fetch_ci_facts(
    binding: &Binding,
    secret_root: &std::path::Path,
) -> Result<CiFacts, BindingRefused> {
    assert_api_host(&binding.endpoint)?;
    let bearer = bearer_for(binding, secret_root)?;

    let page_cap = binding.capabilities.pages();
    let client = crate::webhook::egress_client_for_url(&binding.endpoint)
        .map_err(|_| BindingRefused::HostRefused)?;

    let url = format!(
        "{}/repos/{}/actions/runs?per_page=1",
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

    // The 3xx arm: under `Policy::none()` a redirect arrives as `Ok`, so this
    // is the only place it can be caught, and it is caught as a refusal.
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

    let runs = body
        .get("workflow_runs")
        .and_then(serde_json::Value::as_array);
    // The page bound is ENFORCED against the RESPONSE, using the binding's own
    // ceiling. A response carrying more runs than the binding allows means the
    // authority was not bounded by what we asked; following it would be
    // unbounded consumption with a network bill attached.
    let per_page = 1usize.min(page_cap as usize);
    if runs.is_some_and(|r| r.len() > per_page) {
        return Err(BindingRefused::Unbounded);
    }
    let run = runs.and_then(|runs| runs.first());

    let run_id = run
        .and_then(|r| r.get("id"))
        // An id that is not a plain integer string is not an id.
        .and_then(serde_json::Value::as_i64)
        .map(|n| n.to_string());

    let conclusion = run
        .and_then(|r| r.get("conclusion"))
        .and_then(serde_json::Value::as_str)
        .filter(|c| CONCLUSIONS.contains(c))
        .unwrap_or(UNRECOGNISED)
        .to_string();

    Ok(CiFacts {
        target_ref: binding.target_ref.clone(),
        run_id,
        conclusion,
        pages_read: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unmodelled_conclusion_is_unrecognised_and_never_coerced() {
        // The set the adapter models...
        assert!(CONCLUSIONS.contains(&"success"));
        assert!(CONCLUSIONS.contains(&"timed_out"));
        // ...and the value it reports for anything else. It is NOT "failure":
        // an unmodelled conclusion is a different fact from a failing run, and
        // only one of the two is actionable.
        assert!(!CONCLUSIONS.contains(&UNRECOGNISED));
        assert_ne!(UNRECOGNISED, "failure");
    }

    #[tokio::test]
    async fn a_binding_for_an_unreachable_host_refuses_before_any_request() {
        let binding = Binding {
            id: 1,
            domain: "personal".to_string(),
            target_kind: "ci".to_string(),
            target_ref: "acme/repo".to_string(),
            endpoint: "https://evil.test".to_string(),
            authority_digest: "sha256:aa".to_string(),
            capabilities: super::super::Capabilities::default_read_only(),
            secret_file_name: "gh.token".to_string(),
        };
        let err = fetch_ci_facts(&binding, std::path::Path::new("/nonexistent-root"))
            .await
            .expect_err("a foreign host is refused");
        assert_eq!(err, BindingRefused::HostRefused);
    }
}
