//! Prompt caching + compaction discipline (the "meter visible" lesson).
//!
//! Invariants: the system prompt is a cache-stable prefix — deterministic
//! assembly, no timestamps or randomness, so an unchanged session re-hits the
//! provider prefix cache. Compaction happens only under pressure and NEVER
//! rewrites history: the plan keeps a verbatim tail and appends one summary
//! entry; older entries are referenced, not mutated.

/// Compaction pressure threshold (window tokens).
pub const COMPACT_PRESSURE_TOKENS: usize = 16_000;

/// Verbatim tail kept through compaction (window tokens).
pub const KEEP_VERBATIM_TOKENS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PromptError {
    TooManyLines { lines: usize, max: usize },
}

impl std::fmt::Display for PromptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromptError::TooManyLines { lines, max } => {
                write!(f, "system prompt is {lines} lines (max {max})")
            }
        }
    }
}
impl std::error::Error for PromptError {}

/// Max system-prompt body lines (~20) plus the skill-listing block.
pub const MAX_SYSTEM_PROMPT_LINES: usize = 20;

/// Assemble the cache-stable system prompt: fixed lines first, one skill
/// listing line per skill, nothing time- or run-dependent.
pub fn system_prompt(lines: &[&str], skills: &[&str]) -> Result<String, PromptError> {
    let total = lines.len()
        + if skills.is_empty() {
            0
        } else {
            skills.len() + 1
        };
    if total > MAX_SYSTEM_PROMPT_LINES {
        return Err(PromptError::TooManyLines {
            lines: total,
            max: MAX_SYSTEM_PROMPT_LINES,
        });
    }
    let mut out = String::new();
    for l in lines {
        out.push_str(l);
        out.push('\n');
    }
    if !skills.is_empty() {
        out.push_str("skills:");
        for s in skills {
            out.push(' ');
            out.push_str(s);
        }
        out.push('\n');
    }
    Ok(out)
}

/// True only under real pressure — never speculatively.
pub fn should_compact(window_tokens: usize) -> bool {
    window_tokens >= COMPACT_PRESSURE_TOKENS
}

/// A compaction PLAN over history entries given their token counts.
/// Entry 0 is oldest. The tail within `KEEP_VERBATIM_TOKENS` stays verbatim;
/// everything older folds into ONE appended summary entry. History itself is
/// never rewritten by this crate — the host applies the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompactionPlan {
    /// `history[0..summarize_end]` folds into the summary entry.
    pub summarize_end: usize,
    /// Token estimate of the summary entry the host will append.
    pub summary_tokens_estimate: usize,
}

/// Compute the plan, or `None` when there is nothing to fold (the whole
/// history fits in the verbatim tail).
pub fn compact_plan(token_counts: &[usize]) -> Option<CompactionPlan> {
    let total: usize = token_counts.iter().sum();
    if !should_compact(total) {
        return None;
    }
    let mut keep = 0usize;
    let mut summarize_end = 0usize;
    for (i, t) in token_counts.iter().enumerate().rev() {
        if keep + *t > KEEP_VERBATIM_TOKENS {
            break;
        }
        keep += t;
        summarize_end = i;
    }
    if summarize_end == 0 {
        return None;
    }
    // Summary estimate: ~4 tokens per folded entry header plus a bounded
    // fraction of folded content — deliberately conservative so the summary
    // never silently exceeds what it replaced.
    let folded: usize = token_counts[..summarize_end].iter().sum();
    Some(CompactionPlan {
        summarize_end,
        summary_tokens_estimate: (folded / 4).min(2_000) + 16,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_deterministic_and_cache_stable() {
        let a = system_prompt(&["you are a steward", "be careful"], &["triage"]).unwrap();
        let b = system_prompt(&["you are a steward", "be careful"], &["triage"]).unwrap();
        assert_eq!(a, b);
        assert!(!a.contains("20"), "no timestamps or run data");
        assert!(a.starts_with("you are a steward"));
        assert!(a.ends_with("skills: triage\n"));
    }

    #[test]
    fn oversized_prompt_refused_not_trimmed_silently() {
        let lines: Vec<&str> = (0..25).map(|_| "line").collect();
        assert!(matches!(
            system_prompt(&lines, &[]),
            Err(PromptError::TooManyLines { .. })
        ));
    }

    #[test]
    fn compaction_only_under_pressure() {
        assert!(!should_compact(15_999));
        assert!(should_compact(16_000));
    }

    #[test]
    fn plan_keeps_verbatim_tail_never_rewrites_history() {
        // 10 entries x 5k = 50k tokens: last 4 fit in the 20k tail.
        let counts = vec![5_000usize; 10];
        let plan = compact_plan(&counts).unwrap();
        assert_eq!(plan.summarize_end, 6);
        assert_eq!(
            counts[plan.summarize_end..].iter().sum::<usize>(),
            20_000,
            "verbatim tail preserved exactly"
        );
        assert!(plan.summary_tokens_estimate > 0);
    }

    #[test]
    fn no_plan_when_tail_covers_everything() {
        assert!(
            compact_plan(&[30_000]).is_none(),
            "single entry cannot fold"
        );
        assert!(compact_plan(&[]).is_none());
    }
}
