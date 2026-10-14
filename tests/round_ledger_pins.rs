//! Round-ledger integrity: execution-plan references resolve, parallel
//! lanes carry lane-qualified round identifiers, the shipped-round pin
//! matches release history, and the operational route inventory agrees
//! with the docs-truth census.
//!
//! The lane table lives in `docs/EXECUTION_PLAN_20261007_PARALLEL.md` under
//! a `| Live ID | Lane |` header the parser below reads. Forward-looking
//! round state is deliberately NOT in this tree (local-only plans set);
//! the pins below assert identifiers and inventory, never a status.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

const LEDGER_DOCS: [&str; 1] = ["docs/EXECUTION_PLAN_20261007_PARALLEL.md"];

/// Rows of every `| Live ID | Lane |` ledger table: (doc, live id, lane).
fn ledger_rows() -> Vec<(String, String, String)> {
    let mut rows = Vec::new();
    for doc in LEDGER_DOCS {
        let src = std::fs::read_to_string(repo().join(doc))
            .unwrap_or_else(|_| panic!("ledger doc readable: {doc}"));
        let mut in_table = false;
        for line in src.lines() {
            let t = line.trim();
            if t.starts_with("| Live ID |") {
                in_table = true;
                continue;
            }
            if !in_table {
                continue;
            }
            if !t.starts_with('|') {
                in_table = false;
                continue;
            }
            let cells: Vec<&str> = t.split('|').map(str::trim).collect();
            if cells.len() < 4 || cells[1].starts_with('-') || cells[1] == "Live ID" {
                continue;
            }
            rows.push((doc.to_string(), cells[1].to_string(), cells[2].to_string()));
        }
    }
    rows
}
/// Files whose prose may name an execution plan. AGENTS.md carried the
/// round log until 2026-10-14; the retired rounds live in AGENTS_HISTORY.md,
/// and a reference that moved out of the first file must not silently stop
/// being checked — that gap is exactly what this pin would have grown if
/// only AGENTS.md were scanned.
const PLAN_NAMING_DOCS: [&str; 2] = ["AGENTS.md", "docs/AGENTS_HISTORY.md"];

/// Every backtick-delimited span, WITHOUT assuming the backticks pair up.
///
/// `split('`').skip(1).step_by(2)` assumes an even number of backticks in
/// the whole file. That holds for AGENTS.md and it does NOT hold for
/// AGENTS_HISTORY.md, which carries an odd count and 2,143 code spans that
/// straddle a `> ` blockquote continuation line. On such a file the pairing
/// shifts at the first stray and every span after it is misaligned — the
/// scanner keeps returning something, so it never errors, it just starts
/// reading the wrong substrings. A green result would have meant nothing.
///
/// So the candidate set is the union of BOTH pairings — (0,1),(2,3),… AND
/// (1,2),(3,4),… . Where the backticks genuinely pair, the two agree. Where
/// they do not, the union still contains every real span, and a spurious
/// candidate is pruned by the plan filter rather than trusted. This cannot
/// miss a span for any stray-backtick position, which is the property the
/// old idiom lacked.
fn backtick_spans(src: &str) -> Vec<&str> {
    let pos: Vec<usize> = src
        .char_indices()
        .filter(|(_, c)| *c == '`')
        .map(|(i, _)| i)
        .collect();
    let mut out = Vec::new();
    for start in [0usize, 1] {
        let mut i = start;
        while i + 1 < pos.len() {
            out.push(&src[pos[i] + 1..pos[i + 1]]);
            i += 2;
        }
    }
    out
}

fn is_plan_ref(span: &str) -> bool {
    let span = span.trim();
    span.starts_with("docs/")
        && span.ends_with(".md")
        && !span.contains(' ')
        && (span.contains("EXECUTION_PLAN") || span.contains("EXECUTION_INDEX"))
}

