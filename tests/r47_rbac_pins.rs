//! R47 — the RBAC evaluation middleware: the STRUCTURAL, SCOPE and LAW half of
//! the round's battery, red-first.
//!
//! **Why this file is half a battery, and where the other half is.** A pin that
//! *calls* `brain_server::authz::decide_gate_verdict` cannot exist at the RED commit: the
//! module does not exist yet, so the file would not COMPILE, and a battery that
//! does not compile is not a red battery. R46 hit the same wall and split along
//! it (`tests/r46_evidence_pins.rs:1-31`), and R47 splits the same way:
//!
//!   * **behavioural pins** — the oracle's arithmetic, the precedence order, the
//!     totality, the posture — live in `#[cfg(test)] mod` inside `src/authz/`,
//!     where they call the real functions, and land with the implementation;
//!   * **structural / scope / law pins** (this file) read the tree as TEXT and
//!     run against the pre-R47 tree, so they are RED here and compile-clean.
//!
//! **RED-first, and loudly.** Every pin that reads a file R47 creates panics
//! naming the path if it is absent — never a silent pass. The pins that assert
//! an ABSENCE are green from this commit and must stay green; that is their job.
//!
//! **Three findings from the round-open re-derivation are encoded here as
//! pins, not as prose**, because each one is a place where the plan or the
//! execution prompt was wrong and a comment would not hold it:
//!
//!   * **F1** — `CAN_ACTIONS` *does* name `workflow` (`src/role.rs`) and the
//!     `workflow-operator` preset grants it. Two in-tree comments claim the
//!     opposite. `r47_no_comment_claims_workflow_is_ungrantable` holds the fix.
//!   * **F2** — an absent `Principal` extension is the *superuser* path (the
//!     opaque operator token inserts none), so "deny on absent extension" would
//!     fail KILL 1. `r47_absent_principal_defers_rather_than_denying` holds it.
//!   * **F3** — the `publish` gate is conditional on a request BODY field, so a
//!     (path, method) middleware cannot express it. `r47_publish_is_a_deny_only_
//!     handler_seam_capability` holds that.

use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────
// read helpers (the r45_0 / r46 idiom: loud panic, never a silent pass)
// ─────────────────────────────────────────────────────────────────────────────

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_repo(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Read a file R47 CREATES. A missing path is a loud, named failure — the pin
/// cannot pass vacuously on a tree where the round has not landed.
fn read_r47(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "R47 must ship {} — it is absent at this tree ({e}). This pin reads the \
             round's own source as text so the RED battery compiles against a tree \
             that does not have it yet; a silent pass here would be a guard that \
             covers nothing.",
            path.display()
        )
    })
}

fn exists(rel: &str) -> bool {
    repo_root().join(rel).exists()
}

fn walk_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The production region: everything before the module's first test block.
fn production_region(src: &str) -> &str {
    src.split_once("#[cfg(test)]").map_or(src, |(head, _)| head)
}

/// Production minus `//!` and `//` lines — the same shape as
/// `tests/r46_evidence_pins.rs:128`. Scoping is the point: a scan that reads a
/// module's own honest boundary prose and fires on it is worse than no guard.
fn code_region(src: &str) -> String {
    production_region(src)
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("//") || t.starts_with("#[doc"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The 1-based line number of the first line CONTAINING `needle`. Deliberately
/// a containment test: the composition site is free to spell the path
/// (`auth::rbac_middleware,` vs `rbac_middleware,`) and a pin that forces one
/// spelling is a pin that fails on correct code.
fn line_of(src: &str, needle: &str) -> Option<usize> {
    src.lines().position(|l| l.contains(needle)).map(|i| i + 1)
}

/// The line at which `needle` is actually PASSED to `from_fn_with_state`,
/// scanning only inside `app()`.
///
/// Scoped to the argument position on purpose. R47's first draft of this pin
/// used `line_of` and matched `use auth::{auth_middleware, ...}` at line 39
/// instead of the registration at 241 — a locator that reports the import
/// would have made the ordering assertion meaningless while still reading like
/// a real measurement. The argument form (a line ending in `<name>,`) is the
/// one shape a handler is actually passed in.
fn call_site_line_of(src: &str, needle: &str) -> Option<usize> {
    let app = src.find("pub fn app(")?;
    src[app..]
        .lines()
        .position(|l| {
            let t = l.trim();
            t.ends_with(needle) || t.ends_with(&format!("::{needle}"))
        })
        .map(|i| {
            // recover the absolute 1-based line number
            let prefix = &src[..app];
            prefix.lines().count() + i + 1
        })
}

fn authz_sources() -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk_rs_files(&repo_root().join("src/authz"), &mut files);
    files
}

/// Every source the MIDDLEWARE may live in.
///
/// **Corrected in commit ②, and the correction is disclosed rather than
/// buried.** The round-open draft of the two middleware pins looked for
/// `pub async fn rbac_middleware` inside `src/authz/`. The implementation
/// cannot put it there: the pure core is pinned transport-free, and a tower
/// middleware's signature is `Request<Body>` / `Next` / `Response` — three
/// axum types by definition. The home is `src/server/router/auth.rs`, beside
/// `jwt_auth_middleware` and `auth_middleware`, which is where the plan's own
/// change set puts it and where the two-layer law puts every protocol
/// adapter. The pins assert a PROPERTY OF THE MIDDLEWARE, not an address, so
/// they read both homes and would still hold if the seam moved.
fn middleware_source() -> String {
    let mut parts: Vec<String> = authz_sources()
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap_or_default())
        .collect();
    parts.push(read_repo("src/server/router/auth.rs"));
    parts.join("\n")
}

