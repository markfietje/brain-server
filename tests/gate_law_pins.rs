//! R51 — the gate-law pins (I51.6 census register, I51.6b reachability, I51.6c hooks).
//!
//! **The invariant.** Every `LoopDriver` the system builds in production must be inside
//! the gate law: its output is arbitrated by `parse_and_gate` before it can change
//! durable knowledge state. Until this round that was **enforced by convention** — the
//! most load-bearing invariant in the architecture, and nothing enforced it.
//!
//! **Why an enumerated register, not a prohibition.** The original claim — *"every raw
//! `LoopDriver::new` site is inside `#[cfg(test)]`"* — was **false**. Exactly two
//! production sites exist and **both are inside the law**. A prohibition would fail on
//! a legitimate design, so the pin is an **enumeration plus a reachability assertion**,
//! which is strictly stronger: it names the sites, and it checks what they reach.
//!
//! **The census IS the pin; the register is a fixture of it.** A hand-maintained list is
//! exactly what produced the three-versus-two discrepancy this round had to correct (it
//! counted `case_run.rs:224`, which constructs a `GdlDriver`, not a `LoopDriver`). So
//! the census is **derived from source on every run**, and the register is checked
//! against that derivation. A new production site fails until it is added *with* its
//! justification — amending the register is a governance event.
//!
//! **Two detectors, agreement required.** Detector (a) is the `#[cfg(test)]` that is
//! **immediately followed by `mod tests {`** — deliberately NOT the file's first
//! `#[cfg(test)]`; those are different lines, and a `head -1` scan takes the wrong one
//! (it is what the original census did, surviving only by luck). Detector (b) is the
//! nearest preceding `#[cfg(test)]` on an *item*, which catches `gdl.rs:2542` inside
//! `new_ablated` — far above that file's module boundary. **A site is production only if
//! both agree.** Comment-stripped per the `handler_body` precedent (v1.28.86 / F7-07) so
//! a symbol named in a comment cannot produce a false pass.

use std::collections::BTreeSet;
use std::path::PathBuf;

// ── the production register ────────────────────────────────────────────────

/// The two production `LoopDriver::new` sites, each with its justification.
///
/// **Amending this list is a governance event, not a chore.** A register that is
/// amended casually is one that is failing quietly. If a refactor legitimately
/// introduces a third mediated entry, it is added here *with its reason* and with a
/// reachability argument — never silently.
/// The production construction sites, keyed by **file + the enclosing item's
/// NAME**, never by line number.
///
/// ## Why it is keyed by name, and why that matters
///
/// This register was line-keyed, and a line-keyed governance register has a
/// failure mode worse than having no register: **it fires on edits that changed
/// nothing it claims to measure.** Deleting ten lines of superseded dead code
/// *above* a site moved that site's line, and the census pin demanded a
/// governance amendment for a non-event.
///
/// R53a already recorded this exact tax once — *"AMENDED 2026-09-29 (R53a), line
/// 639 → 656, and nothing else. The shift is purely additive documentation …
/// No statement, no expression, no data path and no control-flow edge at the
/// construction site changed"* — and had to write a paragraph proving the site had
/// not moved in any way that mattered. **A pin that must be amended when the line
/// moves is a pin whose amendments stop carrying information**, because a reader
/// can no longer tell a real new spawn site from a reflow.
///
/// The governance claim was never about a line. It is about *which function
/// constructs an unmediated loop*, and a name survives every edit above it. So the
/// register keys the name, keeps the line only for the failure message, and a new
/// spawn site still fails the pin — now because a genuinely new FUNCTION appears,
/// which is the event the register exists to catch.
const REGISTER: &[(&str, &str, &str)] = &[
    (
        "src/workflow/gdl.rs",
        "new_with_proficiency",
        "The parent engine, built inside `GdlDriver::new_with_proficiency`. \
         `GdlDriver::new` is only the L3-delegating wrapper. Inside the \
         law: the GDL's screening is the phase machine ABOVE the loop (`parse_and_gate`, \
         the authority matrix, MAX_PHASE_ATTEMPTS), and the loop's own output is \
         disposed downstream of the loop.",
    ),
    (
        "src/agentloop/subagents.rs",
        "delegate_owned_budgeted",
        "The production child-loop spawn inside `delegate_owned_budgeted`. Inside the \
         law by its DATA PATH: constructed with filtered tools (`spec.allowed_tools`), a \
         narrowed env (`narrowed_env`), an explicit budget (`Some(spec.token_budget)`, \
         not the `None` uncapped default), a turn cap and a `child:<name>:` prefix; its \
         `SubagentOutcome::Completed { summary }` is consumed in gdl.rs and \
         disposed by `parse_and_gate`. \
         \
         KEYED BY NAME since the line-keyed register proved to be a liability: a \
         documentation-only edit above either site moved its line and demanded a \
         governance amendment that recorded no change of substance.",
    ),
];