/// Plan-shaped path tokens in the RAW text, regardless of backticks.
///
/// The pin's claim is that a plan "named in prose" is a live reference, and
/// prose is not obliged to use code spans. An earlier version read only
/// backtick-delimited content, so a plan cited in plain text was invisible
/// to it — a citation with no backticks around it passed while pointing at a
/// file that does not exist. Scanning the raw tokens closes that: a citation
/// escapes only by not looking like a path.
fn raw_plan_refs(src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for token in src
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '/' && c != '.' && c != '-' && c != '_')
    {
        if is_plan_ref(token) && !out.iter().any(|t| t == token) {
            out.push(token.to_string());
        }
    }
    out
}

/// A `docs/` execution-plan reference named in prose is a live reference —
/// the file must exist in the tree a fresh clone sees. Scoped to
/// plan/index names on purpose: past audit reports are cited in the same
/// files but stay untracked by design (sealed history), so a blanket
/// `docs/*.md` check would fail clean clones over files that must never be
/// committed. (The parallel plan was cited but absent.)
///
/// Reads both the extracted spans and the raw text, so a citation is checked
/// whether or not it is backticked.
#[test]
fn referenced_execution_plans_exist() {
    let mut missing = Vec::new();
    for doc in PLAN_NAMING_DOCS {
        let src =
            std::fs::read_to_string(repo().join(doc)).unwrap_or_else(|_| panic!("{doc} readable"));
        for span in backtick_spans(&src) {
            let span = span.trim();
            if is_plan_ref(span) && !repo().join(span).exists() {
                missing.push(format!("{doc} (code span) -> {span}"));
            }
        }
        for token in raw_plan_refs(&src) {
            if !repo().join(&token).exists() {
                missing.push(format!("{doc} (prose) -> {token}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "execution-plan references that do not exist: {missing:?}"
    );
}

/// The anti-vacuity arm, by PLANT rather than by live content.
///
/// `referenced_execution_plans_exist` asserts an empty miss list, so a
/// scanner that stopped seeing anything would pass while checking nothing —
/// exactly the failure the parity bug would have had. Checking live
/// documents for that is unsound: AGENTS.md legitimately names no plan at
/// all since the log was archived, so a content-derived arm either asserts
/// nothing (vacuous) or demands formatting that prose does not owe.
///
/// So the plant is a synthetic string pushed through the same extraction
/// under the conditions that actually broke it — a stray backtick ahead of
/// it, a span straddling a blockquote continuation, and no backticks at
/// all. Recovery is asserted for each. This depends on no document content,
/// so it stays meaningful as the archive ages, and it fails the moment the
/// extractor is narrowed.
#[test]
fn the_plan_extraction_recovers_a_planted_reference_under_adversarial_text() {
    const PLANT: &str = "docs/EXECUTION_PLAN_29991231_PLANT.md";
    let text = |body: &str| body.replace("PLANTPATH", PLANT);

    // Each case names the ONE route that must recover it. Accepting either
    // route let the raw-text fallback mask a completely broken span
    // extractor — the arm passed while the thing it was guarding was dead.
    let span_cases: [(&str, &str); 3] = [
        ("plain code span", "prose `PLANTPATH` tail"),
        (
            "a stray backtick ahead of the plant",
            "a ` b ` PLANTPATH` c",
        ),
        (
            "several spans then the plant",
            "`one` `two` `PLANTPATH` `three`",
        ),
    ];
    for (label, body) in span_cases {
        let body = text(body);
        let spans = backtick_spans(&body);
        assert!(
            spans.iter().map(|s| s.trim()).any(|s| s == PLANT),
            "planted reference not recovered by SPAN extraction in the {label} \
             case; spans saw {spans:?}. The extractor has been narrowed or lost \
             parity immunity."
        );
    }

    // These two are the reason raw scanning exists at all: one has no
    // backticks to find, and the other\'s span swallows the blockquote
    // continuation, so it is an exact path in no pairing.
    let raw_cases: [(&str, &str); 2] = [
        ("no backticks at all", "see PLANTPATH here"),
        (
            "a path straddling a blockquote continuation",
            "> open `PLANTPATH\n> continued` tail",
        ),
    ];
    for (label, body) in raw_cases {
        let body = text(body);
        let raws = raw_plan_refs(&body);
        assert!(
            raws.iter().any(|t| t == PLANT),
            "planted reference not recovered by RAW scanning in the {label} \
             case; raw saw {raws:?}. A citation can now escape without \
             looking like a path."
        );
    }
}

/// The union across the scanned documents must actually name a plan. Without
/// this, both files could stop citing plans — legitimately, by retiring the
/// last one — and the arms above would go quiet with nothing recording that
/// the coverage ended. Naming the reference keeps that a decision rather
/// than a drift.
#[test]
fn the_scanned_documents_still_name_at_least_one_live_execution_plan() {
    let mut union: Vec<String> = Vec::new();
    for doc in PLAN_NAMING_DOCS {
        let src =
            std::fs::read_to_string(repo().join(doc)).unwrap_or_else(|_| panic!("{doc} readable"));
        for token in raw_plan_refs(&src) {
            if !union.contains(&token) {
                union.push(token);
            }
        }
    }
    assert!(
        !union.is_empty(),
        "no execution plan is named across {PLAN_NAMING_DOCS:?}; the existence pin \
         is now vacuous"
    );
    for p in &union {
        assert!(repo().join(p).exists(), "named plan {p} does not exist");
    }
}

/// Parallel lanes cannot reuse a live round identifier without a lane
/// suffix: full IDs are unique, and a shared stem keeps at most one bare
/// (historical) form. The R84 / R84-fork collision must be recorded, not
/// silent.
#[test]
fn live_lane_ids_are_unique_or_suffixed() {
    let rows = ledger_rows();
    let ids: Vec<&str> = rows.iter().map(|(_, id, _)| id.as_str()).collect();
    assert!(
        ids.contains(&"R84") && ids.contains(&"R84-fork"),
        "the ledger must record both R84 (server, shipped) and R84-fork \
         (parked fork lane): {ids:?}"
    );
    let mut seen = std::collections::BTreeSet::new();
    for id in &ids {
        assert!(
            seen.insert(*id),
            "duplicate live round identifier in the ledger: {id}"
        );
    }
    let mut bare_per_stem: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for id in &ids {
        let stem = id.split_once('-').map(|(s, _)| s).unwrap_or(id);
        if !id.contains('-') {
            *bare_per_stem.entry(stem).or_insert(0) += 1;
        }
    }
    let dupes: Vec<(&str, usize)> = bare_per_stem
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(s, n)| (*s, *n))
        .collect();
    assert!(
        dupes.is_empty(),
        "a bare round stem claimed by two lanes without a suffix: {dupes:?}"
    );
}

/// The `SHIPPED_ROUNDS` const in the register law: its length matches its
/// entries, it holds R84 (shipped in 1.29.4), and every `### R<n>` round
/// heading under a released CHANGELOG section is a member.
#[test]
fn shipped_rounds_cover_released_rounds() {
    let suite = std::fs::read_to_string(repo().join("tests/main_suite.rs"))
        .expect("main_suite.rs readable");
    let head = "const SHIPPED_ROUNDS: [&str; ";
    let start = suite.find(head).expect("SHIPPED_ROUNDS present");
    let after = &suite[start + head.len()..];
    let len_end = after.find(']').expect("length terminator");
    let declared: usize = after[..len_end]
        .trim()
        .parse()
        .expect("array length parses");
    let body_start = after.find('[').expect("entries open") + 1;
    let body_end = after.find("];").expect("entries close");
    let body = &after[body_start..body_end];
    let mut members: Vec<&str> = Vec::new();
    let mut rest = body;
    while let Some(open) = rest.find('"') {
        let tail = &rest[open + 1..];
        let close = tail.find('"').expect("literal closes");
        members.push(&tail[..close]);
        rest = &tail[close + 1..];
    }
    assert_eq!(
        members.len(),
        declared,
        "SHIPPED_ROUNDS length ({declared}) drifted from its entries ({})",
        members.len()
    );
    assert!(
        members.contains(&"R84"),
        "R84 shipped in 1.29.4 but is absent from SHIPPED_ROUNDS: {members:?}"
    );
    let changelog =
        std::fs::read_to_string(repo().join("CHANGELOG.md")).expect("CHANGELOG.md readable");
    let mut released = true;
    let mut uncovered = Vec::new();
    for line in changelog.lines() {
        if line.starts_with("## [") {
            released = !line.starts_with("## [Unreleased]");
        } else if released && line.starts_with("### R") {
            let id: String = line["### ".len()..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            // Round headings name R<digits> (`### R84 "Domains"`); prose
            // subsections (`### Release notes`, `### Red-proofs`) do not.
            let is_round =
                id.len() > 1 && id.starts_with('R') && id[1..].chars().all(|c| c.is_ascii_digit());
            if is_round && !members.contains(&id.as_str()) {
                uncovered.push(id);
            }
        }
    }
    assert!(
        uncovered.is_empty(),
        "released rounds missing from SHIPPED_ROUNDS: {uncovered:?}"
    );
    let audit = std::fs::read_to_string(repo().join("AUDIT.md")).expect("AUDIT.md readable");
    assert!(
        !audit.contains("OPEN — R84"),
        "no register row may route an OPEN item at the shipped R84 lane — \
         use UNROUTED or the lane-qualified R84-fork"
    );
}

/// The brief's route inventory runs from a foreign working directory, is
/// nonzero, and agrees with the docs-truth census.
#[test]
fn repo_brief_matches_docs_truth_from_foreign_cwd() {
    let foreign = repo().join("docs");
    let brief = Command::new("bash")
        .arg(repo().join("scripts/repo-brief.sh"))
        .current_dir(&foreign)
        .output()
        .expect("repo-brief.sh executes from a foreign directory");
    assert!(
        brief.status.success(),
        "repo-brief.sh must succeed away from the repo root: {}",
        String::from_utf8_lossy(&brief.stderr)
    );
    let text = String::from_utf8_lossy(&brief.stdout);
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("unique paths"))
        .expect("the brief prints a unique-paths inventory line");
    let count: usize = line
        .rsplit(':')
        .next()
        .expect("count after colon")
        .split_whitespace()
        .next()
        .expect("the unique-paths count leads the remainder")
        .parse()
        .expect("the unique-paths line leads with a number");
    assert!(count > 0, "the brief must report a nonzero route inventory");
    let truth = Command::new("bash")
        .arg(repo().join("scripts/docs-truth.sh"))
        .current_dir(&foreign)
        .output()
        .expect("docs-truth.sh executes from a foreign directory");
    let truth_text = String::from_utf8_lossy(&truth.stdout);
    let routes_line = truth_text
        .lines()
        .find(|l| l.contains("routes="))
        .expect("docs-truth prints its census");
    let census: usize = routes_line
        .split_whitespace()
        .find_map(|w| w.strip_prefix("routes=").and_then(|n| n.parse().ok()))
        .expect("docs-truth census parses");
    assert_eq!(
        count, census,
        "brief unique paths ({count}) must agree with the docs-truth \
         census ({census}) — same router registrations, same filtering"
    );
}

/// Reverting the unique-paths probe to the thin binary fails this pin:
/// the inventory must read router registrations.
#[test]
fn brief_probe_reads_router_registrations() {
    let script = std::fs::read_to_string(repo().join("scripts/repo-brief.sh"))
        .expect("repo-brief.sh readable");
    assert!(
        script.contains("src/server/router/*.rs"),
        "the unique-paths probe must read the router tree"
    );
    let probe: String = script
        .lines()
        .filter(|l| l.contains("unique paths"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !probe.contains("\"$m\"") && !probe.contains("< \"$m\""),
        "the unique-paths probe must not read the thin src/main.rs binary"
    );
}
