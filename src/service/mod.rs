//! The service layer — where storage lives (the Foundation Line).
//!
//! Two architectures coexisted since the Spine release: the workflow family
//! built as layered cores while the pre-Spine memory surfaces embedded
//! storage in handler bodies. This module tree is the convergence target.
//! The law is the Foundation Line roadmap's wiring law — the plan is law;
//! deviations are amendments, never improvisation:
//!
//! **What a service core owns** — ONE aggregate's complete storage story:
//! every SQL statement, the bounds/caps that guard it, the row-scoping
//! predicates, the FK-children ordering, the tombstone/certificate duties,
//! and the audit rows its mutations owe (written INSIDE the caller's
//! transaction — the audit-per-write law moves WITH the logic).
//!
//! **What a service takes** — `&rusqlite::Connection` for reads, or
//! `&rusqlite::Transaction` (the caller's [`crate::workflow::tx::WorkflowTx`])
//! for anything that writes. It NEVER takes a pool handle, server state, or
//! an HTTP-layer type. Connections are borrowed from the HANDLER's
//! `spawn_blocking` closure; the core cannot outlive them.
//!
//! **What a service returns** — domain types. The `ServiceError`
//! convention: one typed enum per module, carrying `Display` text and a
//! `From<rusqlite::Error>` impl that preserves the rusqlite message verbatim
//! (the handler maps it onto that route's FROZEN probe-blind error
//! vocabulary — 404-vs-403-vs-409 semantics are pinned per route and the
//! service never names an HTTP status). No transport shaping happens below
//! the handler boundary.
//!
//! **Transaction discipline** — every mutation runs inside the CALLER'S
//! transaction; a dropped [`crate::workflow::tx::WorkflowTx`] rolls the
//! mutation AND its evidence back together. `audit::record`/`record_tenant`
//! probe `is_autocommit()` and SAVEPOINT-nest when already inside a tx, so
//! the evidence write is atomic with the write it evidences — the
//! `*_audits_inside_the_tx` pins prove it per aggregate.
//!
//! **Time and inputs** — wall-clock time enters as an argument (unix
//! seconds) so a test pins it; parsing/validation that must produce
//! handler-shaped errors stays at the handler, while the fence that guards
//! the storage (bounds, emptiness, charset) is re-asserted in the core so
//! every future caller inherits it (the fence holds of the FUNCTION, not
//! call-site discipline).
//!
//! **Read seam** — `sanitize_read` stays at the HANDLER emission boundary;
//! services return stored forms. LLM-facing payloads are fenced at the
//! handler. (The retention exemplar carries one documented ceiling: its
//! report rows stay the legacy `serde_json::Value` maps because the
//! byte-for-byte wire pin outranks the domain-type aspiration — typing them
//! is a follow-up, not part of a move.)
//!
//! **Extraction discipline** — a move is NOT a rewrite: the code that moves
//! moves verbatim (assertion bodies included); improvements discovered
//! mid-move are filed as follow-ups. Cross-handler coupling discovered
//! mid-phase is either promoted into the core or explicitly pinned as
//! orchestration — undecided coupling is how the old pattern creeps back.
//!
//! **Enforcement** — the layer contract is pinned shut by the tests at the
//! bottom of this file, ENFORCING since the Cornerstone flip: ANY SQL
//! statement under `src/handlers/` fails CI (there is no baseline and no
//! allowlist — the debt is zero and the guard keeps it there), and the
//! transport-type greps keep the service tree free of HTTP-framework
//! identifiers. Storage changes belong in a service core by construction;
//! a handler that needs one writes the core first.

pub mod art30;
#[cfg(feature = "compliance-pack")]
pub mod compliance;
pub mod domains_admin;
pub mod dsar;
pub mod forget;
pub mod gate;
pub mod ingest;
pub mod lifecycle;
pub mod procedure;
pub mod purge;
pub mod recall;
pub mod register;
pub mod retention;
pub mod suggest;
pub mod ump_ops;
pub mod webhook_ingest;
#[cfg(test)]
mod pins {
    use std::path::Path;

    /// The SQL-statement counter, per the line's definition: one pass,
    /// case-insensitive, non-overlapping occurrences of the four statement
    /// openers (`SELECT `, `INSERT `, `UPDATE `, `DELETE FROM`). Substring
    /// semantics are deliberate — false positives (a comment naming a
    /// keyword) only make the lock stricter, never looser.
    fn count_sql_statements(source: &str) -> usize {
        let lower = source.to_ascii_lowercase();
        ["select ", "insert ", "update ", "delete from"]
            .iter()
            .map(|p| lower.matches(p).count())
            .sum()
    }