/// Symbol binding: the register binds the function the `LoopDriver::new` actually
/// LIVES IN. Binding the wrapper would pin the wrong symbol — `GdlDriver::new` is the
/// delegating wrapper, `new_with_proficiency` is where the engine is constructed.
const GDL_ENCLOSING_SYMBOL: &str = "new_with_proficiency";

// ── source reading + the two detectors ─────────────────────────────────────

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_src(rel: &str) -> String {
    let p = crate_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()))
}

/// Strip comments so a symbol named in a comment cannot produce a false pass.
///
/// Line-wise and deliberately conservative: it only ever REMOVES text, so it can
/// under-report a site, never invent one. A missed site shows up as a census
/// disagreement, not a silent production entry.
fn strip_comments(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in src.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut res = String::new();
        let mut i = 0usize;
        let (mut in_str, mut in_chr) = (false, false);
        while i < chars.len() {
            if in_block {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    in_block = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if !in_str && !in_chr && chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
                break;
            }
            if !in_str && !in_chr && chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                in_block = true;
                i += 2;
                continue;
            }
            let c = chars[i];
            if c == '"' && !in_chr {
                in_str = !in_str;
            } else if c == '\'' && !in_str {
                in_chr = !in_chr;
            }
            res.push(c);
            i += 1;
        }
        out.push(res);
    }
    out
}

fn is_cfg_test(line: &str) -> bool {
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.contains("#[cfg(test)]")
}

/// Detector (a): the `#[cfg(test)]` IMMEDIATELY FOLLOWED BY `mod tests {`.
/// Returns `(first_line, last_line)` of that module, 1-based.
fn detector_a_module_boundary(lines: &[String]) -> Option<(usize, usize)> {
    for (i, line) in lines.iter().enumerate() {
        if !is_cfg_test(line) {
            continue;
        }
        let mut j = i + 1;
        while j < lines.len() && lines[j].trim().is_empty() {
            j += 1;
        }
        if j < lines.len() && lines[j].trim_start().starts_with("mod tests") {
            return Some((j + 1, lines.len()));
        }
    }
    None
}

/// Detector (b): the brace-span of every ITEM carrying a `#[cfg(test)]` attribute.
/// This is what catches `gdl.rs:2542`, inside `new_ablated` (:2532), far above the
/// file's module boundary at :4387.
fn detector_b_item_spans(lines: &[String]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if !t.starts_with("#[") {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut is_test = false;
        while i < lines.len()
            && (lines[i].trim_start().starts_with("#[") || lines[i].trim().is_empty())
        {
            if is_cfg_test(&lines[i]) {
                is_test = true;
            }
            i += 1;
        }
        if !is_test || i >= lines.len() {
            continue;
        }
        let indent = lines[i].len() - lines[i].trim_start().len();
        let end = brace_span(lines, i, indent).unwrap_or(lines.len());
        spans.push((start, end));
        i = end;
    }
    spans
}

