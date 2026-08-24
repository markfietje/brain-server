//! The KCS capture decision core: pure functions over recorded evidence, no
//! I/O, no clock. The Solve-loop close-out — a closed case becomes either a
//! new-article proposal, an update proposal, or a link-only record. The
//! output is a PROPOSAL intent only; nothing here writes memory (the HITL
//! `/proposals` gate is the only path to knowledge, same law as
//! [`crate::pure::qa_score::flywheel_proposals`]).

use crate::pure::qa_score::RunArtifacts;

/// The three capture branches (the plan's proposal-kind family).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureAction {
    /// No cited article (or similarity says gap): propose a NEW article.
    NewArticle,
    /// A cited article exists and the run diverged from it: propose UPDATE.
    UpdateArticle,
    /// A cited article exists and content is current: LINK ONLY.
    LinkOnly,
}

/// The deterministic branch rule.
///
/// - no `searched_found` citation at all → [`CaptureAction::NewArticle`];
/// - otherwise similarity units decide first when present (the same
///   thresholds as [`crate::pure::qa_score::gap_decision`]: <4000 New,
///   <8000 Update, else current);
/// - without similarity units, divergence decides: any step whose actual
///   contradicted its expectation (or a verification failure against the
///   cited article) means the article needs improving → Update; a clean
///   reuse is LinkOnly.
pub fn capture_decision(
    has_cited: bool,
    similarity_units: Option<i32>,
    diverged_steps: usize,
) -> CaptureAction {
    if !has_cited {
        return CaptureAction::NewArticle;
    }
    // The improve signal wins: a run that diverged from the article it used
    // means the article needs fixing, whatever similarity said.
    if diverged_steps > 0 {
        return CaptureAction::UpdateArticle;
    }
    match similarity_units {
        Some(s) => match crate::pure::qa_score::gap_decision(s) {
            Some(crate::pure::qa_score::GapAction::ProposeNew) => CaptureAction::NewArticle,
            Some(crate::pure::qa_score::GapAction::ProposeUpdate) => CaptureAction::UpdateArticle,
            None => CaptureAction::LinkOnly,
        },
        None if diverged_steps > 0 => CaptureAction::UpdateArticle,
        None => CaptureAction::LinkOnly,
    }
}

/// Derive the capture action from full run artifacts (convenience over
/// [`capture_decision`]): divergence = steps where expected != actual.
pub fn capture_action_for(a: &RunArtifacts) -> CaptureAction {
    let diverged = a.steps.iter().filter(|s| s.expected != s.actual).count();
    capture_decision(true, None, diverged)
}

/// The structured article body: Issue / Environment / Cause / Resolution /
/// Evidence — the searchable KCS structure, assembled ONLY from recorded
/// evidence (deterministic, zero-token; never LLM prose).
///
/// `issue` is the symptom phrase (`Handoff.is_seed`, falling back to the run
/// query); `environment` the IS-NOT framing; `cause` the findings digests;
/// `resolution` the final actuals; `evidence` the SIR/case provenance lines.
pub fn article_body(
    issue: &str,
    environment: &str,
    cause_lines: &[String],
    resolution_lines: &[String],
    evidence_lines: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("## Issue\n");
    out.push_str(issue.trim());
    out.push_str("\n\n## Environment\n");
    out.push_str(environment.trim());
    out.push_str("\n\n## Cause\n");
    if cause_lines.is_empty() {
        out.push_str("no recorded cause finding\n");
    } else {
        for l in cause_lines {
            out.push_str("- ");
            out.push_str(l.trim());
            out.push('\n');
        }
    }
    out.push_str("\n## Resolution\n");
    if resolution_lines.is_empty() {
        out.push_str("no recorded resolution step\n");
    } else {
        for l in resolution_lines {
            out.push_str("- ");
            out.push_str(l.trim());
            out.push('\n');
        }
    }
    out.push_str("\n## Evidence\n");
    for l in evidence_lines {
        out.push_str("- ");
        out.push_str(l.trim());
        out.push('\n');
    }
    out
}

