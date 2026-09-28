//! R46 — the spike. Run the resolver over the real-span corpus and report a
//! NUMBER, with every failure itemized by verdict cause (decision E9).
//!
//! **This lives in `tests/`, not `src/`, on purpose.** It is a measurement
//! harness, not production code: it reads files and prints, which the crate's
//! purity law forbids in `src/`. The sibling pins that scan the crate's source
//! walk `src/` only, so this harness is correctly outside their scope, while
//! the round's non-vacuity census deliberately walks the WHOLE crate — both
//! homes, so neither half of the battery is invisible.
//!
//! **The corpus is private and lives in the spine.** A public-only checkout has
//! no corpus, and this prints a loud `NOT RUN` banner rather than passing
//! silently — the R45-0 lesson, stated in the same words. The number the round
//! reports was taken on a machine that has the spine, and the evidence file
//! records that this gate is a measurement, not a CI gate.
//!
//! Run it directly for the full report:
//!     cargo test --manifest-path crates/Cargo.toml -p brain-evidence-core \
//!       --test r46_spike -- --nocapture
//! Point it elsewhere with R46_CORPUS=/path/to/corpus.jsonl.

use std::collections::BTreeMap;
use std::path::PathBuf;

use brain_evidence_core::{EvidenceRef, EvidenceVerdict, cid_v1, failure_cause, resolve};

/// Default location of the private corpus, relative to the kernel root.
const DEFAULT_CORPUS: &str = "../brain-steward-ip/plans/R46_CORPUS_2026-09-28.jsonl";

/// The LOCATOR SET, as preregistered in `evals/R46_DETERMINISTIC_BOUNDARY.md`
/// §6 before any measurement was taken. The resolve rate is measured over
/// exactly these kinds and nothing else.
const LOCATOR_KINDS: &[&str] = &["true", "char_offset"];

/// The boundary case: a rewrite that ALSO re-derives its CID legitimately
/// resolves, and is reported separately from both rates.
const BOUNDARY_KIND: &str = "tampered_recomputed_cid";

/// Kinds whose CID is INTENTIONALLY not the CID of the supplied bytes. These
/// are excluded from the cross-implementation agreement count.
const MISMATCHED_BY_DESIGN: &[&str] = &["wrong_cid", "tampered_original_cid"];

fn corpus_path() -> PathBuf {
    match std::env::var("R46_CORPUS") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join(DEFAULT_CORPUS),
    }
}

/// A minimal field extractor for the corpus's own JSONL shape.
///
/// Hand-rolled because the crate's whole supply-chain claim is that its only
/// dependency is a hash function, and a measurement harness does not get to
/// quietly add a JSON parser to the manifest it is measuring. The corpus is
/// emitted by `evals/r46_corpus_build.py` with a known key order and known
/// escaping, so this covers exactly that: a quoted string with JSON escapes, or
/// a bare number/keyword.
fn field(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":");
    let at = line.find(&pat)? + pat.len();
    let rest = line[at..].trim_start();
    let Some(inner) = rest.strip_prefix('"') else {
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        return Some(rest[..end].trim().to_string());
    };
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('b') => out.push('\u{8}'),
                Some('f') => out.push('\u{c}'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let cp = u32::from_str_radix(&hex, 16).unwrap_or(0xFFFD);
                    out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                }
                Some(other) => out.push(other),
                None => break,
            },
            other => out.push(other),
        }
    }
    None
}

fn num(line: &str, key: &str) -> usize {
    field(line, key)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("corpus row is missing an integer `{key}`: {line}"))
}

struct Row {
    kind: String,
    source: Vec<u8>,
    quote: Vec<u8>,
    start: usize,
    end: usize,
    cid: String,
    empty_refs: bool,
}

fn parse_row(line: &str) -> Row {
    Row {
        kind: field(line, "kind").expect("corpus row is missing `kind`"),
        source: field(line, "source")
            .expect("corpus row is missing `source`")
            .into_bytes(),
        quote: field(line, "quote")
            .expect("corpus row is missing `quote`")
            .into_bytes(),
        start: num(line, "start"),
        end: num(line, "end"),
        cid: field(line, "source_cid").expect("corpus row is missing `source_cid`"),
        empty_refs: line.contains("\"refs\": []"),
    }
}

