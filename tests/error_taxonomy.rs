//! A3 error-taxonomy audit: every error type, one table, machine-checked.
//!
//! Why a table, not prose: error types are the failure contract — each one
//! decides what an operator (or an attacker probing the seam) learns when
//! something breaks. An unlisted error enum is an unreviewed failure story:
//! does it render via `Display` (operator-safe, fixed strings) or does it
//! leak `Debug` internals (paths, SQL, key material) to a client? This test
//! fails the moment a 26th error enum appears, forcing its author to classify
//! it (name → file → category → renderer) in [`KNOWN_ERRORS`] before merge.
//!
//! Renderer rule (the security property):
//! * `display` — the file contains `impl ... Display ... for {Name}`. The
//!   messages are fixed strings; no variant formats raw internals.
//! * `http-envelope` — renders through `IntoResponse` (status + envelope),
//!   never through `Display`. Only [`AppError`] (the legacy HTTP type).
//! * `gap` — listed in [`KNOWN_GAPS`] with a reason. The test pins the gap
//!   set EXACTLY: fixing the gap without removing its entry fails (stale
//!   acceptance), and a new gap without an entry fails (silent rot).
//!
//! Method: source-scan over `src/` in the `dup_guard` idiom. New file only —
//! nothing under `src/` changes for the audit itself (the two `Display`
//! impls this audit produced live beside their enums, not here).

use std::path::{Path, PathBuf};

/// (error type, file relative to `src/`, failure category, renderer).
/// Categories are coarse and security-shaped: `auth` (identity decisions),
/// `storage` (where bytes live), `input` (request-derived), `integrity`
/// (hashes/keys/vectors), `service` (operator workflows), `workflow`
/// (governed lanes), `http` (the wire envelope).
const KNOWN_ERRORS: &[(&str, &str, &str, &str)] = &[
    ("LoadError", "auth/jwks.rs", "auth", "display"),
    ("AuthError", "auth/jwt.rs", "auth", "display"),
    ("RefreshError", "auth/revocation.rs", "auth", "gap"),
    (
        "StorageLayoutError",
        "storage_layout.rs",
        "storage",
        "display",
    ),
    ("EmbedError", "embed.rs", "integrity", "display"),
    (
        "AppError",
        "server/router/memory.rs",
        "http",
        "http-envelope",
    ),
    ("QueryDocError", "search/query.rs", "input", "display"),
    (
        "DomainRegistryError",
        "domain_registry.rs",
        "storage",
        "display",
    ),
    ("ChainKeyError", "audit/mod.rs", "integrity", "display"),
    ("MeshError", "workflow/mesh.rs", "workflow", "display"),
    ("ValetError", "workflow/valet.rs", "workflow", "display"),
    ("ChannelError", "workflow/channel.rs", "workflow", "display"),
    ("ParcelError", "workflow/parcels.rs", "workflow", "display"),
    ("ShiftError", "workflow/shifts.rs", "workflow", "display"),
    ("OutboxError", "workflow/outbox.rs", "workflow", "display"),
    ("CrewError", "workflow/crew.rs", "workflow", "display"),
    ("OfferError", "workflow/relay.rs", "workflow", "display"),
    ("TokenError", "ump_integrity.rs", "auth", "display"),
    ("PurgeError", "service/purge.rs", "service", "display"),
    ("GateError", "service/review.rs", "input", "display"),
    ("IngestError", "service/ingest.rs", "input", "display"),
    ("DsarError", "service/dsar.rs", "service", "display"),
    (
        "RetentionError",
        "service/retention.rs",
        "service",
        "display",
    ),
    ("WfmError", "bin_common/wfm_import.rs", "service", "display"),
    ("SecretError", "secrets.rs", "service", "display"),
];

/// Accepted renderer gaps: (error type, reason, owner). The test pins this
/// list exactly — see the module docs.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[(
    "RefreshError",
    "no Display: renders Debug-only; file is no-touch on the A3 track",
    "auth",
)];

fn src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

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

