//! Compaction quality probes — compaction as a measured experiment.
//!
//! After a compaction commits, a FIXED probe set (10 deterministic lexical
//! probes, each ≤ 128 chars) is retrieved against the session window —
//! pre-compaction corpus versus post-compaction corpus (summary + verbatim
//! tail). Zero provider calls: the probes are local lexical retrievals over
//! text already in memory. The hit-rate delta is a typed telemetry row; a
//! degradation at or beyond the pinned threshold is a named, logged event
//! that flips the loop's posture conservative (no further auto-compaction
//! this episode).
//!
//! Integer law throughout: hit counts and per-mille arithmetic only — no
//! floats, no model self-reports, no provider judgments.

/// The probe census: exactly ten probes per compaction, bounds-capped.
pub(crate) const PROBE_COUNT: usize = 10;

/// Each probe is bounded — a probe is a retrieval key, not a document.
pub(crate) const PROBE_MAX_CHARS: usize = 128;

/// The degradation threshold, in per-mille of the probe set: a compaction
/// that loses HALF the retrievable probes (≥ 500‰) is a degraded summary.
pub(crate) const DEGRADATION_THRESHOLD_PERMILLE: i64 = 500;

/// A probe must clear this length to count as a hit — a one-character
/// coincidence is not a retrieval.
pub(crate) const PROBE_HIT_MIN_CHARS: usize = 6;

/// The typed probe report (the telemetry row's payload shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProbeReport {
    pub probes: usize,
    pub pre_hits: usize,
    pub post_hits: usize,
    /// `(pre - post) * 1000 / probes` — integer per-mille of lost probes.
    pub lost_permille: i64,
    pub degraded: bool,
}

/// Deterministic probe terms from the summarized head: distinct lowercase
/// alphanumeric tokens of content-bearing length, in first-appearance
/// order, capped to [`PROBE_COUNT`]. Same head, same probes — the probe
/// set is a function of the text alone.
pub(crate) fn probe_terms(head_payloads: &[&str]) -> Vec<String> {
    let mut probes: Vec<String> = Vec::with_capacity(PROBE_COUNT);
    for payload in head_payloads {
        let lower = payload.to_lowercase();
        let mut token = String::new();
        for ch in lower.chars().chain(std::iter::once(' ')) {
            if ch.is_ascii_alphanumeric() {
                if token.chars().count() < PROBE_MAX_CHARS {
                    token.push(ch);
                }
            } else {
                flush_token(&mut probes, &mut token);
            }
        }
        if probes.len() >= PROBE_COUNT {
            break;
        }
    }
    probes.truncate(PROBE_COUNT);
    probes
}

fn flush_token(probes: &mut Vec<String>, token: &mut String) {
    if token.chars().count() >= PROBE_HIT_MIN_CHARS
        && !probes.contains(token)
        && probes.len() < PROBE_COUNT
    {
        probes.push(token.clone());
    }
    token.clear();
}

/// Lexical retrieval: the fraction of probes with a hit in the corpus.
/// A probe "hits" when the corpus contains it (case-insensitively) — the
/// corpus is the session text itself, no index, no provider.
pub(crate) fn hits(probes: &[String], corpus: &str) -> usize {
    let lower = corpus.to_lowercase();
    probes
        .iter()
        .filter(|p| {
            let p = p.trim();
            p.chars().count() >= PROBE_HIT_MIN_CHARS && lower.contains(p)
        })
        .count()
}

/// The pre/post report for one committed compaction. `pre_corpus` is the
/// window before the compaction; `post_corpus` is the summary plus the
/// verbatim tail after it; `head_payloads` is the summarized text the
/// probes are drawn from.
pub(crate) fn report(pre_corpus: &str, post_corpus: &str, head_payloads: &[&str]) -> ProbeReport {
    let probes = probe_terms(head_payloads);
    let pre_hits = hits(&probes, pre_corpus);
    let post_hits = hits(&probes, post_corpus);
    let probes_n = probes.len() as i64;
    let lost_permille = if probes_n == 0 {
        0
    } else {
        (pre_hits as i64 - post_hits as i64) * 1000 / probes_n
    };
    // A weak baseline never latches: if fewer than half the probes hit
    // pre-compaction, the head was too thin to measure and the honest
    // posture is "no signal", not "degraded".
    let weak_baseline = pre_hits * 2 < probes.len();
    ProbeReport {
        probes: probes.len(),
        pre_hits,
        post_hits,
        lost_permille,
        degraded: !weak_baseline && lost_permille >= DEGRADATION_THRESHOLD_PERMILLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_are_deterministic_and_capped() {
        let head = vec![
            "the rebuild rate degraded on node-042 during the customer load",
            "battery state reports Failed after the firmware update",
        ];
        let a = probe_terms(&head);
        let b = probe_terms(&head);
        assert_eq!(a, b, "same head, same probes");
        assert!(a.len() <= PROBE_COUNT);
        assert!(a.iter().all(|p| p.chars().count() <= PROBE_MAX_CHARS));
        assert!(
            a.iter().any(|p| p.contains("rebuild")),
            "content-bearing tokens are probe candidates"
        );
    }

    #[test]
    fn short_or_repeated_tokens_never_become_probes() {
        let head = vec!["aa ab abc the the the"];
        assert!(probe_terms(&head).is_empty(), "no content-bearing tokens");
    }

    #[test]
    fn faithful_summary_keeps_every_probe() {
        let head = vec!["the rebuild rate degraded on node-042; battery state reports Failed"];
        let probes = probe_terms(&head);
        assert!(!probes.is_empty());
        let corpus = head[0];
        assert_eq!(hits(&probes, corpus), probes.len());
        let r = report(corpus, corpus, &head);
        assert!(!r.degraded);
        assert_eq!(r.lost_permille, 0);
    }

    #[test]
    fn lossy_summary_past_half_the_probes_is_degraded() {
        let head =
            vec!["rebuild throughput collapsed node-042 battery firmware salience drift observed"];
        // The summary keeps only two of the probe terms verbatim.
        let summary = "rebuild throughput observed";
        let r = report(head[0], summary, &head);
        assert!(r.pre_hits >= 5, "a measurable baseline: {r:?}");
        assert!(r.degraded, "losing most probes is degraded: {r:?}");
    }

    #[test]
    fn weak_baseline_never_latches() {
        let head = vec!["aa bb cc dd"];
        let r = report(head[0], "", &head);
        assert!(!r.degraded, "a thin head is no signal, not degradation");
    }
}
