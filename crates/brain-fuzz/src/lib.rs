//! Fuzz targets for the SDK's pure surfaces: evidence reducer, WorkflowMeta
//! validator, hostcall payload canonicalizer, scorer.
//!
//! Each target is a total function over arbitrary bytes — it must return,
//! never panic. The committed corpus under `corpus/` replays through the SAME
//! targets in the normal test suite (`cargo test`), so a crash found by the
//! fuzzer becomes a regression test by dropping its bytes in `corpus/`.

use brain_engine_sdk::hostcall::HostCallPayload;
use brain_engine_sdk::pure::evidence::{reduce, Finding};
use brain_engine_sdk::pure::qa_score::{score_run, RunArtifacts, StepRow};
use brain_engine_sdk::workflow::{WorkflowMeta, MAX_TOTAL_AGENTS};

/// Evidence reducer over arbitrary finding batches.
pub fn fuzz_evidence(data: &[u8]) {
    let findings = decode_findings(data);
    let _ = reduce(findings);
}

fn decode_findings(data: &[u8]) -> Vec<Finding> {
    data.chunks(8)
        .filter(|c| c.len() == 8)
        .map(|c| Finding {
            claim: String::from_utf8_lossy(&c[..4]).to_string(),
            evidence: String::from_utf8_lossy(&c[4..]).to_string(),
            source: "fuzz".into(),
            confidence: 0.5,
            ts: 1,
        })
        .collect()
}

/// WorkflowMeta validator: any byte string is a candidate name/description.
pub fn fuzz_meta(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let meta = WorkflowMeta {
        name: text.chars().take(200).collect(),
        description: text.to_string(),
        when_to_use: None,
        phases: text.lines().take(MAX_TOTAL_AGENTS).map(str::to_string).collect(),
    };
    // Must validate-or-refuse; never panic.
    let _ = meta.validate();
}

/// Hostcall payload canonicalizer over wire-shaped triples.
pub fn fuzz_hostcall(data: &[u8]) {
    if data.is_empty() {
        return;
    }
    let kind = match data[0] % 4 {
        0 => "tool",
        1 => "secret",
        2 => "exec",
        _ => "unknown-kind",
    };
    let mut rest = &data[1..];
    let split = rest.iter().position(|b| *b == b'\n').unwrap_or(rest.len());
    let (name, body) = rest.split_at(split);
    if !body.is_empty() {
        rest = &body[1..];
    }
    let _ = HostCallPayload::canonicalize(
        kind,
        &String::from_utf8_lossy(name),
        &String::from_utf8_lossy(rest),
    );
}

/// Scorer over arbitrary artifact shapes.
pub fn fuzz_scorer(data: &[u8]) {
    let steps: Vec<StepRow> = data
        .chunks(4)
        .map(|c| StepRow {
            expected: String::from_utf8_lossy(&c[..1.min(c.len())]).to_string(),
            actual: String::from_utf8_lossy(&c[c.len().saturating_sub(1)..]).to_string(),
            skipped_verify: c.first().is_some_and(|b| b & 1 == 1),
            abstained: false,
            guidance_accepted: None,
        })
        .take(64)
        .collect();
    let artifacts = RunArtifacts {
        steps,
        findings: vec![String::from_utf8_lossy(data).to_string()],
        contradictions: 0,
        audit_ok: true,
        repeat_contact: false,
        handoff_complete: true,
        verified: true,
        escalation_honored: true,
    };
    let _ = score_run(&artifacts);
}

#[cfg(feature = "libfuzzer")]
mod libfuzzer_targets {
    use super::*;
    use libfuzzer_sys::fuzz_target;

    /// One entry point dispatching on the first byte — cargo-fuzz wants a
    /// single target per binary; the dispatcher keeps all four surfaces live.
    fuzz_target!(|data: &[u8]| match data.first().map(|b| b % 4) {
        Some(0) => fuzz_evidence(data),
        Some(1) => fuzz_meta(data),
        Some(2) => fuzz_hostcall(data),
        _ => fuzz_scorer(data),
    });
}