/// End line (1-based) of the block whose opening brace is at/after `from_col` on
/// `start_idx`.
fn brace_span(lines: &[String], start_idx: usize, from_col: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut seen = false;
    for (idx, line) in lines.iter().enumerate().skip(start_idx) {
        let col_from = if idx == start_idx { from_col } else { 0 };
        for c in line.chars().skip(col_from) {
            match c {
                '{' => {
                    depth += 1;
                    seen = true;
                }
                '}' => {
                    depth -= 1;
                    if seen && depth == 0 {
                        return Some(idx + 1);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// A `LoopDriver::new` occurrence, classified.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    file: String,
    /// The enclosing top-level item's name — the stable identity of the site.
    item: String,
    /// Which construction this is **within** that item, 0-based.
    ///
    /// **This is load-bearing, and its absence was a real weakening.** Keying on
    /// the name alone collapses two spawns inside one function into one entry, so
    /// a second, unmediated spawn in an already-registered function passes the
    /// census. Verified by planting exactly that: the name-keyed register went
    /// GREEN against a live bypass. A site is therefore `item#ordinal`.
    ordinal: usize,
    /// Retained for the failure message only; never compared.
    line: usize,
}

impl Site {
    /// The register's key form.
    fn key(&self) -> String {
        if self.ordinal == 0 {
            self.item.clone()
        } else {
            format!("{}#{}", self.item, self.ordinal)
        }
    }
}

/// The top-level `fn`/`impl` item a line sits inside, or `"<module>"`.
///
/// **This is what makes the register reflow-proof.** Keyed on a line number, the
/// register broke on an unrelated deletion ten lines above it — and a pin that
/// fails on an edit that changed nothing it claims to measure trains a reader to
/// amend it reflexively. The enclosing item's NAME is what the governance claim is
/// actually about ("the parent engine", "the subagent child"), and a name does not
/// move when code above it is added or removed.
fn enclosing_item(lines: &[String], idx: usize) -> String {
    for i in (0..=idx).rev() {
        let t = lines[i].trim_start();
        // `async fn` and `pub(crate) async fn` are ordinary top-level items here,
        // so the qualifier list must cover them or the scan walks PAST the real
        // function and reports whatever earlier item it lands on.
        let rest = [
            "pub fn ",
            "pub(crate) fn ",
            "fn ",
            "pub async fn ",
            "async fn ",
            "pub(crate) async fn ",
            "impl ",
        ]
        .iter()
        .find_map(|kw| t.strip_prefix(kw));
        if let Some(rest) = rest {
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return name;
            }
        }
    }
    "<module>".to_string()
}

/// Derive the production set from source, with both detectors required to agree.
fn derive_production_sites() -> BTreeSet<Site> {
    let mut production = BTreeSet::new();
    let mut stack: Vec<String> = vec!["src".to_string()];
    let mut files: Vec<String> = Vec::new();
    while let Some(dir) = stack.pop() {
        let p = crate_root().join(&dir);
        let entries =
            std::fs::read_dir(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()));
        for e in entries.flatten() {
            let path = e.path();
            let rel = path
                .strip_prefix(crate_root())
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                stack.push(rel);
            } else if rel.ends_with(".rs") {
                files.push(rel);
            }
        }
    }
    files.sort();
    let mut per_item: std::collections::BTreeMap<(String, String), usize> =
        std::collections::BTreeMap::new();
    for rel in files {
        let lines = strip_comments(&read_src(&rel));
        let a = detector_a_module_boundary(&lines);
        let b = detector_b_item_spans(&lines);
        for (idx, line) in lines.iter().enumerate() {
            if !line.contains("LoopDriver::new") {
                continue;
            }
            let ln = idx + 1;
            let in_a = a.is_some_and(|(s, e)| ln >= s && ln <= e);
            let in_b = b.iter().any(|(s, e)| ln >= *s && ln <= *e);
            // production only if BOTH detectors agree
            if !in_a && !in_b {
                let item = enclosing_item(&lines, idx);
                let n = per_item.entry((rel.clone(), item.clone())).or_insert(0);
                let ordinal = *n;
                *n += 1;
                production.insert(Site {
                    file: rel.clone(),
                    // The enclosing item's NAME plus its ordinal, not the line: a
                    // register keyed on a line number breaks on any edit above it
                    // and forces a governance amendment for a non-event, while a
                    // name alone would hide a second spawn in one function.
                    item,
                    ordinal,
                    line: ln,
                });
            }
        }
    }
    production
}

// ── I51.6 · the census pin ─────────────────────────────────────────────────

/// **I51.6 — the derived census equals the recorded register.**
#[test]
fn r51_gate_law_census_matches_the_register() {
    // Keyed on (file, enclosing item NAME) — never on the line, which moves on
    // any edit above the site and would demand a governance amendment for a
    // reflow. The register's doc says why in full.
    let derived: BTreeSet<(String, String)> = derive_production_sites()
        .into_iter()
        .map(|s| {
            let key = s.key();
            (s.file, key)
        })
        .collect();
    let registered: BTreeSet<(String, String)> = REGISTER
        .iter()
        .map(|(f, item, _)| (f.to_string(), item.to_string()))
        .collect();

    assert_eq!(
        derived, registered,
        "The derived gate-law census does not match the register.\n\
         DERIVED:   {derived:?}\n\
         REGISTER:  {registered:?}\n\
         A new production `LoopDriver::new` site has appeared. It is INSIDE the gate law \
         only if its output is arbitrated by `parse_and_gate` before it can change durable \
         state — prove that, then amend the register WITH its justification. Amending the \
         register is a governance event. If the site is NOT inside the law, this is a live \
         finding: a production spawn that bypasses the gate. \
         NOTE: sites are keyed by enclosing ITEM NAME, so a line shift alone can never \
         cause this failure."
    );
}

/// The census must be able to come back EMPTY or SHORT, or the equality pin above
/// could pass because everything matched.
#[test]
fn r51_gate_law_census_is_not_vacuous() {
    let derived = derive_production_sites();
    assert_eq!(
        derived.len(),
        REGISTER.len(),
        "the census returned {derived:?} — the register has {} entries. A census that \
         cannot come back short cannot detect an addition.",
        REGISTER.len()
    );
}

/// Every registered site must carry a NON-EMPTY justification. A register entry
/// without a reason is an incomplete entry.
#[test]
fn r51_gate_law_register_entries_carry_justifications() {
    for (file, item, why) in REGISTER {
        assert!(
            why.len() > 40,
            "register entry {file}::{item} has no substantive justification ({why:?}). \
             Every entry records WHY the site is inside the gate law."
        );
        assert!(
            !why.contains("new\n"),
            "register entry {file}::{item} justification must be a single paragraph"
        );
    }
}

// ── I51.6 · symbol binding ─────────────────────────────────────────────────

/// The register binds the function the `LoopDriver::new` actually LIVES IN.
/// `GdlDriver::new` is the L3-delegating wrapper; binding it would pin the wrong
/// symbol — a refactor of the wrapper would leave the engine untouched and the pin
/// would still pass, or fail for the wrong reason.
#[test]
fn r51_gdl_engine_site_is_bound_to_new_with_proficiency() {
    let lines = strip_comments(&read_src("src/workflow/gdl.rs"));
    let site_line = 2500usize;

    // Walk backwards to the nearest `fn` declaration.
    let enclosing = lines
        .iter()
        .take(site_line - 1)
        .enumerate()
        .rev()
        .find(|(_, l)| {
            l.trim_start().starts_with("fn ")
                || l.trim_start().starts_with("pub(crate) fn ")
                || l.trim_start().starts_with("pub fn ")
        })
        .map(|(i, l)| (i + 1, l.trim().to_string()));

    let (decl_line, decl) = enclosing.expect("the site must live inside some fn");
    assert!(
        decl.contains(GDL_ENCLOSING_SYMBOL),
        "the `LoopDriver::new` at gdl.rs:{site_line} lives inside a function declared at \
         :{decl_line} — `{decl}` — but the register binds `{GDL_ENCLOSING_SYMBOL}`. The \
         register must bind the function the call actually lives in: `GdlDriver::new` \
         (:2458) is only the L3-delegating wrapper that calls `new_with_proficiency` \
         (:2482), which is where the engine is built."
    );

    // And the wrapper really is a delegator, not a constructor. Bound the search to
    // `impl GdlDriver` so an unrelated `fn new()` elsewhere in the file cannot match.
    let impl_start = lines
        .iter()
        .position(|l| l.trim() == "impl GdlDriver {")
        .expect("gdl.rs must contain `impl GdlDriver`")
        + 1;
    let wrapper = lines
        .iter()
        .enumerate()
        .skip(impl_start)
        .find(|(_, l)| l.contains("fn new(") && !l.contains("new_with"))
        .map(|(i, _)| i + 1);
    let w = wrapper.expect("`impl GdlDriver` must contain `fn new(`");
    let body: String = lines[w..(w + 25).min(lines.len())].join("\n");
    assert!(
        body.contains("new_with_proficiency"),
        "GdlDriver::new (:{w}) is expected to delegate to `new_with_proficiency`. If that \
         changed, the register's symbol binding must be re-derived from the code, never \
         from prose."
    );
}

// ── I51.6b · reachability — the load-bearing half ─────────────────────────

/// The public entry constructs a `GdlDriver` and contains **no** raw
/// `LoopDriver::new`. The public entry is a separate concept from the allowlist: it is
/// where the reachability chain BEGINS, not a spawn site.
#[test]
fn r51_public_entry_constructs_gdl_and_no_raw_loopdriver() {
    let text = read_src("src/handlers/case_run.rs");
    let lines = strip_comments(&text);

    assert!(
        text.contains("GdlDriver::new"),
        "src/handlers/case_run.rs must construct a GdlDriver — it is the public entry of \
         the case machine and the root of the reachability chain."
    );
    let raw: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("LoopDriver::new"))
        .map(|(i, _)| i + 1)
        .collect();
    assert!(
        raw.is_empty(),
        "src/handlers/case_run.rs contains a raw `LoopDriver::new` at {raw:?}. The public \
         entry must go through `GdlDriver`, which is what carries the gate law."
    );
}

/// Every `delegate_owned*` CALL SITE in `src/` is in `gdl.rs` — the adversarial-recheck
/// seam. This is the seam that makes the child's output reach `parse_and_gate`.
///
/// **Re-verified 2026-09-29, against a first reading that got this wrong.** The
/// `AgentsSvc::delegate` at `services.rs:361` is production SOURCE and calls
/// `delegate_owned_budgeted`, but it is **unreachable in production**: `resolve_profile`
/// and `assemble` have no caller outside `services.rs`'s own `mod tests`, so the `web`
/// profile that mounts `KEY_AGENTS` is only ever resolved by tests. Being inside a
/// production function is not the same as being reachable — and for a reachability pin
/// that distinction is the whole point. The pin below therefore asserts the CALL-SEITE
/// set, which is the property that actually holds.
#[test]
fn r51_delegate_call_sites_are_the_gdl_recheck_seam() {
    let mut call_sites: Vec<Site> = Vec::new();
    let mut stack = vec!["src".to_string()];
    let mut files: Vec<String> = Vec::new();
    while let Some(dir) = stack.pop() {
        let p = crate_root().join(&dir);
        let entries = std::fs::read_dir(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        for e in entries.flatten() {
            let path = e.path();
            let rel = path
                .strip_prefix(crate_root())
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                stack.push(rel);
            } else if rel.ends_with(".rs") {
                files.push(rel);
            }
        }
    }
    files.sort();
    for rel in files {
        let lines = strip_comments(&read_src(&rel));
        let a = detector_a_module_boundary(&lines);
        let b = detector_b_item_spans(&lines);
        for (idx, line) in lines.iter().enumerate() {
            // a DECLARATION (`fn delegate_owned(`) is not a call site
            if line.contains("fn delegate_owned") {
                continue;
            }
            if !(line.contains("delegate_owned(") || line.contains("delegate_owned_budgeted(")) {
                continue;
            }
            let ln = idx + 1;
            // a bare `use ... delegate_owned_budgeted` import is not a call site
            if line.trim_start().starts_with("use ") {
                continue;
            }
            // inside the file's `mod tests` / a #[cfg(test)] item
            if a.is_some_and(|(s, e)| ln >= s && ln <= e)
                || b.iter().any(|(s, e)| ln >= *s && ln <= *e)
            {
                continue;
            }
            call_sites.push(Site {
                file: rel.clone(),
                item: enclosing_item(&lines, idx),
                ordinal: 0,
                line: ln,
            });
        }
    }
    // A production-SOURCE call site outside the chain is only tolerable if it is
    // UNREACHABLE. `services.rs:361` (AgentsSvc::delegate) is exactly that case, and
    // the proof is mechanical: the service registry is only ever entered through
    // `resolve_profile` + `assemble`, and neither has a caller outside `services.rs`'s
    // own `mod tests` — so the `web` profile that mounts `KEY_AGENTS` is resolved by
    // tests only. Assert the UNREACHABILITY rather than assuming it, because the
    // difference between "production source" and "reachable" is the whole point.
    let outside: Vec<&Site> = call_sites
        .iter()
        .filter(|s| s.file != "src/agentloop/subagents.rs" && s.file != "src/workflow/gdl.rs")
        .collect();
    for s in &outside {
        let services = strip_comments(&read_src("src/agentloop/services.rs"));
        let tests_from = detector_a_module_boundary(&services).map_or(services.len(), |(a, _)| a);
        // every key the profile could mount must be requested by a caller OUTSIDE tests
        for entry in ["resolve_profile(", "assemble("] {
            let prod_callers = services
                .iter()
                .enumerate()
                .filter(|(_, l)| l.contains(entry) && !l.trim_start().starts_with("pub(crate) fn"))
                .filter(|(i, _)| *i + 1 < tests_from)
                .count();
            assert_eq!(
                prod_callers, 0,
                "{entry} now has {prod_callers} production caller(s) in services.rs, so the \
                 delegate call at {}:{} may be REACHABLE. Re-derive the gate-law argument: a \
                 reachable spawn path must prove its output reaches `parse_and_gate` before it \
                 can change durable state, or it is a live finding.",
                s.file, s.line
            );
        }
        println!(
            "note: {}:{} is production SOURCE but UNREACHABLE (no production caller of \
             resolve_profile/assemble) — the unreachability argument holds",
            s.file, s.line
        );
    }

    // The GDL seam specifically: exactly one, and it is the root of the chain.
    let gdl_sites: Vec<&Site> = call_sites
        .iter()
        .filter(|s| s.file == "src/workflow/gdl.rs")
        .collect();
    assert_eq!(
        gdl_sites.len(),
        1,
        "exactly one production delegate call site is expected in gdl.rs (the \
         adversarial-recheck seam), found {gdl_sites:?}. A second one is a second spawn path \
         and needs its own reachability argument."
    );
}

/// The seam's outcome is consumed by the GDL and routed through the arbiter, bounded by
/// `MAX_PHASE_ATTEMPTS`. **A spawn that returns directly to a durable write must fail
/// this pin even though it passes an absence check** — that is the interesting failure,
/// and an absence check can never catch it.
#[test]
fn r51_spawn_outcome_reaches_parse_and_gate_bounded_by_phase_attempts() {
    let gdl = strip_comments(&read_src("src/workflow/gdl.rs"));

    // The seam: the delegate call is followed by the Completed outcome being consumed.
    let seam = gdl
        .iter()
        .position(|l| l.contains("delegate_owned("))
        .map(|i| i + 1)
        .expect("gdl.rs must call delegate_owned (the adversarial-recheck seam)");
    let window: String = gdl[(seam - 1).min(gdl.len())..(seam + 60).min(gdl.len())].join("\n");
    assert!(
        window.contains("SubagentOutcome::Completed"),
        "the delegate seam at gdl.rs:{seam} must match on `SubagentOutcome::Completed` — the \
         child's summary is the thing that has to reach the gate. If the outcome is dropped \
         or written durably instead, this pin must fail."
    );
    assert!(
        window.contains("summary"),
        "the seam at gdl.rs:{seam} must CONSUME the child's summary. A spawn whose output is \
         discarded, or written straight to durable state, is exactly the failure this pin \
         exists to catch — and an absence check can never catch it."
    );

    // The arbiter exists and is pure, and the dispose sites call it.
    assert!(
        gdl.iter().any(|l| l.contains("fn parse_and_gate(")),
        "the arbiter `parse_and_gate` must exist in gdl.rs — it is the disposition point."
    );
    let dispose_sites = gdl
        .iter()
        .filter(|l| l.contains("parse_and_gate(") && !l.contains("fn parse_and_gate("))
        .count();
    assert!(
        dispose_sites >= 2,
        "the model artifact must be DISPOSED by `parse_and_gate` at its call sites \
         (found {dispose_sites}); a disposition point that is defined but not called is not \
         a gate."
    );

    // The bound: exhausting phase attempts ROUTES the case, it never resolves it.
    assert!(
        gdl.iter()
            .any(|l| l.contains("MAX_PHASE_ATTEMPTS: u32 = 3")),
        "`MAX_PHASE_ATTEMPTS = 3` must exist — the gate's bound is part of the law."
    );
    let attempt_sites = gdl
        .iter()
        .filter(|l| l.contains("attempt >= MAX_PHASE_ATTEMPTS"))
        .count();
    assert!(
        attempt_sites >= 2,
        "the attempt bound must be checked where artifacts are disposed (found \
         {attempt_sites} sites) — exhausting the attempts ROUTES the case, never resolves it."
    );
}

// ── I51.6c · hooks are uniform; the watch item is what remains ─────────────

/// **I51.6c — CLOSED, and this pin records the verified answer.** The question was:
/// *does the child's `pass_through()` mean its screening path differs from the main
/// loop's?* **It does not.** Every production construction site uses
/// `LoopHooks::pass_through()` — `gdl.rs:2509` (parent) and `subagents.rs:652`
/// (child) — and `case_run.rs:222` records *"hooks stay constructor-injected policy"*.
/// **Production injects no hooks anywhere**, so there is no parent/child asymmetry at
/// the hooks level.
///
/// Screening is **structural**, not hook-based: the GDL's is the phase machine above
/// the loop; the child's is delegation narrowing (filtered tools, `narrowed_env`, an
/// explicit budget) plus its summary returning into the adversarial-recheck seam
/// (gdl.rs:2680 / :2694), where the finding is disposed by the same machinery.
/// Deliberate, not accidental.
#[test]
fn r51_production_construction_sites_inject_no_hooks() {
    for (file, item, _) in REGISTER {
        let lines = strip_comments(&read_src(file));
        // Locate the CONSTRUCTION by function, not by a recorded line: an edit
        // anywhere in the file can no longer move this check onto other code.
        let anchor = lines
            .iter()
            .position(|l| l.contains(&format!("fn {item}(")))
            .unwrap_or_else(|| {
                panic!("register names {file}::{item}, which no longer exists in the file")
            });
        // From the item's own opening line, find the FIRST `LoopDriver::new`
        // that belongs to it: stop at the next top-level `fn`, so a sibling
        // item's construction cannot stand in for this one's.
        let mut construction: Option<usize> = None;
        for (off, line) in lines.iter().enumerate().skip(anchor + 1) {
            let t = line.trim_start();
            if (t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("pub(crate) fn "))
                && !t.starts_with("fn run_tests")
                && off > anchor + 1
                && !line.starts_with(' ')
            {
                break; // the next top-level item begins
            }
            if line.contains("LoopDriver::new") {
                construction = Some(off);
                break;
            }
        }
        let Some(at) = construction else {
            panic!("{file}::{item} is registered as a construction site but contains none");
        };
        // The construction's own argument list is what must carry the hooks —
        // a `pass_through()` further down the function is a different call.
        let tail = lines[at..(at + 20).min(lines.len())].join("\n");
        assert!(
            tail.contains("LoopHooks::pass_through()"),
            "the production site {file}::{item} must construct with \
             `LoopHooks::pass_through()`. This is the I51.6c watch item: **`LoopHooks` are \
             per-construction-site, not inherited.** If policy hooks are ever injected at one \
             production site, they must be injected at EVERY site in this register — the \
             gdl.rs engine AND the subagents.rs child — or the child silently bypasses hook \
             policy. A register entry that omits the hooks argument is an incomplete entry. \
             FOUND: {tail}"
        );
    }
}
