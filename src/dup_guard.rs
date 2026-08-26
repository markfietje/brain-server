//! dup_guard: the "added a helper that already existed" gate.
//!
//! The failure mode (IBM Technology, "How AI Coding Agents Understand Your
//! Codebase", 2026-08): an agent writes a helper that duplicates one the repo
//! already has, or re-imports a library for something a shared utility
//! already covers. Style lints do not catch it; review often does not either.
//!
//! This guard scans every `src/**/*.rs` for TOP-LEVEL free items (`fn` /
//! `const` at column zero — impl methods and nested-module items are
//! indented, so they are out of scope by construction) and fails when the
//! same NAME is defined in more than one file without an entry in
//! [`ALLOWED_DUPES`]. A second pin keeps the allowlist honest: an entry whose
//! duplication disappeared must be deleted.
//!
//! Scope note: server tree only (`src/`). `crates/` and `client/` are
//! separate workspaces with their own owners.

#![cfg(test)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// (name, reason) pairs permitted to repeat across files.
/// Every entry MUST carry the files it covers + why unification is wrong or
/// pending. Stale entries are themselves a test failure (see below).
///
/// Categories used in reasons:
///   [bin]     — separate `[[bin]]` crates that intentionally share nothing
///               except `src/bin_common/*`; unifying means growing that seam,
///               a deliberate refactor, not drive-by edits.
///   [domain]  — module-local vocabulary/constants; coupling sibling workflow
///               domains through one constants blob is worse than repetition.
///   [adapter] — per-source adapter logic (CRM/GitHub translators) whose
///               bodies deliberately differ per vendor dialect.
///   [shim]    — thin delegation to the single source of truth.
///   TODO(unify) — REAL duplication debt; extracted to a shared home in a
///               later cleanup. Tracked here so it cannot grow silently
///               (counts are visible in this file's diffs).
const ALLOWED_DUPES: &[(&str, &str)] = &[
    ("main", "[bin] every binary entrypoint"),
    ("run", "[bin] brain_migrate_rehearse + bench entry loops"),
    ("usage", "[bin] per-binary help printers"),
    ("run_eval", "[bin] brain.rs + bench eval harness entry"),
    ("parse_argv", "[bin] crm/gh connector arg parsers"),
    (
        "auth_token",
        "[bin] token resolution ladder repeated per binary",
    ),
    ("base_url", "[bin] server URL resolution per binary"),
    ("DEFAULT_URL", "[bin] per-binary loopback default"),
    ("dirs_home", "[bin] home-dir fallback per binary"),
    ("emit_done", "[bin] connector JSON emit tail"),
    ("emit_error", "[bin] connector JSON emit tail"),
    ("emit_log", "[bin] connector JSON emit tail"),
    ("emit_progress", "[bin] connector JSON emit tail"),
    (
        "ct_eq",
        "TODO(unify): mcp.rs + main.rs constant-time compare — fold into one shared fn",
    ),
    (
        "SERVER_VERSION",
        "TODO(unify): mcp.rs copy vs config::SERVER_VERSION",
    ),
    (
        "resolve_passphrase",
        "[bin] backup passphrase ladder duplicated into migrate-rehearse bin",
    ),
    (
        "now_iso",
        "TODO(unify): backup.rs vs migrate-rehearse bin timestamp helper",
    ),
    (
        "now_unix",
        "TODO(unify): auth/revocation.rs vs handlers/auth.rs epoch helper",
    ),
    (
        "set_mode_0600",
        "TODO(unify): two copies INSIDE bin/brain.rs (~3565/~3572)",
    ),
    (
        "set_mode_0700",
        "TODO(unify): two copies INSIDE bin/brain.rs (~3578/~3585)",
    ),
    (
        "sha256_hex",
        "TODO(unify): 4 copies (backup/kb/mesh/parcels) — canonical home should be one util module",
    ),
    (
        "hex_encode",
        "TODO(unify): 4 copies (model_pin/audit/frontend/ump)",
    ),
    (
        "build_url",
        "TODO(unify): hostcalls vs bin_common/http URL join",
    ),
    (
        "is_valid_domain",
        "[shim] handlers/mod.rs delegates to storage_layout (single truth)",
    ),
    (
        "guard_capacity",
        "TODO(unify): main.rs wrapper vs handlers/mod.rs core",
    ),
    (
        "db_err",
        "[domain] per-workflow-module error mapping closures",
    ),
    (
        "emit_event",
        "[domain] channel vs relay lineage emit shapes differ",
    ),
    ("TOPIC", "[domain] per-module outbox topic vocabulary"),
    (
        "MAX_PRINCIPAL_LEN",
        "[domain] same bound restated per domain module",
    ),
    (
        "KCS_FRESHNESS_SECS",
        "[domain] kcs core + handler view of one bound",
    ),
    (
        "MAX_OVERLAP_MINUTES",
        "[domain] shifts vs relay share one business bound",
    ),
    (
        "route",
        "[domain] domain_router dispatch vs frontdoor routing table",
    ),
    (
        "classify",
        "[domain] procedural classification vs capacity target classifier",
    ),
    (
        "resolve",
        "[domain] role registry resolve vs secrets path resolve",
    ),
    (
        "translate_issue",
        "[adapter] github translator vs pipeline fallback",
    ),
    (
        "translate_page",
        "[adapter] salesforce vs genesys page dialects",
    ),
    ("api_base", "[adapter] per-CRM base URL builders"),
    ("presets", "[domain] role vs profile preset vocabularies"),
    ("PRESETS_RAW", "[domain] role vs profile raw preset tables"),
    ("list", "[domain] role vs profile list helpers"),
    ("load", "[domain] role vs profile load helpers"),
    ("upsert", "[domain] role vs profile upsert helpers"),
    (
        "default_dsar_action",
        "[domain] clients vs observe default action",
    ),
    (
        "ingest_one",
        "[bin] bench ingest driver vs github connector ingest",
    ),
    ("map_err", "[domain] tiny handler error-mapping closures"),
    (
        "crew_touch",
        "TODO(unify): mesh/channel/relay best-effort crew touch per wiring law — extract when handlers converge",
    ),
    (
        "mask_email",
        "TODO(unify): pii_mask.rs canonical vs gate.rs local read-seam copy",
    ),
    (
        "mask_phone",
        "TODO(unify): pii_mask.rs canonical vs gate.rs local copy",
    ),
    (
        "mask_card",
        "TODO(unify): pii_mask.rs canonical vs gate.rs local copy",
    ),
    ("luhn_ok", "TODO(unify): pii_mask.rs vs gate.rs Luhn check"),
    (
        "count_digits",
        "TODO(unify): pii_mask.rs vs gate.rs digit counter",
    ),
];

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Extract names of column-zero `fn` / `const` definitions.
/// Column zero ⇒ a top-level item of that file: impl methods, nested `mod`
/// bodies, and everything inside `mod tests { … }` is indented and skipped.
fn top_level_def_names(text: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_start();
        if raw.starts_with(' ') || raw.starts_with('\t') {
            continue;
        }
        let rest = line.strip_prefix("pub ").unwrap_or(line);
        // Strip a visibility qualifier like `(crate)`, `(super)`, `(in crate::x)`.
        let rest = rest
            .strip_prefix('(')
            .and_then(|after_paren| after_paren.find(')').map(|close| &after_paren[close + 1..]))
            .map_or(rest, str::trim_start);
        let rest = rest
            .strip_prefix("const ")
            .or_else(|| rest.strip_prefix("fn "));
        if let Some(rest) = rest {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && !name.parse::<u64>().is_ok() {
                out.push((name, i + 1));
            }
        }
    }
    out
}