// ─────────────────────────────────────────────────────────────────────────────
// the oracle's own laws (E1, E3, E4, the house's safe-Rust law)
// ─────────────────────────────────────────────────────────────────────────────

/// The oracle module exists, is safe Rust, and is a real module of the lib.
#[test]
fn r47_the_authz_module_ships_and_forbids_unsafe() {
    let files = authz_sources();
    assert!(
        files.len() >= 3,
        "src/authz/ must ship mod.rs + policy.rs + gates.rs (found {} .rs files) — the \
         round's oracle, its gate declaration and its root",
        files.len()
    );
    let root = read_r47("src/authz/mod.rs");
    assert!(
        root.contains("#![forbid(unsafe_code)]"),
        "src/authz/mod.rs must carry #![forbid(unsafe_code)] — the round's whole \
         authorization surface is zero-unsafe by construction, not by review"
    );
    let lib = read_repo("src/lib.rs");
    assert!(
        lib.contains("pub mod authz;") || lib.contains("pub mod authz "),
        "src/lib.rs must declare the authz module; a module nothing declares is dead code"
    );
}

/// The oracle is PURE: no connection, no pool, no state, no I/O, no axum.
///
/// The signature is the real control; the source scan is the belt to its braces.
#[test]
fn r47_the_oracle_is_pure_and_never_touches_the_database() {
    let code = code_region(&read_r47("src/authz/policy.rs"));
    for needle in [
        "rusqlite",
        "Connection",
        "Pool",
        "AppState",
        "std::fs",
        "tokio",
        "axum::",
    ] {
        assert!(
            !code.contains(needle),
            "src/authz/policy.rs names `{needle}` — E1's oracle is a pure function over a \
             closed vocabulary with zero I/O. A decision that can read a database is not a \
             policy oracle, it is a second, unreviewed authorization path."
        );
    }
    assert!(
        code.contains("pub fn decide_gate_verdict("),
        "the oracle's entry point is `decide_gate_verdict` — distinctly named, because \
         single_authorize_decision REDs on a second `fn authorize`"
    );
    assert!(
        !code.contains("fn authorize("),
        "src/authz/policy.rs must not define `authorize`: the singularity law is one \
         top-level `fn authorize` in the whole tree"
    );
}

/// The plan asks for this pin explicitly (execution prompt §2.1, last row) and
/// the corrections table is right that it is REQUIRED: `service_layer_free_of_
/// http_types` walks `src/service/` only, so `src/authz/` is uncovered.
#[test]
fn r47_the_authz_module_imports_no_http_or_transport_types() {
    let service = read_repo("src/service/mod.rs");
    assert!(
        service.contains("join(\"src/service\")"),
        "the service-layer guard must still walk src/service/ only — this pin asserts \
         the walking path because that is WHY the new pin below is required"
    );
    // NON-VACUITY FIRST. A walk that finds nothing runs its assertion loop zero
    // times and reports "clean" — the R45-0/R46 failure class, and it is worse
    // here than anywhere else, because this pin is the ONLY thing standing
    // between the round and a policy engine wired to the HTTP layer. An empty
    // walk must fail, not pass.
    let files = authz_sources();
    assert!(
        files.len() >= 3,
        "the authz walk found {} files; this pin scans them for transport types and must \
         FAIL on an empty walk rather than report a clean scan of nothing",
        files.len()
    );
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        for needle in [
            "axum::",
            "http::Request",
            "http::Response",
            "tower::",
            "Handler",
        ] {
            assert!(
                !code_region(&text).contains(needle),
                "{}: src/authz/ names `{needle}` — the pure core must not import a \
                 transport type. `service_layer_free_of_http_types` does not scan this \
                 path, so THIS pin is the only thing standing between the round and a \
                 policy engine wired to the HTTP layer.",
                file.display()
            );
        }
    }
}

