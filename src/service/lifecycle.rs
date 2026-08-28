//! The lifecycle aggregate family — the gate handler's decay + GDPR
//! surfaces converged onto the service layer, one submodule per aggregate.
//! Laws that travel with the family:
//!
//! - [`decay`] — `/decayed` as ONE unit: the superset SQL only narrows the
//!   scan, the Rust arbiter decides every row (`sql_superset_plus_rust_
//!   arbiter_move_together` pins the pairing).
//! - [`purge`] — `/purge` by-ids/by-owner around the shared primitive
//!   ([`crate::service::purge`]): the by-owner sweep + hold preflight +
//!   evidence audit run INSIDE one tx; negative-reach invalidation (the
//!   primitive's `recall_traces` deletes + tombstone) rides the same tx.
//! - [`fetch`] — by-id/batch read projections returning STORED forms; the
//!   read seam + row-domain re-authz + record gate stay at the handler
//!   emission boundary.
//!
//! FK-children map: NO parent-row DELETE lives here — decay and fetch are
//! read-only; the only deletion is the primitive's `knowledge` hard-delete,
//! whose complete FK-children map is the [`crate::service::purge`] module
//! header (incl. the NO ACTION ceilings). Its residue rows-affected checks
//! (`if n > 0` → tombstone + count) are unchanged there.
//!
//! Bounds: every cap is a named constant re-asserted inside the core (the
//! fence holds of the FUNCTION): [`crate::config::MAX_DECAYED`],
//! [`crate::config::MAX_MULTI_GET`], [`crate::config::MAX_PURGE_IDS`]. The
//! routes keep their identical wire fences in front.
//!
//! Wire-shape ceiling: rows stay the legacy `serde_json::Value` shapes with
//! the exact pre-move `json!` literals — byte-for-byte wire pins outrank
//! the domain-type aspiration (the retention exemplar's ceiling).

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