#[test]
fn no_duplicate_top_level_helpers_across_src() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);
    assert!(
        files.len() > 100,
        "sanity: expected the full src tree, found {} files",
        files.len()
    );

    // name -> [(file-displayname, line)]
    let mut defs: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    for f in &files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let display = f
            .strip_prefix(&src)
            .unwrap_or(f)
            .to_string_lossy()
            .to_string();
        for (name, line) in top_level_def_names(&text) {
            defs.entry(name).or_default().push((display.clone(), line));
        }
    }

    let allowed: std::collections::HashSet<&str> = ALLOWED_DUPES.iter().map(|(n, _)| *n).collect();

    let mut offenders: Vec<String> = Vec::new();
    for (name, sites) in &defs {
        if sites.len() < 2 || allowed.contains(name.as_str()) {
            continue;
        }
        let locs: Vec<String> = sites.iter().map(|(f, l)| format!("{}:{}", f, l)).collect();
        offenders.push(format!(
            "  {} defined {}x: {}",
            name,
            sites.len(),
            locs.join(", ")
        ));
    }

    assert!(
        offenders.is_empty(),
        "duplicate top-level helpers detected (extract to a shared module or \
         add a reasoned ALLOWED_DUPES entry):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn allowlist_entries_are_still_duplicates() {
    // An allowlist entry whose duplication vanished is stale documentation
    // of an exception that no longer exists — delete it.
    let manifest = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in &files {
        if let Ok(text) = std::fs::read_to_string(f) {
            for (name, _) in top_level_def_names(&text) {
                *counts.entry(name).or_default() += 1;
            }
        }
    }
    let mut stale: Vec<&str> = Vec::new();
    for (name, reason) in ALLOWED_DUPES {
        if counts.get(*name).copied().unwrap_or(0) < 2 {
            stale.push(name);
            let _ = reason;
        }
    }
    assert!(
        stale.is_empty(),
        "stale ALLOWED_DUPES entries (duplication gone — delete them): {:?}",
        stale
    );
}