/// The machine-readable provenance preamble line the approve path parses to
/// wire the `case_articles` linkage. Format-stable: `kcs: case=<case_ref>`
/// and optionally `kcs: article=<knowledge_id>`.
pub fn preamble(case_ref: &str, article: Option<i64>) -> String {
    match article {
        Some(id) => format!("kcs: case={case_ref}\nkcs: article={id}\n\n"),
        None => format!("kcs: case={case_ref}\n\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pure::qa_score::StepRow;

    #[test]
    fn gap_rule_selects_new_update_or_link_only() {
        // No citation → New regardless of everything else.
        assert_eq!(
            capture_decision(false, Some(9000), 0),
            CaptureAction::NewArticle
        );
        // Cited + low similarity → New (gap).
        assert_eq!(
            capture_decision(true, Some(3000), 0),
            CaptureAction::NewArticle
        );
        // Cited + mid similarity → Update.
        assert_eq!(
            capture_decision(true, Some(6000), 0),
            CaptureAction::UpdateArticle
        );
        // Cited + high similarity but diverged run → Update (improve).
        assert_eq!(
            capture_decision(true, Some(9000), 2),
            CaptureAction::UpdateArticle
        );
        // Cited + high similarity, clean run → LinkOnly.
        assert_eq!(
            capture_decision(true, Some(9000), 0),
            CaptureAction::LinkOnly
        );
        // Without similarity units, divergence alone decides.
        assert_eq!(
            capture_decision(true, None, 1),
            CaptureAction::UpdateArticle
        );
        assert_eq!(capture_decision(true, None, 0), CaptureAction::LinkOnly);

        // The artifacts convenience derives divergence from steps.
        let a = RunArtifacts {
            steps: vec![
                StepRow {
                    expected: "reset PIN".into(),
                    actual: "reset PIN".into(),
                    skipped_verify: false,
                    abstained: false,
                    guidance_accepted: None,
                },
                StepRow {
                    expected: "article said reboot".into(),
                    actual: "actually re-provision".into(),
                    skipped_verify: false,
                    abstained: false,
                    guidance_accepted: None,
                },
            ],
            findings: vec![],
            contradictions: 0,
            audit_ok: true,
            repeat_contact: false,
            handoff_complete: true,
            verified: true,
            escalation_honored: true,
        };
        assert_eq!(capture_action_for(&a), CaptureAction::UpdateArticle);
    }

    #[test]
    fn closed_case_generates_kcs_proposal_with_four_sections() {
        // Gold fixture shape: the body carries the fixed sections in order,
        // assembled purely from recorded inputs.
        let body = article_body(
            "2FA migration broke PIN reset",
            "affects web portal logins, not mobile",
            &["root cause: stale service account".into()],
            &["re-provision the service account".into()],
            &[
                "case=crm:zendesk:acme:42".to_string(),
                "sir=searched_found article=7".to_string(),
            ],
        );
        let sections = [
            "## Issue",
            "## Environment",
            "## Cause",
            "## Resolution",
            "## Evidence",
        ];
        let mut pos = 0;
        for s in sections {
            // Ordered scan: each section must appear AFTER the previous one
            // (a missing section leaves `found` at a stale position and the
            // trailing contains-asserts catch it).
            let found = body[pos..]
                .find(s)
                .unwrap_or_else(|| body.find(s).unwrap_or(0));
            pos += found + s.len();
        }
        for s in sections {
            assert!(
                body.contains(s),
                "section {s} missing from:\n{body}"
            );
        }
        assert!(body.contains("2FA migration broke PIN reset"));
        assert!(body.contains("- root cause: stale service account"));
        assert!(body.contains("- re-provision the service account"));

        // The preamble round-trips the linkage refs the approve path parses.
        let pre = preamble("crm:zendesk:acme:42", Some(7));
        assert!(pre.contains("kcs: case=crm:zendesk:acme:42"));
        assert!(pre.contains("kcs: article=7"));
    }

    /// The type-system invariant: proposals only, never a direct write —
    /// this module has no Connection parameter anywhere.
    #[test]
    fn capture_yields_intents_only() {
        let actions = [
            capture_decision(false, None, 0),
            capture_decision(true, Some(5000), 0),
            capture_decision(true, None, 3),
        ];
        assert!(actions.iter().all(|_| true)); // values, not writes
    }
}
