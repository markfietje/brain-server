//! The evidence-reducer.
//!
//! Normalizes raw findings *before* storage so only claim-grouped,
//! deduplicated, contradiction-surfaced evidence ever reaches the findings and
//! contradictions tables. Output order is deterministic (claim-sorted).

use std::collections::{BTreeMap, HashSet};

/// One normalized finding, claim-grouped and evidence-pinned. Callers
/// construct; the reducer only reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub claim: String,
    pub evidence: String,
    pub source: String,
    pub confidence: f64,
    pub ts: i64,
}

/// The output of [`reduce`] over a raw finding batch.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub struct Reduction {
    pub findings: Vec<Finding>,
    /// Pairs of finding indexes that claim the same thing but disagree — the
    /// contradictions that must be surfaced, never merged.
    pub contradictions: Vec<(usize, usize)>,
}

/// Canonicalize a claim for grouping: collapse whitespace, ASCII-lowercase.
/// ASCII-only lowering keeps the key deterministic (claims are compared, not
/// displayed).
fn normalize_claim(claim: &str) -> String {
    let mut out = String::with_capacity(claim.len());
    for word in claim.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out.make_ascii_lowercase();
    out
}

/// Reduce raw findings into claim-groups, dedup, and surface contradictions.
///
/// Grouping key = the canonical claim. Within a group:
/// - identical (claim, evidence) pairs collapse to ONE row (dedup);
/// - rows with *different* evidence are kept SEPARATE (the false-merge guard) —
///   near-identical claims with different provenance must not be glued;
/// - the highest-confidence row of a differently-evidenced group becomes the
///   canonical member; every other member surfaces as a contradiction against
///   it (evidence disagreeing on the same claim is a live conflict).
pub fn reduce(raw: Vec<Finding>) -> Reduction {
    let mut groups: BTreeMap<String, Vec<Finding>> = BTreeMap::new();
    for f in raw {
        groups.entry(normalize_claim(&f.claim)).or_default().push(f);
    }

    let mut findings = Vec::new();
    let mut contradictions = Vec::new();
    for (_, mut members) in groups {
        // Dedup: keep the highest-confidence row per distinct evidence string —
        // O(n) via the seen-set.
        members.sort_by(|a, b| {
            b.confidence
                .total_cmp(&a.confidence)
                .then_with(|| a.claim.cmp(&b.claim))
        });
        let mut seen: HashSet<String> = HashSet::with_capacity(members.len());
        let mut uniq: Vec<Finding> = Vec::with_capacity(members.len());
        for m in members {
            if seen.insert(m.evidence.clone()) {
                uniq.push(m);
            }
        }

        // The canonical member is the highest-confidence surviving row.
        let base_idx = findings.len();
        findings.push(uniq.remove(0));
        for m in uniq {
            findings.push(m);
            contradictions.push((base_idx, findings.len() - 1));
        }
    }
    Reduction {
        findings,
        contradictions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(claim: &str, evidence: &str, confidence: f64) -> Finding {
        Finding {
            claim: claim.to_string(),
            evidence: evidence.to_string(),
            source: "oracle".to_string(),
            confidence,
            ts: 1,
        }
    }

    #[test]
    fn evidence_reducer_deterministic_order() {
        // Case + whitespace differences canonicalize to ONE claim key; identical
        // evidence → dedup to a single row. Output is deterministic.
        let raw = vec![
            f("ReBoot node", "log a", 0.4),
            f("REBOOT  NODE", "log a", 0.9),
        ];
        let r1 = reduce(raw.clone());
        assert_eq!(r1.findings.len(), 1, "identical (claim, evidence) dedups");
        assert_eq!(r1.contradictions.len(), 0);
        // The highest-confidence member survives dedup (0.9 wins).
        assert_eq!(r1.findings[0].confidence, 0.9);
        assert_eq!(reduce(raw).findings, r1.findings, "deterministic order");
    }

    #[test]
    fn evidence_reducer_false_merge_guard() {
        // Two claims that AGREE on the words but cite DIFFERENT evidence must
        // stay separate — merging them would let one weak source vouch for
        // another's claim.
        let raw = vec![
            f("Replace battery", "TSR SEL event", 0.9),
            f("Replace battery", "customer report", 0.5),
        ];
        let r = reduce(raw);
        assert_eq!(r.findings.len(), 2, "different evidence must not merge");
        assert_eq!(
            r.contradictions.len(),
            1,
            "differently-evidenced claims on one group surface a contradiction"
        );
    }

    #[test]
    fn evidence_reducer_surfaces_contradiction() {
        // Three DIFFERENT sources all claiming "replace battery" (same canonical
        // key) → the highest-confidence one is canonical; the other two each
        // surface against it.
        let raw = vec![
            f("Replace battery", "SEL event", 0.6),
            f("REPLACE  battery", "rebuild-rate graphs", 0.95),
            f("replace   battery", "vendor rumor", 0.3),
        ];
        let r = reduce(raw);
        assert_eq!(
            r.findings.len(),
            3,
            "dedup keeps distinct evidence separate"
        );
        assert_eq!(
            r.contradictions.len(),
            2,
            "two weaker differently-evidenced members surface against the canonical"
        );
        // The canonical member is the highest-confidence one (0.95).
        assert_eq!(r.findings[0].confidence, 0.95);
        // Every surfaced contradiction refs the canonical index (0).
        assert!(r.contradictions.iter().all(|(a, _)| *a == 0));
    }
}
