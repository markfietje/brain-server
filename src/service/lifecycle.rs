//! The lifecycle aggregate family — the gate handler's decay + GDPR-
//! lifecycle surfaces converged onto the service layer, one submodule per
//! aggregate.
//!
//! **Submodules (one aggregate each):**
//! - [`decay`] — the `/decayed` operator review list. The SQL-superset WHERE
//!   and the Rust-side expiry arbiter move as ONE unit (the superset builder
//!   and the arbiter are inseparable): the SQL narrows the scan, the Rust
//!   filter decides every row's fate — the SQL-never-decides-a-row law
//!   travels with BOTH halves, pinned by
//!   `sql_superset_plus_rust_arbiter_move_together`.
//! - [`purge`] — the `/purge` by-ids/by-owner families: target resolution,
//!   the legal-hold preflight (the exact shared `409 legal_hold_active`
//!   envelope), and the single-tx orchestration around the shared
//!   knowledge-purge primitive ([`crate::service::purge`]). The negative-
//!   reach invalidation rides the SAME tx inside that primitive: the
//!   `recall_traces` deletes (a trace whose `$.hits` still names a purged id
//!   is a stale negative-lookup artifact that would "prove" erased content
//!   was returned) and the tombstone row commit-or-roll-back together with
//!   the erasure — pinned in the Quarry primitive tests, re-asserted here.
//! - [`fetch`] — the by-id/batch read projections: the `/get/{id}` +
//!   `/multi-get` row loads and the shared knowledge-row projection the
//!   `/ump/*` record paths and `/export` render from. Read-only; the read
//!   seam (`sanitize_read*`) stays at the handler emission boundary —
//!   services return stored forms.
//!
//! **FK-children map (scope law #3).** This family performs NO parent-row
//! DELETE of its own: decay and fetch are read-only aggregates, and the only
//! deletion is the shared primitive's `knowledge` hard-delete — its complete
//! FK-children map (`vec_knowledge`, `relationships`, `evidence_links`,
//! `proposals.conflict_with`, `recall_traces` JSON1 sweep, `embeddings`
//! CASCADE auto, the orphan-`entities` sweep, and the documented NO ACTION
//! ceilings `case_articles` + `kcs_translations`) is the
//! [`crate::service::purge`] module header, unchanged by this move.
//!
//! **Rows-affected checks (the certified-silence class).** The residue
//! DELETEs and the tombstone-only-when-a-row-was-actually-deleted check
//! (`if n > 0`) live in the primitive and keep their Quarry pins; this move
//! adds no delete path, so it adds no rows-affected site.
//!
//! **Bounds inventory.** Every cap the family enforces is a named constant
//! re-asserted inside the core (the fence holds of the FUNCTION, not
//! call-site discipline): `/decayed` clamps to [`crate::config::MAX_DECAYED`]
//! with a non-negative offset; `/multi-get` refuses beyond
//! [`crate::config::MAX_MULTI_GET`]; `/purge` refuses beyond
//! [`crate::config::MAX_PURGE_IDS`]. The routes keep their identical wire
//! fences in front, so the probe-blind vocabulary is unchanged.
//!
//! **Wire-shape ceiling (honest).** The decayed rows and the knowledge-row
//! projection stay the legacy `serde_json::Value` shapes, built with the
//! exact `json!` literals the handlers used pre-move — the byte-for-byte
//! wire pins outrank the domain-type aspiration (the retention exemplar's
//! documented ceiling, same reasoning).

pub mod decay;
pub mod fetch;
pub mod purge;

#[cfg(test)]
mod pins {
    use std::path::Path;

    /// The lifecycle source assertion: the family is free of the
    /// handler + transport layers. Production source across `lifecycle.rs`
    /// and every `lifecycle/*.rs` submodule never names a handler-module
    /// path or a handler type, never names a transport type, and never takes
    /// a connection-factory handle — services take connections and return
    /// domain types; the handler maps to HTTP at the boundary. (The general
    /// tree grep in `service/mod.rs` scans only the files DIRECTLY under
    /// `src/service/` — this pin walks the lifecycle subtree, closing the
    /// same blind spot the dsar pin closes for `dsar/sweep.rs`.)
    #[test]
    fn lifecycle_module_has_no_http_types() {
        const FORBIDDEN: &[&str] = &[
            "crate::handlers",
            "HandlerError",
            "AppError",
            "axum",
            "StatusCode",
            "Json",
            "AppState",
            "Pool",
        ];
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service/lifecycle");
        let mut files =
            vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service/lifecycle.rs")];
        let mut stack = vec![base.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("lifecycle submodule dir must exist") {
                let path = entry.expect("readable dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().map(|x| x == "rs").unwrap_or(false) {
                    files.push(path);
                }
            }
        }
        assert!(
            files.len() >= 4,
            "sanity: expected lifecycle.rs + at least the three aggregates \
             (decay, purge, fetch), found {}",
            files.len()
        );
        files.sort();
        for f in &files {
            let text = std::fs::read_to_string(f).expect("service file must be readable");
            let prod = text
                .split("#[cfg(test)]")
                .next()
                .expect("split always yields a first slice");
            let display = f
                .strip_prefix(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("src/service/")
                        .as_path(),
                )
                .unwrap_or(f)
                .to_string_lossy()
                .into_owned();
            for token in FORBIDDEN {
                assert!(
                    !prod.contains(token),
                    "layer violation in src/service/{display}: production source names \
                     `{token}` — the lifecycle cores take connections and return domain \
                     types; map to HTTP at the handler boundary"
                );
            }
        }
    }
}