/// Top-level `pub enum {Name}` sites for every Name ending in `Error`
/// (the `singularity_pins` top-level idiom: indented items are methods or
/// nested types, never the error contract).
fn error_enum_sites(root: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    collect_rs_files(root, &mut files);
    let mut out = Vec::new();
    for f in &files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for line in text.lines() {
            if line.starts_with(' ') || line.starts_with('\t') {
                continue;
            }
            let mut rest = line.trim_start();
            if let Some(after_pub) = rest.strip_prefix("pub ") {
                rest = after_pub.trim_start();
                if let Some(after_paren) = rest.strip_prefix('(')
                    && let Some(close) = after_paren.find(')')
                {
                    rest = after_paren[close + 1..].trim_start();
                }
            }
            if let Some(after_enum) = rest.strip_prefix("enum ") {
                let ident: String = after_enum
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if ident.ends_with("Error") {
                    let display = f
                        .strip_prefix(root)
                        .unwrap_or(f)
                        .to_string_lossy()
                        .to_string();
                    out.push((ident, display));
                }
            }
        }
    }
    out.sort();
    out
}

fn file_has_display(src_file: &Path, name: &str) -> bool {
    let text = match std::fs::read_to_string(src_file) {
        Ok(t) => t,
        Err(_) => return false,
    };
    // `impl Display for X`, `impl std::fmt::Display for X`, `impl fmt::Display
    // for X` — any path spelling counts; the property is "a Display impl
    // exists", not its import style.
    text.split("impl")
        .skip(1)
        .any(|block| block.contains("Display") && block.contains(&format!("for {name}")))
}

/// 1. The inventory is complete: the scanner's error-enum set equals the
///
/// table's name set. A new error type fails here until it is classified.
#[test]
fn taxonomy_covers_every_error_enum() {
    let root = src_root();
    let found = error_enum_sites(&root);
    let mut found_names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
    found_names.sort_unstable();
    let mut known_names: Vec<&str> = KNOWN_ERRORS.iter().map(|(n, _, _, _)| *n).collect();
    known_names.sort_unstable();
    assert_eq!(
        found_names, known_names,
        "error-enum set drifted — classify the newcomer in KNOWN_ERRORS \
         (found: {found_names:?}, table: {known_names:?})"
    );
    // Each table row points at the file that actually defines the enum.
    for (name, file, _, _) in KNOWN_ERRORS {
        let hits: Vec<&String> = found
            .iter()
            .filter(|(n, f)| n == name && f == file)
            .map(|(_, f)| f)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "{name} must be defined in exactly {file} (hits: {hits:?}) — \
             the enum moved; update the table deliberately"
        );
    }
}

/// 2. The renderer rule holds per row: `display` rows have a Display impl,
///
/// `http-envelope` rows render through IntoResponse, `gap` rows are exactly
/// the accepted gap set (no silent new gaps, no stale acceptances).
#[test]
fn every_error_renders_safely_or_is_a_pinned_gap() {
    let root = src_root();
    let gap_names: Vec<&str> = KNOWN_GAPS.iter().map(|(n, _, _)| *n).collect();
    for (name, file, _, renderer) in KNOWN_ERRORS {
        let path = root.join(file);
        match *renderer {
            "display" => assert!(
                file_has_display(&path, name),
                "{name} ({file}) is tabled as `display` but has no Display impl — \
                 add one (fixed strings, no internals) or re-table it"
            ),
            "http-envelope" => {
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(
                    text.contains(&format!("IntoResponse for {name}")),
                    "{name} ({file}) is tabled as `http-envelope` but has no \
                     IntoResponse impl — the wire mapping moved; update the table"
                );
            }
            "gap" => assert!(
                gap_names.contains(name),
                "{name} ({file}) is tabled as `gap` but has no KNOWN_GAPS entry — \
                 every gap needs a reason + owner"
            ),
            other => panic!("{name}: unknown renderer {other:?} — fix the table"),
        }
    }
    // The gap set is exact: a fixed gap left listed fails (stale acceptance),
    // an unlisted gap fails above. Both directions ratchet toward zero gaps.
    for (name, reason, owner) in KNOWN_GAPS {
        let row = KNOWN_ERRORS
            .iter()
            .find(|(n, _, _, _)| n == name)
            .unwrap_or_else(|| panic!("KNOWN_GAPS lists {name} but KNOWN_ERRORS does not"));
        assert_eq!(
            row.3, "gap",
            "{name} is in KNOWN_GAPS but tabled as {:?} — pick one",
            row.3
        );
        assert!(
            !reason.is_empty() && !owner.is_empty(),
            "{name}: gap needs reason + owner"
        );
        let path = root.join(row.1);
        assert!(
            !file_has_display(&path, name),
            "{name} grew a Display impl but is still listed in KNOWN_GAPS — \
             remove the gap entry (the ratchet only moves toward zero)"
        );
    }
    eprintln!(
        "error taxonomy: {} types, {} pinned gap(s)",
        KNOWN_ERRORS.len(),
        KNOWN_GAPS.len()
    );
}