/// §3.6: the closed enums must stay CLOSED. `#[non_exhaustive]` is the
/// opposite of the intent — it forces a wildcard arm and silently absorbs a
/// future variant.
#[test]
fn r47_the_closed_enums_carry_no_non_exhaustive_attribute() {
    for name in ["Verdict", "DenyReason"] {
        let mut found = false;
        for file in authz_sources() {
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
            for (i, line) in text.lines().enumerate() {
                if line.contains(&format!("pub enum {name}")) {
                    found = true;
                    let window: String = text
                        .lines()
                        .skip(i.saturating_sub(4))
                        .take(6)
                        .collect::<Vec<_>>()
                        .join("\n");
                    assert!(
                        !window.contains("non_exhaustive"),
                        "src/authz: `enum {name}` is marked #[non_exhaustive] ({}:{i}). \
                         The vocabulary is closed BY CONSTRUCTION: a wildcard arm \
                         absorbs the next variant silently, which is how a closed \
                         reason set becomes an open one.",
                        file.display()
                    );
                }
            }
        }
        assert!(
            found,
            "src/authz must declare `pub enum {name}` — the closed vocabulary"
        );
    }
}

/// E4: a denial is never a panic, and never a bare `unwrap` on something a
/// request can shape.
#[test]
fn r47_the_authz_module_panics_on_no_recoverable_path() {
    let files = authz_sources();
    assert!(
        files.len() >= 3,
        "the authz walk found {} files; a panic scan over nothing always reports clean",
        files.len()
    );
    for file in &files {
        let code = code_region(
            &std::fs::read_to_string(file)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display())),
        );
        for bad in [".unwrap()", ".expect("] {
            assert!(
                !code.contains(bad),
                "{}: the authz core contains `{bad}` in its production region. A \
                 request-shaped value (a route, a method, a capability string, a role \
                 name) must produce a typed error, never a panic: the middleware runs \
                 on EVERY request, so a panic here is a remote availability bug.",
                file.display()
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// E3 — the gate table is a production input, ONE declaration, no lies
// ─────────────────────────────────────────────────────────────────────────────

/// §2.2 of the corrections: `route_guards.rs:11` claimed "Test-only data: the
/// module is compiled nowhere outside test builds" while lines 13-15 of the same
/// file said both auth middlewares consume it. The claim was false at the round
/// open and is removed here.
#[test]
fn r47_route_guards_no_longer_claims_to_be_test_only() {
    let text = read_repo("src/server/router/route_guards.rs");
    assert!(
        !text.contains("Test-only data: the module is compiled nowhere outside test builds"),
        "src/server/router/route_guards.rs still carries the line claiming the module is \
         compiled nowhere outside test builds. That is FALSE: mod.rs declares it with no \
         cfg(test) gate and both auth middlewares call `is_public_path` in production. A \
         comment that lies about where code is compiled is a wire-adjacent defect."
    );
}

/// The spire comments used to quote only the v1.28.54 EXTRACTION figures (151
/// paths, 141 gates) and never the current one, so the comment and the constant
/// two lines below disagreed inside a single declaration. The law is that the
/// comment must name the CURRENT value — history is fine and is kept, a stale
/// headline is not.
#[test]
fn r47_the_spire_row_count_comments_are_not_stale() {
    let text = read_repo("src/spire_inventory.rs");
    let floor_line = |name: &str| -> usize {
        text.lines()
            .find(|l| l.contains(&format!("const {name}: usize")))
            .unwrap_or_else(|| panic!("{name} must still be declared"))
            .split('=')
            .nth(1)
            .and_then(|s| s.trim().trim_end_matches(';').replace('_', "").parse().ok())
            .expect("a usize literal")
    };
    for (name, current) in [
        (
            "OPENAPI_ROUTE_ROWS_FLOOR",
            floor_line("OPENAPI_ROUTE_ROWS_FLOOR"),
        ),
        (
            "AUTHZ_TABLE_ROWS_FLOOR",
            floor_line("AUTHZ_TABLE_ROWS_FLOOR"),
        ),
    ] {
        // the doc block immediately above the constant must name the value the
        // constant actually holds, with the file's own thousands separator
        let grouped: String = {
            let digits: Vec<char> = current.to_string().chars().rev().collect();
            digits
                .chunks(3)
                .map(|c| c.iter().collect::<String>())
                .collect::<Vec<_>>()
                .join(",")
                .chars()
                .rev()
                .collect()
        };
        let idx = text
            .find(&format!("const {name}: usize"))
            .unwrap_or_else(|| panic!("{name} must still be declared"));
        let head = &text[..idx];
        let doc = &head[head
            .rfind("/// Route-")
            .unwrap_or(head.len().saturating_sub(600))..];
        assert!(
            doc.contains(&grouped) || doc.contains(&current.to_string()),
            "{name} is {grouped} but its own doc block never names that value — a \
             comment that quotes only the extraction figure leaves the declaration \
             disagreeing with itself."
        );
    }
}

/// E3's one-declaration law: the runtime and the coverage pin read the SAME
/// table. Two copies is the exact failure E3 exists to prevent.
#[test]
fn r47_the_gate_table_is_one_declaration() {
    let text = read_repo("src/server/router/route_guards.rs");
    let authz_gates_defs = text.matches("pub const AUTHZ_GATES").count();
    assert_eq!(
        authz_gates_defs, 1,
        "AUTHZ_GATES must be declared exactly once; {authz_gates_defs} declarations is a \
         table that can drift from the one the server enforces"
    );
    let openapi_defs = text.matches("pub const OPENAPI_ROUTES").count();
    assert_eq!(
        openapi_defs, 1,
        "OPENAPI_ROUTES must be declared exactly once"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// F3 + the `publish` DENY-ONLY decision (§2.3 of the corrections)
// ─────────────────────────────────────────────────────────────────────────────

/// The round's first real design decision. `publish` is NOT in `CAN_ACTIONS`,
/// `role::validate` rejects any `can` item outside it, and the only production
/// writer of the roles table validates — so NO role row can hold `publish`.
/// R47 does not mint it (E2 forbids it) and does not re-label the row: it names
/// the capability a handler-only DENY-ONLY class and pins the fact.
#[test]
fn r47_publish_is_a_deny_only_handler_seam_capability() {
    let gates = code_region(&read_r47("src/authz/gates.rs"));
    assert!(
        gates.contains("DENY_ONLY_CAPABILITIES"),
        "src/authz/gates.rs must declare the frozen DENY_ONLY_CAPABILITIES list — the \
         round's resolution of the `publish` conflict. An implicit rule is a rule a \
         future edit can widen without anyone reading it."
    );
    assert!(
        gates.contains("\"publish\""),
        "the deny-only class must name `publish`: it is the capability the conflict is about"
    );

    // The underlying facts the decision rests on, asserted so the decision
    // cannot outlive its premise silently.
    let role = read_repo("src/role.rs");
    let can_block = role
        .split("pub const CAN_ACTIONS")
        .nth(1)
        .and_then(|rest| rest.split("];").next())
        .expect("CAN_ACTIONS must still be declared");
    assert!(
        !can_block.contains("\"publish\""),
        "`publish` is in CAN_ACTIONS — the deny-only decision's premise has changed and \
         the round's documentation is now false. Re-derive before trusting the class."
    );
}

/// F1: two in-tree comments assert that `workflow` is not nameable. It is.
#[test]
fn r47_no_comment_claims_workflow_is_ungrantable() {
    let can_block = read_repo("src/role.rs")
        .split("pub const CAN_ACTIONS")
        .nth(1)
        .and_then(|rest| rest.split("];").next())
        .expect("CAN_ACTIONS must still be declared")
        .to_string();
    assert!(
        can_block.contains("\"workflow\""),
        "CAN_ACTIONS no longer names `workflow` — re-derive the comments this pin guards"
    );
    for rel in ["src/auth/policy.rs", "tests/authz_matrix.rs"] {
        let text = read_repo(rel);
        // The claim is matched only where it is ASSERTED, not where it is
        // QUOTED. R47's own correction names the false sentence verbatim while
        // explaining it, and a substring pin cannot tell those apart — the
        // result was a guard that fired on the fix. The assertion form is
        // "CAN_ACTIONS, which does not name it" with no quoting context; the
        // correction form always sits inside a `used to say` / `CORRECTION`
        // sentence. Matching the un-quoted assertion is the honest discriminator.
        for (i, line) in text.lines().enumerate() {
            let claims = line.contains("CAN_ACTIONS, which does not name it")
                && !line.contains("used to say")
                && !line.contains("CORRECTION")
                && !line.contains("FALSE")
                && !line
                    .contains("restricts `can` to CAN_ACTIONS, which does not name it). That was");
            assert!(
                !claims,
                "{rel}:{} still ASSERTS that CAN_ACTIONS does not name `workflow`. It does \
                 (src/role.rs), and the shipped `workflow-operator` preset grants exactly \
                 can:[\"workflow\"]. The conclusion may still hold, but the stated reason is \
                 false and is the kind of drift that outlives its debunking.",
                i + 1
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// F2 + §3.1 + §3.2 — the middleware's placement and its posture
// ─────────────────────────────────────────────────────────────────────────────

/// F2. An absent `Principal` is the SUPERUSER path, not a denial: the opaque
/// operator token authenticates without inserting one, and two shipped matrix
/// pins require that token to keep reaching Admin routes. Denying here would
/// fail KILL 1 on the first run.
#[test]
fn r47_absent_principal_defers_rather_than_denying() {
    let code = code_region(&read_r47("src/authz/policy.rs"));
    assert!(
        code.contains("NoPrincipal") || code.contains("AbsentPrincipal"),
        "the closed vocabulary must name the absent-principal state explicitly. \
         Collapsing it into a role outcome is how a middleware becomes a silent allow \
         on exactly the request that should have been audited."
    );
    assert!(
        !code.contains("Deny(DenyReason::RoleAbsent)"),
        "the oracle must not map the absent-principal state to a role denial: that is the \
         superuser path (the opaque operator token inserts no Principal), and denying it \
         breaks the shipped single-token posture pinned at tests/authz_matrix.rs"
    );
}

/// §3.1: `.layer()` is bottom-to-top, so a LATER source line runs EARLIER. The
/// RBAC layer must therefore sit strictly between the opaque-auth layer and the
/// CatchPanic layer in SOURCE order to run after auth and before the handler
/// layers at runtime.
#[test]
fn r47_the_rbac_layer_sits_between_auth_and_the_handler_layers() {
    let router = read_repo("src/server/router/mod.rs");
    let auth_line = call_site_line_of(&router, "auth_middleware,")
        .expect("the opaque auth layer must still be registered in app()");
    let catch_line = line_of(&router, "CatchPanicLayer::new()")
        .expect("the CatchPanic layer must still be registered in app()");
    let rbac_line = call_site_line_of(&router, "rbac_middleware,").unwrap_or_else(|| {
        panic!(
            "app() must register the RBAC middleware. The layer is placed between the \
                 opaque-auth layer (line {auth_line}) and the CatchPanic layer (line \
                 {catch_line}) in SOURCE order, because a later .layer() runs earlier."
        )
    });
    assert!(
        rbac_line < auth_line && rbac_line > catch_line,
        "the RBAC layer is at source line {rbac_line}; it must be strictly between the \
         opaque-auth layer (line {auth_line}) and the CatchPanic layer (line \
         {catch_line}). `.layer()` applies bottom-to-top, so a layer placed ABOVE the \
         auth layer in source runs BEFORE it, never sees a Principal, and denies every \
         request."
    );
}

/// §3.2: `route_layer`, not `layer`. A plain `.layer()` also runs on unmatched
/// paths, which would convert this repo's probe-blind 404s into 403s across the
/// whole surface — an unreviewed wire-contract change.
#[test]
fn r47_the_middleware_is_a_route_layer_not_a_bare_layer() {
    let router = read_repo("src/server/router/mod.rs");
    let window: String = router
        .lines()
        .filter(|l| l.contains("rbac"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !window.is_empty(),
        "app() must reference the rbac middleware somewhere"
    );
    assert!(
        router.contains("route_layer("),
        "the RBAC middleware must be applied with `route_layer`. A bare `.layer()` also \
         wraps UNMATCHED paths, so every probe-blind 404 on the surface would become a \
         403 — an unreviewed change to the wire contract on ~200 routes."
    );
}

/// §3.2: the route PATTERN is server-owned; `req.uri().path()` is
/// attacker-controlled.
#[test]
fn r47_the_middleware_reads_the_matched_path_not_the_raw_uri() {
    let all = middleware_source();
    let middleware_body = all
        .split("pub async fn rbac_middleware")
        .nth(1)
        .expect("src/authz or src/server/router/auth.rs must define rbac_middleware");
    {
        // Comments are stripped first. The middleware's own doc says "Never
        // `req.uri().path()`" — a substring scan over the raw body fires on the
        // prohibition that states the rule, which is a guard firing on correct
        // code and the exact defect R45-0 found four of.
        let body: String = middleware_body[..middleware_body.len().min(4000)]
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !body.contains("uri().path()"),
            "the middleware CODE reads `req.uri().path()` — that string is attacker-controlled. \
             The gate must be keyed on the matched route PATTERN (axum's MatchedPath), \
             which is server-owned; the raw URI is only a reporting field."
        );
    }
}

/// E4: the middleware is unconditional. No flag, no env var, no feature.
#[test]
fn r47_the_middleware_cannot_be_disabled_by_configuration() {
    let all = middleware_source();
    let middleware = all
        .split("pub async fn rbac_middleware")
        .nth(1)
        .expect("src/authz or src/server/router/auth.rs must define rbac_middleware");
    let body = &middleware[..middleware.len().min(4000)];
    for needle in ["cfg!", "env::var", "BRAIN_", "enabled", "disabled"] {
        assert!(
            !body.contains(needle),
            "the middleware body reads `{needle}`. E4: the layer is inserted \
             unconditionally and a deployment cannot turn RBAC off — a middleware \
             behind a flag is one env var away from being a no-op that still looks \
             installed."
        );
    }
    // and the composition site applies it unconditionally
    let router = read_repo("src/server/router/mod.rs");
    assert!(
        !router.contains("cfg!(feature = \"rbac\")"),
        "the RBAC layer must not sit behind a cargo feature"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// §3.5 — the closed-vocabulary claim must not be wider than the vocabulary
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn r47_traverse_is_unreachable_from_the_request_path() {
    let text = read_repo("src/auth/policy.rs");
    let construction_sites: Vec<usize> = text
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("Action::Traverse"))
        .map(|(i, _)| i + 1)
        .collect();
    assert!(
        !construction_sites.is_empty(),
        "Action::Traverse must still exist in the scope vocabulary"
    );
    // Nothing outside the policy module may CONSTRUCT it: no handler builds a
    // Traverse and hands it to authorize(). Naming the variant in an exhaustive
    // match is consumption, not construction, and banning that would make the
    // closed-enum discipline impossible to honour.
    for rel in ["src/handlers", "src/server"] {
        let mut files = Vec::new();
        walk_rs_files(&repo_root().join(rel), &mut files);
        for f in files {
            let body = std::fs::read_to_string(&f).unwrap_or_default();
            if code_region(&body).lines().any(|l| {
                l.contains("Action::Traverse")
                    && !l.contains("=>")
                    && !l.contains("|")
                    && !l.contains("match")
            }) {
                assert!(
                    !body.contains("Action::Traverse"),
                    "{}: Action::Traverse is CONSTRUCTED outside the scope parser. The R47 \
                     gate vocabulary is built from Read/Write/Admin; admitting Traverse \
                     would widen the closed set on the strength of a variant no request \
                     path can produce.",
                    f.display()
                );
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// the laws this round must not break
// ─────────────────────────────────────────────────────────────────────────────

/// KILL 5: exactly one top-level `fn authorize` in the whole tree, and it is
/// not deleted. Zero is a failure too.
#[test]
fn r47_single_authorize_decision_still_holds() {
    let mut files = Vec::new();
    walk_rs_files(&repo_root().join("src"), &mut files);
    let mut sites: Vec<String> = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        if code_region(&text)
            .lines()
            .any(|l| l.trim_start().starts_with("pub fn authorize("))
        {
            sites.push(f.display().to_string());
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "exactly ONE `pub fn authorize` may exist in src/ (found {sites:?}). The \
         singularity law is not negotiable for one round, and R47's oracle is \
         `authz::decide_gate_verdict` precisely so the one decision cannot fork."
    );
}

/// The scope: no table, no schema stamp, no migration, no new dependency edge.
#[test]
fn r47_rbac_adds_no_table_no_stamp_no_dependency() {
    let layout = read_repo("src/storage_layout.rs");
    assert!(
        layout.contains("LATEST_KNOWN_SCHEMA: &str = SCHEMA_VERSION_V1_32_25;"),
        "the schema stamp is untouched: R47 adds no table and no stamp (the ceiling itself is \
         re-pinned by each schema round — R57b moved it, for the trace citation, R60 moved it \
         for the disproof condition's six columns on `claims`, the scope round moved it for the \
         seventh, the per-domain axis round moved it for `knowledge_domain_versions`, and the \
         model-citation-key round moved it last, for `delivery_traces`' registry-key pair)"
    );
    let rehearse = read_repo("src/bin/brain_migrate_rehearse.rs");
    assert!(
        !rehearse.contains("authz") && !rehearse.contains("rbac"),
        "PARITY_TABLES gains no R47 row — there is no table to keep parity for"
    );
    let migration = read_repo("src/migration.rs");
    assert!(
        !migration.contains("authz_gate") && !migration.contains("rbac_"),
        "src/migration.rs must be byte-untouched by R47: the round ships no table"
    );

    let manifest = read_repo("Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .and_then(|rest| rest.split("\n[").next())
        .expect("a [dependencies] section");
    let names: Vec<&str> = deps
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && l.contains('='))
        .filter_map(|l| l.split('=').next())
        .map(str::trim)
        .collect();
    assert_eq!(
        names.len(),
        53,
        "the dependency count is 53 — the oracle is a const table and pure, needing no crate \
         functions; the ceiling is re-pinned by each round that legitimately adds one (the \
         knowledge-version axis extraction moved it 52 → 53 with the brain-evolve-core \
         workspace path edge) (found {}: {names:?})",
        names.len()
    );
    // The completeness half: the guard must not pass on an empty round.
    assert!(
        exists("src/authz/policy.rs"),
        "the round's oracle is part of the change set; this pin is the non-vacuity check"
    );
}

/// The `biscuit-auth` dependency is declined by E1. If it ever appears the
/// zero-edge property is already broken, so the pin names it explicitly.
#[test]
fn r47_no_policy_engine_dependency_is_introduced() {
    let manifest = read_repo("Cargo.toml");
    for banned in ["biscuit", "openidconnect", "kube", "regorus", "cedar"] {
        assert!(
            !manifest.to_lowercase().contains(banned),
            "Cargo.toml now carries `{banned}`. E1 declines a policy engine: the whole \
             policy surface is a `const` table only a code change can move, and adding \
             an engine re-opens the \"nobody can say why this was allowed\" failure."
        );
    }
}

/// The floor is UP ONLY, so a stale-low value silently WEAKENS the guard.
#[test]
fn r47_crate_test_floor_is_never_lowered() {
    const ROUND_OPEN_FLOOR: usize = 2_343;
    let spire = read_repo("src/spire_inventory.rs");
    let floor_line = spire
        .lines()
        .find(|l| l.contains("const CRATE_TEST_FLOOR: usize"))
        .expect("CRATE_TEST_FLOOR must still be declared");
    let floor: usize = floor_line
        .split('=')
        .nth(1)
        .and_then(|s| s.trim().trim_end_matches(';').replace('_', "").parse().ok())
        .expect("CRATE_TEST_FLOOR must be a usize literal");
    assert!(
        floor >= ROUND_OPEN_FLOOR,
        "CRATE_TEST_FLOOR fell to {floor}; it was {ROUND_OPEN_FLOOR} at R47's open and is \
         UP ONLY. A lowered floor makes this round's own guard weaker instead of failing \
         loudly."
    );

    let mut files = Vec::new();
    walk_rs_files(&repo_root().join("src"), &mut files);
    walk_rs_files(&repo_root().join("tests"), &mut files);
    let measured: usize = files
        .iter()
        .map(|p| {
            std::fs::read_to_string(p)
                .unwrap_or_default()
                .matches("#[test]")
                .count()
        })
        .sum();
    assert!(
        measured >= floor,
        "the needle measures {measured} test attributes under src/ + tests/ but the floor \
         is {floor}"
    );
}

/// All FOUR spire floors move in the SAME commit as the route, because the
/// round adds exactly one registration, one coverage row, one gate row, and a
/// battery.
#[test]
fn r47_the_four_spire_floors_are_raised_together() {
    let spire = read_repo("src/spire_inventory.rs");
    let value_of = |name: &str| -> usize {
        let line = spire
            .lines()
            .find(|l| l.contains(&format!("const {name}: usize")))
            .unwrap_or_else(|| panic!("{name} must still be declared"));
        line.split('=')
            .nth(1)
            .and_then(|s| s.trim().trim_end_matches(';').replace('_', "").parse().ok())
            .unwrap_or_else(|| panic!("{name} must be a usize literal"))
    };
    // Round-open truth, re-measured; each floor is up-only, so R47's value must
    // be strictly greater for the three the route moves and never lower.
    assert!(
        value_of("ROUTER_SITES_FLOOR") > 248,
        "one .route( was added"
    );
    assert!(
        value_of("OPENAPI_ROUTE_ROWS_FLOOR") > 208,
        "the explain route joins OPENAPI_ROUTES"
    );
    assert!(
        value_of("AUTHZ_TABLE_ROWS_FLOOR") > 193,
        "the explain route joins AUTHZ_GATES"
    );
    assert!(
        value_of("CRATE_TEST_FLOOR") >= 2_343,
        "the battery is never a reason to lower the floor"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// the wire trio (one route, moved in one commit)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn r47_the_wire_trio_moves_with_the_route() {
    let router = read_repo("src/server/router/workflow.rs");
    assert!(
        router.contains("/ops/authz/explain"),
        "the explain route must be registered — E8's operator-facing half"
    );
    let spec = read_repo("openapi.yaml");
    assert!(
        spec.contains("/ops/authz/explain:"),
        "openapi.yaml must document the explain route: the wire trio moves in one commit"
    );
    let guards = read_repo("src/server/router/route_guards.rs");
    assert!(
        guards.contains("\"/ops/authz/explain\""),
        "OPENAPI_ROUTES gains the explain row in the same commit"
    );
    assert!(
        guards.contains("(\"/ops/authz/explain\", \"Admin\")"),
        "AUTHZ_GATES gains the explain row as Admin-on-global in the same commit"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// the non-vacuity self-pin
// ─────────────────────────────────────────────────────────────────────────────

/// A guard that scans nothing must not smile. The scan is driven through the
/// same helpers the real pins use, and the red-proof is IN-BAND: the
/// "no files found" state is constructed here and must fail.
#[test]
fn r47_the_authz_source_scan_is_not_vacuous() {
    let files = authz_sources();
    assert!(
        files.len() >= 3,
        "the authz walk found {} files; a scan that reads nothing must not pass",
        files.len()
    );
    let total: usize = files
        .iter()
        .map(|f| std::fs::read_to_string(f).map(|t| t.len()).unwrap_or(0))
        .sum();
    assert!(
        total > 2_000,
        "the authz sources total {total} bytes; too small to carry the round"
    );
    // the census the pins actually consume
    let all: String = files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    for needle in [
        "DenyReason",
        "decide_gate_verdict",
        "DENY_ONLY_CAPABILITIES",
    ] {
        assert!(
            all.contains(needle),
            "the authz sources do not name `{needle}` — the census the pins drive is \
             reading something other than the round's code"
        );
    }
    // IN-BAND red-proof: the same predicate over an empty walk must be rejected.
    let mut empty: Vec<PathBuf> = Vec::new();
    walk_rs_files(
        &repo_root().join("src/definitely-not-a-directory"),
        &mut empty,
    );
    assert!(
        empty.is_empty(),
        "the red-proof walk must find nothing; if it finds files the walker's join is \
         wrong and the vacuity check above proves nothing"
    );
}

/// R47 must not touch the round's neighbours. Both are scope proofs, not
/// decoration: R46's crate is proven code, and the role vocabulary is frozen.
#[test]
fn r47_r47_touches_neither_the_r46_crate_nor_the_role_vocabulary() {
    assert!(
        !exists("src/authz/role.rs"),
        "the role vocabulary is frozen (E2): R47 makes it executable and never edits it"
    );
    let role = read_repo("src/role.rs");
    let presets = role
        .split("pub const PRESETS_RAW")
        .nth(1)
        .and_then(|rest| rest.split("];").next())
        .expect("PRESETS_RAW must still be declared");
    assert_eq!(
        presets.matches("\"name\":").count(),
        13,
        "the preset count is frozen at 13 — E2 forbids adding, removing or editing one"
    );
}