#[test]
fn r46_spike_resolve_and_discrimination_rates() {
    let path = corpus_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        println!(
            "\n\
             ============================ R46 SPIKE: NOT RUN ============================\n\
             The private corpus is not present at {}.\n\
             R46's corpus lives in the private spine and is never copied into the\n\
             kernel, so a public-only checkout cannot run the spike. This is a\n\
             MEASUREMENT, not a CI gate: the resolve and discrimination rates the\n\
             round reports were measured on a host that has the spine, and the\n\
             round evidence file records the run.\n\
             ============================================================================\n",
            path.display()
        );
        return;
    };

    let mut locator_total = 0usize;
    let mut locator_resolved = 0usize;
    let mut near_miss_total = 0usize;
    let mut near_miss_correct = 0usize;
    let mut boundary_total = 0usize;
    let mut boundary_resolved = 0usize;
    let mut vacuous = 0usize;
    let mut cid_agree = 0usize;
    let mut cid_checked = 0usize;
    let mut cid_disagreed: Vec<String> = Vec::new();
    let mut histogram: BTreeMap<String, usize> = BTreeMap::new();
    let mut mislabelled: Vec<String> = Vec::new();

    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row = parse_row(line);
        let expected = field(line, "expected").unwrap_or_default();

        let refs: Vec<EvidenceRef<'_>> = if row.empty_refs {
            Vec::new()
        } else {
            vec![EvidenceRef {
                source_cid: &row.cid,
                quote: &row.quote,
                byte_range: row.start..row.end,
            }]
        };
        let verdict = resolve(&row.source, &refs);
        let cause = failure_cause(&verdict).unwrap_or("resolved").to_string();

        // The corpus CIDs are computed by a SEPARATE implementation
        // (evals/r46_corpus_build.py, Python hashlib + base64). The house's
        // two-implementation law says a second implementation must exist and
        // its agreement must be RECORDED — so it is counted here, and a
        // disagreement names the line rather than being averaged away.
        if !MISMATCHED_BY_DESIGN.contains(&row.kind.as_str()) {
            cid_checked += 1;
            if cid_v1(&row.source) == row.cid {
                cid_agree += 1;
            } else if cid_disagreed.len() < 10 {
                cid_disagreed.push(format!(
                    "line {} [{}]: python {} vs rust {}",
                    i + 1,
                    row.kind,
                    row.cid,
                    cid_v1(&row.source)
                ));
            }
        }

        // The KILL-3 rate: a Resolved whose declared bytes are not the quote.
        // The resolver makes this structurally impossible; it is reported
        // anyway, because a non-zero value is a KILL-3 firing, not a metric.
        if verdict == EvidenceVerdict::Resolved {
            let declared = row.source.get(row.start..row.end).unwrap_or(b"");
            if declared != row.quote.as_slice() {
                vacuous += 1;
                println!("VACUOUS RESOLVED at corpus line {}", i + 1);
            }
        }

        if LOCATOR_KINDS.contains(&row.kind.as_str()) {
            locator_total += 1;
            if verdict == EvidenceVerdict::Resolved {
                locator_resolved += 1;
            }
        } else if row.kind == BOUNDARY_KIND {
            boundary_total += 1;
            if verdict == EvidenceVerdict::Resolved {
                boundary_resolved += 1;
            }
        } else {
            near_miss_total += 1;
            if cause == expected {
                near_miss_correct += 1;
            } else {
                mislabelled.push(format!(
                    "line {} [{}]: expected {expected}, got {cause}",
                    i + 1,
                    row.kind
                ));
            }
        }
        *histogram.entry(cause.clone()).or_insert(0) += 1;
    }

    let pct = |n: usize, d: usize| -> f64 {
        if d == 0 {
            0.0
        } else {
            100.0 * n as f64 / d as f64
        }
    };

    println!("\n================ R46 SPIKE ================");
    println!("corpus: {}", path.display());
    println!("--- LOCATOR SET (resolve rate) ---");
    println!(
        "  {locator_resolved}/{locator_total} resolved = {:.2}%",
        pct(locator_resolved, locator_total)
    );
    for kind in LOCATOR_KINDS {
        println!("    kind {kind}");
    }
    println!("--- NEAR-MISSES (discrimination rate) ---");
    println!(
        "  {near_miss_correct}/{near_miss_total} refused with the EXPECTED cause = {:.2}%",
        pct(near_miss_correct, near_miss_total)
    );
    println!("--- BOUNDARY (rewrite + CID re-derived) ---");
    println!("  {boundary_resolved}/{boundary_total} resolved = the documented, legitimate case");
    println!("--- KILL-3 RATE (must be 0) ---");
    println!("  vacuous Resolved: {vacuous}");
    println!("--- TWO-IMPLEMENTATION AGREEMENT (python corpus vs rust crate) ---");
    println!("  {cid_agree}/{cid_checked} corpus CIDs reproduce exactly");
    for d in &cid_disagreed {
        println!("  DISAGREEMENT {d}");
    }
    println!("--- FAILURE HISTOGRAM BY CAUSE ---");
    for (cause, n) in &histogram {
        println!("  {cause:20} {n}");
    }
    if !mislabelled.is_empty() {
        println!("--- MISLABELLED (expected cause != actual) ---");
        for m in mislabelled.iter().take(20) {
            println!("  {m}");
        }
        if mislabelled.len() > 20 {
            println!("  ... and {} more", mislabelled.len() - 20);
        }
    }
    println!("==========================================\n");

    // The one assertion. The spike's NUMBER is reported, not gated — a low
    // resolve rate is a FINDING to be reported, not a test failure. The
    // vacuous-Resolved rate, by contrast, is KILL-3 and must be zero.
    assert_eq!(
        vacuous, 0,
        "KILL-3: a Resolved whose bytes are not the quote"
    );
    assert_eq!(
        cid_agree, cid_checked,
        "the python corpus builder and the rust crate must agree on every CID — \
         a disagreement means one of the two CID implementations is wrong, and a \
         corpus whose CIDs do not reproduce measures nothing"
    );
}