    /// v1.28.52 "Cornerstone" — the enforcing flip. The Foundation Line
    /// began by FREEZING the handler-side debt (v1.28.46 "Plumb": a per-file
    /// baseline at 445 statements across 29 files — regressions failed,
    /// progress printed deltas), then burned it to zero one extraction per
    /// commit (Quarry 359, Masonry 354, Terrace 272, Aqueduct 241, Confluence
    /// 78 — every handler file EXCEPT gate.rs drained; Cornerstone's final
    /// vein took gate.rs 78 → 0 across six surfaces: the review-queue read,
    /// the creation insert, the expire/reject family, the edit path, the
    /// approve family, and the export read). With nothing left to compare
    /// against, the baseline table and the allowlist machinery are DELETED:
    /// ANY counted statement in ANY file under `src/handlers/` — production
    /// source, test fixture, or comment residue — fails. The walk is
    /// recursive so a future subdirectory cannot quietly escape the law.
    /// The file-count sanity below refuses a vacuous pass (the lipstyk
    /// lesson: a guard that scans nothing must not smile).
    #[test]
    fn no_sql_in_handlers_enforced() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/handlers");
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("src/handlers must exist") {
                let entry = entry.expect("readable dir entry");
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().map(|x| x == "rs").unwrap_or(false) {
                    out.push(path);
                }
            }
        }
        walk(&dir, &mut files);
        assert!(
            files.len() >= 30,
            "sanity: expected the full handler tree (30+ files at the flip), found {} \
             — a walk that finds nothing has found nothing",
            files.len()
        );
        files.sort();

        let mut violations: Vec<String> = Vec::new();
        for f in &files {
            let text = std::fs::read_to_string(f).expect("handler file must be readable");
            let n = count_sql_statements(&text);
            if n > 0 {
                let display = f
                    .strip_prefix(&dir)
                    .unwrap_or(f)
                    .to_string_lossy()
                    .into_owned();
                violations.push(format!("  src/handlers{display}: {n} statement matches"));
            }
        }
        assert!(
            violations.is_empty(),
            "SQL-inventory VIOLATION — handlers are protocol adapters ONLY; ALL SQL, \
             bounds/caps, FK-children ordering, and invariants live in a domain \
             core (`src/workflow/*`, `src/service/*`) taking `&Connection` / \
             `WorkflowTx`. Write the core first, then the handler:
{}",
            violations.join("\n")
        );
    }

    /// The enforcing flip's self-pin: the counter the guard runs is the
    /// line's counter — it must still detect the four statement openers on a
    /// synthetic fixture (a guard that cannot fire is decoration, not
    /// enforcement).
    #[test]
    fn sql_statement_counter_still_fires() {
        assert_eq!(count_sql_statements(""), 0);
        assert_eq!(
            count_sql_statements("SELECT a FROM t; insert into u values (1);"),
            2
        );
        assert_eq!(
            count_sql_statements(
                "// the review-queue SELECT extracted from the handler\nlet x = 1;"
            ),
            1,
            "comment residue counts too — substring semantics are deliberate"
        );
        assert_eq!(
            count_sql_statements("UPDATE t SET a = 1; DELETE FROM u;"),
            2
        );
        assert_eq!(
            count_sql_statements("no keywords here, updates deferred indefinitely"),
            0,
            "`updates` (no following space) is not a statement opener"
        );
    }

    /// The layer contract as an executable grep: production source under
    /// `src/service/` never names transport types — no HTTP-framework
    /// identifiers, no HTTP status type, no body-wrapper, no server state,
    /// no pool handle. `#[cfg(test)]` regions are exempt (pins may name
    /// what they refute). Born a hard error at the Plumb pin (there was
    /// never a warning phase); renamed to its line-plan name at the
    /// Cornerstone flip.
    #[test]
    fn service_layer_free_of_http_types() {
        const FORBIDDEN: &[&str] = &["axum", "StatusCode", "Json", "AppState", "Pool"];
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service");
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("src/service must exist")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
            .collect();
        assert!(
            files.len() >= 2,
            "sanity: expected the service tree (mod + at least one core), found {}",
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
                .strip_prefix(&dir)
                .unwrap_or(f)
                .to_string_lossy()
                .into_owned();
            for token in FORBIDDEN {
                assert!(
                    !prod.contains(token),
                    "layer violation in src/service{display}: production source names \
                     the transport type `{token}` — services take connections and \
                     return domain types; map to HTTP at the handler boundary"
                );
            }
        }
    }

    /// v1.28.49 "Terrace" — the register surfaces' compile-time + source
    /// proof that the pool authority cannot leak into the register family:
    ///
    /// 1. TYPE-LEVEL: every core storage fn of `service::register` coerces
    ///    to a plain fn pointer taking a connection/transaction FIRST — if a
    ///    future edit changes a signature to take the registry, a pool
    ///    handle, or server state, these coercions stop compiling (the
    ///    signature no longer unifies).
    /// 2. SOURCE-LEVEL: production source of `register.rs` +
    ///    `domains_admin.rs` never names the registry type, a transport
    ///    type, or a handler type (the same token walk the lifecycle pin
    ///    runs over its subtree). `#[cfg(test)]` regions are exempt — pins
    ///    may name what they refute.
    #[test]
    fn register_services_receive_no_registry() {
        // (1) fn-pointer coercions = compile-time signature proof. Each alias
        // IS the pinned signature — if a fn's parameters change to take a
        // registry/pool/state handle, the coercion below stops compiling.
        use crate::service::domains_admin as da;
        use crate::service::register as reg;
        type ConnFn<R> = fn(&rusqlite::Connection) -> R;
        type ListFor = fn(
            &rusqlite::Connection,
            Option<&[String]>,
        ) -> Result<Vec<reg::Client>, reg::RegisterError>;
        type ByName =
            fn(&rusqlite::Connection, &str) -> Result<Option<reg::Client>, reg::RegisterError>;
        type ActiveClient =
            fn(&rusqlite::Connection, &str) -> Result<reg::Client, reg::RegisterError>;
        type Archive = fn(&rusqlite::Transaction, &str, i64) -> Result<bool, reg::RegisterError>;
        type SetDpa =
            fn(&rusqlite::Transaction, &str, &reg::DpaTerms) -> Result<usize, reg::RegisterError>;
        type Scaffold = fn(
            &rusqlite::Transaction,
            &str,
            &str,
            &str,
            Option<&str>,
            i64,
        ) -> Result<(), reg::RegisterError>;
        type Coach = fn(
            &rusqlite::Transaction,
            &str,
            i64,
            Option<String>,
            bool,
        ) -> Result<usize, reg::RegisterError>;
        type Export =
            fn(&rusqlite::Connection, &str) -> Result<(Vec<u8>, String), da::DomainAdminError>;
        let _list: ConnFn<Result<Vec<reg::Client>, reg::RegisterError>> = reg::list;
        let _granted: ListFor = reg::list_for_domain_grants;
        let _by_name: ByName = reg::by_name;
        let _active: ActiveClient = reg::require_active_client;
        let _archive: Archive = reg::archive;
        let _dpa: SetDpa = reg::set_dpa_terms;
        let _scaffold: Scaffold = reg::scaffold_and_register;
        let _coach: Coach = reg::coach_note;
        let _shim_rows: ConnFn<Result<Vec<da::DomainRow>, da::DomainAdminError>> =
            da::shim_domain_rows;
        let _file_counts: ConnFn<(i64, i64, i64)> = da::file_domain_counts;
        let _empty: ConnFn<bool> = da::is_empty_store;
        let _vacuum: ConnFn<Result<(), da::DomainAdminError>> = da::vacuum;
        let _export: Export = da::export_snapshot;

        // (2) the token walk over the register family's production source.
        const FORBIDDEN: &[&str] = &[
            "DomainRegistry",
            "AppState",
            "Pool",
            "axum",
            "StatusCode",
            "Json",
            "HandlerError",
        ];
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service");
        let mut files = vec![base.join("register.rs"), base.join("domains_admin.rs")];
        assert!(
            files.iter().all(|f| f.exists()),
            "sanity: the register-family files must exist"
        );
        files.sort();
        for f in &files {
            let text = std::fs::read_to_string(f).expect("service file must be readable");
            let prod = text
                .split("#[cfg(test)]")
                .next()
                .expect("split always yields a first slice");
            let display = f
                .strip_prefix(&base)
                .unwrap_or(f)
                .to_string_lossy()
                .into_owned();
            for token in FORBIDDEN {
                assert!(
                    !prod.contains(token),
                    "layer violation in src/service/{display}: production source names \
                     `{token}` — the register cores take connections and return domain \
                     types; the registry/pool authority and HTTP mapping stay at the handler"
                );
            }
        }
    }

    /// v1.28.50 "Aqueduct" — the recall core's compile-time + source proof
    /// that the recall service is handler-free and transport-free:
    ///
    /// 1. TYPE-LEVEL: every core fn of `service::recall` coerces to a plain
    ///    fn pointer whose parameters are connections, pure inputs, and
    ///    domain types — if a future edit hands the core the pool authority,
    ///    server state, or a handler type, these coercions stop compiling.
    ///    The generic guard parameter of `record_recall_read_event` is
    ///    coerced through a test-local `Deref<Target = Connection>` guard to
    ///    prove the core accepts ANY connection guard and names none.
    /// 2. SOURCE-LEVEL: production source never names a transport type, the
    ///    pool authority, or a handler type. `#[cfg(test)]` regions are
    ///    exempt — pins may name what they refute.
    #[test]
    fn recall_core_is_handler_free() {
        use crate::service::recall as rc;

        struct Guard(rusqlite::Connection);
        impl std::ops::Deref for Guard {
            type Target = rusqlite::Connection;
            fn deref(&self) -> &rusqlite::Connection {
                &self.0
            }
        }

        type Merge = fn(
            Vec<(String, Vec<crate::search::SearchResult>)>,
            usize,
        ) -> Vec<(crate::search::SearchResult, String)>;
        type DomainFilters = fn(
            &crate::search::SearchFilters,
            &str,
            bool,
            &std::collections::HashMap<String, Vec<(String, i64)>>,
        ) -> crate::search::SearchFilters;
        type Finish =
            fn(Option<&rusqlite::Connection>, &mut [crate::search::SearchResult], &str, bool, bool);
        type ReadEvent = fn(&rusqlite::Connection, [Guard; 1], rc::ReadEvent<'_>) -> Option<i64>;
        let _merge: Merge = rc::rrf_merge_domains;
        let _filters: DomainFilters = rc::domain_filters;
        let _finish: Finish = rc::finish_domain_results;
        let _read_event: ReadEvent = rc::record_recall_read_event::<Guard, [Guard; 1]>;

        const FORBIDDEN: &[&str] = &[
            "DomainRegistry",
            "AppState",
            "Pool",
            "axum",
            "StatusCode",
            "Json",
            "HandlerError",
            "handlers::",
        ];
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service/recall.rs");
        assert!(f.exists(), "sanity: the recall core must exist");
        let text = std::fs::read_to_string(&f).expect("service file must be readable");
        let prod = text
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first slice");
        for token in FORBIDDEN {
            assert!(
                !prod.contains(token),
                "layer violation in src/service/recall.rs: production source names \
                 `{token}` — the recall core takes connections and domain types; \
                 the pool schedule and HTTP mapping stay at the handler"
            );
        }
    }

    /// v1.28.50 "Aqueduct" — the ingest core's compile-time + source proof
    /// that the structured-ingest service is handler-free and
    /// transport-free:
    ///
    /// 1. TYPE-LEVEL: every core fn of `service::ingest` coerces to a plain
    ///    fn pointer whose parameters are connections/transactions, pure
    ///    inputs, and domain types — if a future edit hands the core the
    ///    pool authority, server state, or a handler type, these coercions
    ///    stop compiling.
    /// 2. SOURCE-LEVEL: production source never names a transport type, the
    ///    pool authority, or a handler type. `#[cfg(test)]` regions are
    ///    exempt — pins may name what they refute.
    #[test]
    fn ingest_core_is_handler_free() {
        use crate::service::ingest as ig;

        type Ttl = fn(Option<i64>, Option<i64>, i64) -> Result<Option<i64>, ig::IngestError>;
        type Profile = fn(
            Option<&brain_server::profile::Profile>,
            &str,
            &str,
            Option<String>,
            Option<&str>,
        ) -> Result<(String, String, Option<String>), ig::IngestError>;
        type Screen = fn(&str, &str, Option<&str>, Option<&str>) -> Result<bool, ig::IngestError>;
        type Store = fn(
            &rusqlite::Transaction<'_>,
            &ig::StoreRecord<'_>,
        ) -> Result<ig::StoreOutcome, ig::IngestError>;
        let _ttl: Ttl = ig::ttl_days_to_expires;
        let _profile: Profile = ig::apply_profile_ingest;
        let _screen: Screen = ig::screen_structured;
        let _store: Store = ig::store_record;

        const FORBIDDEN: &[&str] = &[
            "DomainRegistry",
            "AppState",
            "Pool",
            "axum",
            "StatusCode",
            "Json",
            "HandlerError",
            "handlers::",
        ];
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service/ingest.rs");
        assert!(f.exists(), "sanity: the ingest core must exist");
        let text = std::fs::read_to_string(&f).expect("service file must be readable");
        let prod = text
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first slice");
        for token in FORBIDDEN {
            assert!(
                !prod.contains(token),
                "layer violation in src/service/ingest.rs: production source names \
                 `{token}` — the ingest core takes transactions and domain types; \
                 the transport and HTTP mapping stay at the handler"
            );
        }
    }
}
