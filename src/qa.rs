pub fn scope_violation(restricted: bool, domains: &[String]) -> bool {
    restricted && domains.iter().any(|d| d != "global")
}

pub fn scorecard(in_scope: bool, cited: bool, confident: bool) -> i64 {
    let base = if in_scope { 50 } else { 0 };
    base + if cited { 30 } else { 0 } + if confident { 20 } else { 10 }
}

/// v1.27.8 "QaQueue": compose the R7 scorecard from a proposal's trace signals
/// (pure over the read shapes). An absent trace degrades `cited` to neutral —
/// a proposal with no linked recall-trace is never penalized for being
/// uncited. `in_scope` = the proposal's `owner` falls under the supervisor's
/// `manages` set (R1 role).
pub(crate) fn score_for(in_scope: bool, cited: bool, confident: bool, has_trace: bool) -> i64 {
    scorecard(in_scope, cited || !has_trace, confident)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(domains: &[&str]) -> Vec<String> {
        domains.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn scope_violation_flags_cross_domain_when_restricted() {
        assert!(scope_violation(true, &d(&["global", "beta-eu"])));
        assert!(!scope_violation(false, &d(&["global", "beta-eu"])));
        assert!(!scope_violation(true, &d(&["global"])));
    }

    #[test]
    fn scorecard_is_deterministic_and_bounded() {
        assert_eq!(scorecard(false, false, false), 10);
        assert_eq!(scorecard(true, true, true), 100);
        for i in 0..2u8 {
            for j in 0..2u8 {
                for k in 0..2u8 {
                    let s = scorecard(i == 1, j == 1, k == 1);
                    assert!((0..=100).contains(&s), "score {s} out of range");
                }
            }
        }
    }

    #[test]
    fn scorecard_penalizes_out_of_scope_sharply() {
        for cited in [false, true] {
            for confident in [false, true] {
                assert!(
                    scorecard(false, cited, confident) < scorecard(true, cited, confident),
                    "out-of-scope can never out-score in-scope"
                );
            }
        }
    }

    #[test]
    fn score_for_absent_trace_is_cited_neutral_not_nan() {
        assert_eq!(score_for(false, false, false, false), 40);
        assert_eq!(score_for(true, false, false, false), 90);
        assert_eq!(score_for(true, true, true, true), 100);
        assert_eq!(score_for(false, true, true, true), 50);
    }
}
