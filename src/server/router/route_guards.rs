//! The route guard tables, as plain data.
//!
//! Extracted verbatim from the `#[cfg(test)]` block of `main.rs`, where they
//! sat buried at line ~12k: the route-coverage table (every path the
//! composed router registers — `test_openapi_covers_routes` asserts each is
//! documented in `openapi.yaml`) and the route-authz table (every
//! non-public route's expected `authorize()` action —
//! `authz_gates_cover_every_non_public_route` source-scans each handler for
//! its gate). Row counts are floored in `spire_inventory`; rows are added or
//! removed only in the same commit as the route/wire change that earns it.
//!
//! **This module is PRODUCTION code, and the line that said otherwise is
//! gone.** It used to close with "Test-only data: the module is compiled
//! nowhere outside test builds" — which contradicted lines 13-15 of this very
//! file ("Both auth middlewares consume it through `is_public_path`") and was
//! false: `server/router/mod.rs` declares the module `pub` with no `cfg(test)`
//! gate, `server/router/auth.rs` calls `is_public_path` from both middlewares,
//! and `spire_inventory.rs` reads both tables from a non-test path. this round removed
//! the sentence and added `r47_route_guards_no_longer_claims_to_be_test_only`,
//! because a comment that lies about WHERE CODE IS COMPILED is a wire-adjacent
//! defect, not a style note — and it is exactly what stopped a reviewer from
//! noticing that the RBAC middleware could read this table at all.

/// The ONE public-path list. Both auth middlewares consume it through
/// [`is_public_path`] — there is no second copy to drift (the old
/// duplicate-per-middleware `matches!` blocks are gone; the middlewares'
/// sources carry only the `is_public_path` call). The list lives beside the
/// tables it feeds: the reverse-direction guard exempts these paths from
/// `AUTHZ_GATES`, so the exemption and the tables read from one place.
pub const PUBLIC_PATHS: &[&str] = &[
    "/health",
    "/ready",
    "/version",
    "/openapi.yaml",
    // OIDC discovery + JWKS are public by design (clients need them to
    // verify tokens; can't require a token to learn how to verify tokens).
    // `/auth/refresh` verifies its own refresh token. `/auth/logout` is NOT
    // public: it revokes the presented access token, so the middleware must
    // verify the bearer first — a public logout could revoke nothing and
    // silently "succeed" (the handler reads the principal from the
    // extension; with no principal it 401s unconditionally).
    "/.well-known/openid-configuration",
    "/.well-known/jwks.json",
    "/.well-known/security.txt",
    "/.well-known/ai-notice",
    "/.well-known/ai-literacy",
    "/.well-known/cop-notice",
    "/.well-known/ump.json",
    "/ump/capabilities",
    "/auth/refresh",
];

/// The webhook seams: routes exempt from the bearer middleware because they
/// are authenticated by their OWN in-handler verification (an HMAC signature
/// over the raw body — GitHub and the channel bridges cannot present a brain
/// bearer token, and inventing one for them would defeat the point).
///
/// **F8-06: this list is EXPLICIT where it was `path.starts_with("/webhooks/")`.**
/// The prefix rule exempted every route under `/webhooks/` from authN + authZ,
/// including any route added there in future — so the exemption was a
/// convention nobody was forced to honour, and a handler registered at
/// `/webhooks/anything` would have inherited "public" without ever verifying a
/// signature. That is not a hole today (all six verify and fail closed) and it
/// is exactly the "machine is right for the wrong reason" shape: right by
/// convention, not by enforcement.
///
/// Naming each route makes the exemption auditable: a new webhook route is NOT
/// automatically public, it must be added here — and adding it is the moment
/// the author states, in this file, that the handler verifies its own
/// signature. `r70_every_webhook_route_is_named_and_verifies` is the pin.
///
/// Method-independent, like every entry here: CORS preflight (`OPTIONS`) is
/// exempted separately in each middleware.
pub const WEBHOOK_PATHS: &[&str] = &[
    // GitHub + Signal + kb-feedback + the delivery observation sub-family —
    // one handler, four internal branches, each verifying in-handler
    // (`webhooks.rs:receive`).
    "/webhooks/{kind}",
    "/webhooks/delivery/{kind}",
    // The channel bridge family — Standard Webhooks HMAC per discovered
    // per-config secret (`channel_webhook.rs:verify_bridge`, called by all
    // four handlers).
    "/webhooks/channel/{kind}",
    "/webhooks/channel/{kind}/drain",
    "/webhooks/channel/{kind}/drain/ack",
    "/webhooks/channel/{kind}/console",
];

/// The public-path decision both auth middlewares run: exact [`PUBLIC_PATHS`]
/// entries, the EXPLICIT [`WEBHOOK_PATHS`] list, then the rules that can't be a
/// const list — the client SPA seat (static assets, no data) and the root
/// redirect. Method-independent: CORS preflight (`OPTIONS`) is exempted
/// separately in each middleware.
pub fn is_public_path(path: &str) -> bool {
    PUBLIC_PATHS.contains(&path)
        || is_webhook_path(path)
        || path == "/"
        // `/app` or `/app/...` — exact segment match so a future route like
        // `/apple` can never ride the prefix silently (2026-09-11 fix).
        || path == "/app"
        || path.starts_with("/app/")
}

/// Does this request path address one of the declared [`WEBHOOK_PATHS`]?
///
/// **Why a template match and not `contains`.** The three `is_public_path`
/// call sites do NOT agree on what they pass: `server/router/auth.rs:129`
/// passes axum's `MatchedPath` (the TEMPLATE, `/webhooks/{kind}`), while
/// `:277` and `:549` pass `req.uri().path()` (the CONCRETE path,
/// `/webhooks/github`). An exact `contains` on templates would therefore have
/// exempted the requests and REFUSED the concrete ones — silently disabling
/// every webhook, which is fail-OPEN's mirror and just as bad. So both forms
/// resolve here.
///
/// The match is **segment-wise**, not a prefix: `/webhooks/{kind}` must not
/// admit `/webhooks/channel/foo` (three segments), and `/webhooks/{kind}`
/// must not admit `/webhooks/` + an empty segment. A `{param}` segment
/// matches exactly one non-empty segment; every other segment must be equal.
pub fn is_webhook_path(path: &str) -> bool {
    WEBHOOK_PATHS
        .iter()
        .any(|template| matches_path_template(template, path))
}

/// Does `path` address `template`, matching `{param}` segments one-for-one?
fn matches_path_template(template: &str, path: &str) -> bool {
    let mut t = template.split('/');
    let mut p = path.split('/');
    loop {
        match (t.next(), p.next()) {
            (None, None) => return true,
            (None, _) | (_, None) => return false,
            (Some(seg_t), Some(seg_p)) => {
                // A whole parameter segment (`{kind}`) matches exactly one
                // NON-EMPTY segment. The check is on the WHOLE segment, not on
                // `split('/')` equality — `{kind}` arrives as one piece, so a
                // literal comparison would never see it and would reject every
                // real request (`/webhooks/github` vs `/webhooks/{kind}`).
                let is_param = seg_t.starts_with('{') && seg_t.ends_with('}');
                if is_param {
                    if seg_p.is_empty() {
                        // `/webhooks/` must not satisfy `/webhooks/{kind}`.
                        return false;
                    }
                } else if seg_t != seg_p {
                    return false;
                }
            }
        }
    }
}

/// Every path registered by `build_app`, in registration order.
/// Consumed by `test_openapi_covers_routes` (openapi.yaml coverage pin).
pub const OPENAPI_ROUTES: &[&str] = &[
    "/health",
    "/health/db",
    "/ready",
    "/openapi.yaml",
    "/stats",
    "/version",
    "/add",
    "/ingest/memory",
    "/search",
    "/v1/embeddings",
    "/ingest/markdown",
    "/reindex",
    "/get/{id}",
    "/multi-get",
    "/graph/entity/{name}",
    "/graph/relations",
    "/graph/traverse",
    "/graph/relationships/{id}/history",
    "/recall",
    "/ingest",
    "/memory/{id}",
    "/domains",
    // per-domain lifecycle
    "/domains/{name}",
    "/domains/{name}/vacuum",
    "/domains/{name}/export",
    "/domains/{name}/import",
    // bulk relabel across domains.
    "/domains/move",
    // one-shot recompute sweep.
    "/domains/recompute",
    // the preset API + the domain binding.
    "/profiles",
    "/profiles/{name}",
    "/domains/{name}/profile",
    "/roles",
    "/roles/{name}",
    "/legal-hold",
    "/legal-hold/{id}/release",
    "/legal-holds",
    // the breach-notification workflow.
    "/breach",
    "/breach/{id}/event",
    "/breach/{id}/close",
    "/breaches",
    "/breaches/{id}",
    // the transfer register + TIA/DPA artifacts.
    "/transfers",
    "/transfers/{id}/tia",
    "/transfers/{id}/dpa",
    // the BPO operating register.
    "/clients",
    "/clients/{name}",
    "/clients/{name}/dpa",
    "/clients/{name}/dsar",
    "/clients/{name}/hold",
    "/clients/{name}/end",
    // the supervisor QA surface.
    "/clients/{name}/proposals",
    "/clients/{name}/proposals/{id}/coach",
    "/retention/report",
    // the curated legal-rules DB (Admin + DPO role; read-only diff).
    "/legal/rules",
    "/sources/reconcile",
    "/sources/{id}",
    "/connectors",
    // profile-gated registration (Admin).
    "/connectors/register",
    "/verify",
    "/suggest",
    "/suggest/feedback",
    "/suggest/metrics",
    "/procedure",
    "/procedure/{id}/steps",
    "/classify",
    "/decision/{id}/evaluate",
    "/webhooks/{kind}",
    // Switchboard (HMAC self-authenticating like /webhooks/*)
    "/webhooks/channel/{kind}",
    "/webhooks/channel/{kind}/drain",
    "/webhooks/channel/{kind}/drain/ack",
    // Herald (the bridge-relayed operator console; same seam)
    "/webhooks/channel/{kind}/console",
    // the delivery authority-observation sub-family. Public by the SAME
    // `/webhooks/` prefix rule as its siblings — it adds no new public path,
    // and it authenticates with the same shipped GitHub HMAC verifier.
    "/webhooks/delivery/{kind}",
    "/audit",
    "/audit/verify",
    "/metrics",
    "/quarantine",
    "/quarantine/{id}/release",
    "/quarantine/{id}/delete",
    "/consolidate/propose",
    "/consolidate/apply",
    "/consolidate/undo",
    "/ingest/proposal",
    "/proposals",
    "/proposals/{id}/approve",
    "/proposals/{id}/reject",
    "/proposals/{id}/edit",
    "/decayed",
    "/export",
    "/purge",
    "/recall/{trace_id}/trace",
    "/dsar",
    "/tombstones",
    "/dsar/{id}/certificate",
    "/auth/refresh",
    "/auth/logout",
    "/auth/revoke",
    "/.well-known/openid-configuration",
    "/.well-known/jwks.json",
    "/.well-known/security.txt",
    "/.well-known/ai-notice",
    "/.well-known/ai-literacy",
    "/.well-known/cop-notice",
    "/retention",
    "/art30",
    "/snapshot/status",
    "/ump/capabilities",
    "/ump/remember",
    "/ump/memory/{id}",
    "/ump/recall",
    "/ump/revise",
    "/ump/forget",
    "/ump/feedback",
    "/ump/subscribe",
    "/ump/audit",
    "/ump/audit/verify",
    "/.well-known/ump.json",
    "/events",
    // The engine-facing workflow surfaces (substrate projections).
    "/workflow/runs",
    "/workflow/runs/{id}",
    "/workflow/runs/{id}/state",
    "/workflow/runs/{id}/events",
    "/workflow/runs/{id}/rewind",
    "/workflow/runs/{id}/handoff",
    "/workflow/runs/{id}/context",
    "/workflow/runs/{id}/answer",
    "/workflow/runs/{id}/steering",
    "/workflow/runs/{id}/steps",
    // the recorded-rows report at a pinned law version (Read).
    "/workflow/runs/{id}/report",
    "/workflow/runs/{id}/suggestions",
    // The personal assistant's cranks + views.
    "/workflow/valet/due",
    "/workflow/valet/brief",
    "/workflow/valet/consent",
    // The KCS article lifecycle (Evolve).
    "/kcs/articles",
    "/kcs/articles/{id}/approve",
    "/kcs/articles/{id}/publish",
    "/kcs/articles/{id}/preview",
    // the RBAC introspection surface. Admin-on-global, reason-only, and
    // scoped to the caller's own principal.
    "/ops/authz/explain",
    "/kcs/translate",
    "/ops/shifts",
    "/ops/crew",
    "/ops/skills",
    "/ops/crew/config",
    // Workload + competence visibility (the Handshake milestone).
    "/ops/workload",
    "/ops/coverage",
    "/workflow/runs/{id}/handover/offer",
    "/workflow/runs/{id}/handover/{offer_id}/accept",
    "/workflow/runs/{id}/handover/{offer_id}/decline",
    "/workflow/runs/{id}/handoff/decision",
    "/workflow/runs/{id}/back-referral/return",
    "/ops/handovers",
    "/workflow/runs/{id}/notes",
    "/workflow/runs/{id}/notes/{invite_id}/accept",
    // the only writer of the table).
    "/workflow/channel/user-map",
    // Mesh: agents as named colleagues — signed cards + delegation.
    "/ops/agents/cards",
    // The ASI03/07 principal kill-switch.
    "/ops/agents/revoke",
    "/ops/agents/revocations",
    // Live agent bill of materials (AgBOM): read-only projection.
    "/ops/agents/bom",
    // The scoreboard + its sign-off, and the plugin mount seam — always-on
    // registrations that were missing from this table (table debt; the
    // handler gates were verified correct at the same audit that found the
    // gap).
    "/workflow/scoreboard",
    "/workflow/reflection/corpus",
    "/workflow/calibration/sign",
    "/workflow/plugins/mount",
    "/workflow/runs/{id}/delegations",
    "/workflow/runs/{id}/delegations/{delegation_id}/result",
    "/workflow/runs/{id}/complaint/lifecycle",
    "/workflow/runs/{id}/complaint/remedy",
    "/workflow/runs/{id}/complaint/adr-packet",
    "/workflow/runs/{id}/complaint/ack",
    "/workflow/complaints/ack-sweep",
    "/workflow/outreach/campaign",
    "/workflow/outreach/campaign/{id}",
    "/workflow/outreach/consent",
    "/workflow/runs/{id}/outreach/followup",
    "/workflow/runs/{id}/status-ref",
    // The operator case-launch boundary: agents are refused at the
    // handler (agents do not self-launch cases).
    "/workflow/cases/{id}/gdl",
    "/parcels",
    "/parcels/export",
    "/parcels/import",
    // The StewardOS account surfaces (deliberately-not-a-CRM).
    "/accounts",
    "/accounts/{id}",
    "/accounts/{id}/pipeline",
    "/accounts/{id}/requests",
    "/accounts/{id}/requests/{run_id}/link",
    // The κ labeling bench (the operator labeling round's instrument).
    "/workflow/kappa/queue",
    "/workflow/kappa/labels",
    "/workflow/kappa/report",
    // The agreement-labelling path: a verdict bound to a REAL run row. Same
    // shape as the bench — queue + capture are `calibrate`-gated Writes, the
    // report is the DPO dual gate.
    "/workflow/agreement/queue",
    "/workflow/agreement/labels",
    "/workflow/agreement/report",
    // The wizard pack catalog (the shell renderer's pack read).
    "/workflow/wizard/packs",
    // The decision-run surfaces: execute (POST), the stored trace by row
    // id (GET), and the replay-diff (POST). POST + GET share the base
    // path; the tables are path-keyed, the stricter (DPO-gated) listing
    // registers last (the /accounts convention).
    "/workflow/decision-runs",
    "/workflow/decision-runs/{id}",
    "/workflow/decision-runs/{id}/replay-diff",
    // The model registry's three distinct paths; the listing is last so the
    // stricter DPO-gated handler is the one the source scan resolves.
    "/workflow/model-registry/register",
    "/workflow/model-registry/{model_ref}",
    "/workflow/model-registry",
    // The decision-evaluation control plane. The base path carries POST and
    // the DPO-gated listing GET; the detail path is the id-scoped read.
    "/workflow/decision-evals",
    "/workflow/decision-evals/{id}",
    // The delivery loop's run writes, plus the attestation round's first READ. The read exists
    // because the attestation chain is evidence a reviewer must be able to
    // fetch; it re-derives from stored bytes and takes no body.
    "/workflow/delivery/runs",
    "/workflow/delivery/runs/{id}/advance",
    "/workflow/delivery/runs/{id}/answer",
    "/workflow/delivery/runs/{id}/gates",
    "/workflow/delivery/runs/{id}/attestations",
    // the replay round: two more reads over the same stored bytes. Both take
    // no body — the verdict re-derives from storage and the listing serves it.
    "/workflow/delivery/runs/{id}/replay-verify",
    "/workflow/delivery/runs/{id}/trace",
    // the bindings read. The domain is a query parameter, so the row this
    // serves is domain-scoped rather than run-scoped.
    "/workflow/delivery/bindings",
    // the release family: the governed write whose consequences reach another
    // system. One path per move — file, approve, promote — so the audit trail
    // names the act, and no route both approves and promotes in one request
    // (the atomic approve-and-promote is exactly what makes replay possible).
    "/workflow/delivery/releases",
    "/workflow/delivery/releases/{id}/approve",
    "/workflow/delivery/releases/{id}/promote",
    // the /due crank. Write, like the valet crank it transplants: it moves
    // durable rows and drives egress, and it is scoped to the BODY's domain
    // (it has no run id to be probe-blind with).
    "/workflow/delivery/due",
    // the run read census. The two shared-path reads (GET /releases, GET
    // /runs) are Read rows beside their Write rows: the scan maps actions to
    // handlers per method, and least privilege keeps every read Read.
    "/workflow/delivery/runs/{id}",
    "/workflow/delivery/runs/{id}/steps",
    // the derived read model (the operate round). The domain is a required
    // query parameter — the bindings shape: a read over the domain's own
    // audited release rows, domain-scoped rather than run-scoped, and the
    // window is bounded and validated in the core.
    "/workflow/delivery/outcomes",
    // The create loop's five distinct paths across six surfaces. The base path
    // carries the proposal POST and the gated listing GET; the id-scoped path
    // is the promotion-screen read; the two sub-paths are the gate and the
    // promotion act. The schema route sits on its own path because authoring a
    // slot schema is a human act with a different role gate from proposing a
    // claim.
    "/workflow/claim-schemas",
    "/workflow/claims",
    "/workflow/claims/{id}",
    "/workflow/claims/{id}/verify",
    "/workflow/claims/{id}/promote",
];

/// Every non-public route and the `Action::X` its handler must carry.
/// Consumed by `authz_gates_cover_every_non_public_route` (the authz
/// source-scan pin).
pub const AUTHZ_GATES: &[(&str, &str)] = &[
    ("/add", "Write"),
    ("/ingest/memory", "Write"),
    ("/search", "Read"),
    ("/v1/embeddings", "Write"),
    ("/ingest/markdown", "Write"),
    ("/reindex", "Admin"),
    ("/get/{id}", "Read"),
    ("/multi-get", "Read"),
    ("/graph/entity/{name}", "Read"),
    ("/graph/relations", "Read"),
    ("/graph/traverse", "Read"),
    ("/graph/relationships/{id}/history", "Admin"),
    ("/recall", "Read"),
    ("/ingest", "Write"),
    ("/memory/{id}", "Admin"),
    ("/domains", "Read"),
    ("/domains/{name}", "Admin"),
    ("/domains/{name}/vacuum", "Admin"),
    // shim mode resolves any name to the ONE shared pool — the
    // exported bytes are the whole multi-tenant DB, so the gate is
    // Admin there (Read only in multi-db, where the file IS the
    // domain). [errata-exempt: audit-id predates the hygiene pin; the table row cites it]
    ("/domains/{name}/export", "Admin"),
    ("/domains/{name}/import", "Admin"),
    ("/domains/move", "Admin"),
    ("/domains/recompute", "Admin"),
    // reads are Read; upsert + bind are Admin (the
    // POST on /profiles/{name} shares its path with a Read GET, so
    // Admin is the conservative check — the /retention precedent).
    ("/profiles", "Read"),
    ("/profiles/{name}", "Admin"),
    ("/domains/{name}/profile", "Admin"),
    // reads are Read; upsert is Admin (the POST on
    // /roles/{name} shares its path with a Read GET, so Admin is the
    // conservative check — the /profiles precedent).
    ("/roles", "Read"),
    ("/roles/{name}", "Admin"),
    // legal hold + the retention schedule are
    // operator surfaces (Admin).
    ("/legal-hold", "Admin"),
    ("/legal-hold/{id}/release", "Admin"),
    ("/legal-holds", "Admin"),
    // breach workflow is a DPO surface.
    ("/breach", "Admin"),
    ("/breach/{id}/event", "Admin"),
    ("/breach/{id}/close", "Admin"),
    ("/breaches", "Admin"),
    ("/breaches/{id}", "Admin"),
    // the transfer register + TIA/DPA artifacts
    // are operator evidence surfaces (Admin).
    ("/transfers", "Admin"),
    ("/transfers/{id}/tia", "Admin"),
    ("/transfers/{id}/dpa", "Admin"),
    // the BPO operating register (Admin, audited).
    // /clients + /clients/{name} stay Admin at the path
    // gate; a client-auditor principal gets a row-level domain filter
    // (the handler still enforces authorize — defense-in-depth).
    ("/clients", "Admin"),
    ("/clients/{name}", "Admin"),
    ("/clients/{name}/dpa", "Admin"),
    ("/clients/{name}/dsar", "Admin"),
    ("/clients/{name}/hold", "Admin"),
    ("/clients/{name}/end", "Admin"),
    ("/clients/{name}/proposals", "Admin"),
    ("/clients/{name}/proposals/{id}/coach", "Admin"),
    ("/retention/report", "Admin"),
    // the curated legal-rules DB: Admin gate; the handler then demands the
    // DPO role (the scoreboard posture — DPO evidence, not a public read).
    ("/legal/rules", "Admin"),
    ("/sources/reconcile", "Write"),
    ("/sources/{id}", "Write"),
    ("/connectors", "Read"),
    ("/connectors/register", "Admin"),
    ("/verify", "Read"),
    ("/suggest", "Read"),
    ("/suggest/feedback", "Write"),
    ("/suggest/metrics", "Read"),
    ("/procedure", "Write"),
    ("/procedure/{id}/steps", "Read"),
    ("/classify", "Read"),
    ("/decision/{id}/evaluate", "Read"),
    ("/consolidate/propose", "Read"),
    ("/consolidate/apply", "Write"),
    ("/consolidate/undo", "Write"),
    ("/audit", "Admin"),
    ("/audit/verify", "Admin"),
    ("/metrics", "Read"),
    ("/quarantine", "Read"),
    ("/quarantine/{id}/release", "Admin"),
    ("/quarantine/{id}/delete", "Admin"),
    ("/auth/revoke", "Admin"),
    ("/ingest/proposal", "Write"),
    ("/proposals", "Read"),
    ("/proposals/{id}/approve", "Write"),
    ("/proposals/{id}/reject", "Write"),
    ("/proposals/{id}/edit", "Write"),
    ("/decayed", "Read"),
    ("/export", "Read"),
    ("/purge", "Admin"),
    // trace replay + DSAR are operator surfaces.
    ("/recall/{trace_id}/trace", "Admin"),
    ("/dsar", "Admin"),
    ("/tombstones", "Admin"),
    ("/dsar/{id}/certificate", "Admin"),
    // retention policy set + compliance/snapshot reads
    // are operator surfaces (Admin). GET /retention is Read, but the
    // route shares a path with POST (Admin); the scan maps to the last
    // registered handler (POST), so Admin is the conservative check.
    ("/retention", "Admin"),
    ("/art30", "Admin"),
    ("/snapshot/status", "Admin"),
    // §3.3 matrix — Writes for remember/revise/forget/
    // feedback, Read for recall/get/subscribe, Admin for audit.
    ("/ump/remember", "Write"),
    ("/ump/memory/{id}", "Read"),
    ("/ump/recall", "Read"),
    ("/ump/revise", "Write"),
    ("/ump/forget", "Write"),
    ("/ump/feedback", "Write"),
    ("/ump/subscribe", "Read"),
    ("/ump/audit", "Admin"),
    ("/ump/audit/verify", "Admin"),
    ("/events", "Read"),
    // the workflow scoreboard is a DPO/admin evidence surface.
    ("/workflow/scoreboard", "Admin"),
    // the corpus export: Admin-gated here; the handler additionally
    // demands the DPO role (the scoreboard posture)
    ("/workflow/reflection/corpus", "Admin"),
    // the monthly human-signed calibration gate: Admin + DPO role.
    ("/workflow/calibration/sign", "Admin"),
    // the governed-workflow run surfaces: reads on the run's domain,
    // steering is a Write + approve-class role gate.
    ("/workflow/runs/{id}", "Read"),
    ("/workflow/runs/{id}/steps", "Read"),
    ("/workflow/runs/{id}/steering", "Write"),
    ("/workflow/runs/{id}/suggestions", "Read"),
    // the recorded-rows report: a Read on the run's domain (the report is
    // advisory-only on the legal DB — never a refusal, never a block).
    ("/workflow/runs/{id}/report", "Read"),
    // workflow-role Writes on global; the brief is a Read.
    ("/workflow/valet/due", "Write"),
    ("/workflow/valet/brief", "Read"),
    ("/workflow/valet/consent", "Write"),
    // Engine surfaces: open/state/events carry the `workflow` role
    // gate, answer the `approve` (HITL) gate; steering drain is a
    // Read on the run's domain.
    ("/workflow/runs", "Write"),
    // GET and PUT share this path; the scan maps to the LAST
    // registered handler (PUT), so Write is the checked gate (same
    // conservative convention as `/retention`).
    ("/workflow/runs/{id}/state", "Write"),
    ("/workflow/runs/{id}/events", "Write"),
    // Lineage: the events read + handoff packet are Reads
    // on the run's domain; rewind is a Write + `approve` role gate.
    ("/workflow/runs/{id}/rewind", "Write"),
    // remedy proposals are Writes + `workflow` role; the ADR packet
    // is a Read on the run's domain.
    ("/workflow/runs/{id}/complaint/lifecycle", "Write"),
    ("/workflow/runs/{id}/complaint/remedy", "Write"),
    ("/workflow/runs/{id}/complaint/adr-packet", "Read"),
    ("/workflow/runs/{id}/complaint/ack", "Write"),
    ("/workflow/complaints/ack-sweep", "Write"),
    // read are global-scope (no run binds them); follow-up rides
    // the run's domain.
    ("/workflow/outreach/campaign", "Write"),
    ("/workflow/outreach/campaign/{id}", "Read"),
    ("/workflow/outreach/consent", "Read"),
    ("/workflow/runs/{id}/outreach/followup", "Write"),
    // Keystone: status-ref actions are approve-role writes on the
    // run's domain.
    ("/workflow/runs/{id}/status-ref", "Write"),
    // The case-launch boundary is a workflow Write on the run's
    // domain; the operator/agent split is enforced at the handler.
    ("/workflow/cases/{id}/gdl", "Write"),
    ("/workflow/runs/{id}/handoff", "Read"),
    // The derived context window — a Read on the run's
    // domain (pure derivation over the lineage the events read serves).
    ("/workflow/runs/{id}/context", "Read"),
    ("/workflow/runs/{id}/answer", "Write"),
    // plugin mount evidence: any authenticated principal records its
    // own composition (a Write, metadata-only).
    ("/workflow/plugins/mount", "Write"),
    // The KCS article lifecycle: the worklist is a Read; approve is
    // the HITL Write + `approve` role gate.
    ("/kcs/articles", "Read"),
    ("/kcs/articles/{id}/approve", "Write"),
    // Keystone: filing a translation proposal is a workflow write.
    ("/kcs/translate", "Write"),
    // Beacon: publish PROPOSAL creation is a Write (the capability
    // gate lives at approval time); the preview is a Read over the
    // sanitized public render path.
    ("/kcs/articles/{id}/publish", "Write"),
    ("/kcs/articles/{id}/preview", "Read"),
    // the RBAC introspection route. Admin-on-global — it describes the
    // deployment's gate table, exactly the posture `health_db_admin_full_read_
    // reduced` uses for `/health/db`.
    ("/ops/authz/explain", "Admin"),
    // Watchbill: the ring view is a Read; declaring a shift is pure
    // operator configuration → Admin (an agent-class principal must
    // not re-anchor the follow-the-sun queue). GET and POST share the
    // path; the scan maps to the last registered handler (POST), so
    // Admin is the checked gate.
    ("/ops/shifts", "Admin"),
    // Crew: the roster is a Read over people-visibility (hidden when
    // the DPO switch is off); proposing a skills change is a Write —
    // only approval writes tags; toggling presence visibility is
    // governance → Admin. GET /ops/skills (the WFM feed) shares the
    // skills path; the scan maps to the LAST registered handler
    // (POST), so Write is the checked gate.
    ("/ops/crew", "Read"),
    ("/ops/skills", "Write"),
    ("/ops/crew/config", "Admin"),
    // Workload + competence visibility: pure
    // lineage reads over people-shaped aggregates (no case content) —
    // Read on the domain, same posture as the roster.
    ("/ops/workload", "Read"),
    ("/ops/coverage", "Read"),
    // Relay: the offer/accept/decline are Writes on the run's domain
    // (accept performs the owner CAS); the handover-due board is a
    // Read over the ring.
    ("/workflow/runs/{id}/handover/offer", "Write"),
    ("/workflow/runs/{id}/handover/{offer_id}/accept", "Write"),
    ("/workflow/runs/{id}/handover/{offer_id}/decline", "Write"),
    // The operator decision surfaces: delivering/cancelling a handoff and
    // releasing a back-referral are operator Writes on the run's domain
    // (both handlers additionally demand the `workflow` role — the HITL
    // law's gate shape).
    ("/workflow/runs/{id}/handoff/decision", "Write"),
    ("/workflow/runs/{id}/back-referral/return", "Write"),
    ("/ops/handovers", "Read"),
    // Channel: posting a note (and its mention-resolved invites) is a
    // Write; the channel view is a Read over the same run. GET and
    // POST share the path; the scan maps to the last registered
    // handler (GET), so Read is the checked gate — the POST side is
    // pinned by its handler source below.
    ("/workflow/runs/{id}/notes", "Read"),
    // Accepting an invite joins the room: a Write, ownership never
    // moves.
    ("/workflow/runs/{id}/notes/{invite_id}/accept", "Write"),
    // Filing a user-map proposal is a governance Write; the table's
    // ONLY writer is the approval path, never this route.
    ("/workflow/channel/user-map", "Write"),
    // Mesh: provisioning/re-signing a card is governance over the
    // agent's identity → Admin; the verified card views are Reads.
    ("/ops/agents/cards", "Read"),
    // Attestation: the kill-switch revokes an IDENTITY (not a
    // domain row) → Admin on global; the register view is a Read.
    ("/ops/agents/revoke", "Admin"),
    ("/ops/agents/revocations", "Read"),
    // AgBOM: live inventory projection — a Read on global.
    ("/ops/agents/bom", "Read"),
    // Delegation: requesting work from a named agent and returning its
    // result are Writes on the run's domain; the delegation view is a
    // Read (GET/POST share the path — Read is the checked gate, the
    // POST side pinned by handler source below).
    ("/workflow/runs/{id}/delegations", "Read"),
    (
        "/workflow/runs/{id}/delegations/{delegation_id}/result",
        "Write",
    ),
    // Parcels: exporting signed knowledge off-site is governance →
    // Admin; importing lands rows as pending proposals (a Write —
    // nothing reaches knowledge without human approval); the ledger
    // view is a Read.
    ("/parcels", "Read"),
    ("/parcels/export", "Admin"),
    ("/parcels/import", "Write"),
    // ── rows the reverse-direction guard found missing (the scan walked the
    // composed router and demanded every non-public route appear here) ──
    // `/stats` is legacy-shaped (200-shell denial) but its handler carries a
    // real `Action::Read` gate; the scoreboard + calibration sign are Admin
    // (the handler then demands a DPO role); the mount seam is a Write whose
    // bridge-identity HMAC check precedes the gate.
    ("/.well-known/security.txt", "public"),
    ("/stats", "Read"),
    ("/workflow/scoreboard", "Admin"),
    ("/workflow/calibration/sign", "Admin"),
    ("/workflow/plugins/mount", "Write"),
    // The StewardOS account surfaces: writes and per-account reads are
    // workflow-role Writes on the account's domain; GET and POST share
    // /accounts and the scan maps the path to the LAST registered handler
    // (the DPO dual gate — the stricter check, the /retention convention);
    // the POST side is a Write + `workflow` role route pinned by its
    // handler source below.
    ("/accounts", "Admin"),
    ("/accounts/{id}", "Write"),
    ("/accounts/{id}/pipeline", "Write"),
    ("/accounts/{id}/requests", "Write"),
    ("/accounts/{id}/requests/{run_id}/link", "Write"),
    // The κ labeling bench: queue + label capture are `calibrate`-gated
    // Writes on the global domain (the role gate is pinned by the handler
    // source below); the report is the DPO dual gate — Admin here, the
    // DPO role + `calibrate` capability demanded by the handler.
    ("/workflow/kappa/queue", "Write"),
    ("/workflow/kappa/labels", "Write"),
    ("/workflow/kappa/report", "Admin"),
    // The agreement-labelling path: queue + verdict capture are
    // `calibrate`-gated Writes on the global domain (the role gate is pinned
    // by the handler source below); the report is the DPO dual gate — Admin
    // here, the DPO role + `calibrate` capability demanded by the handler.
    ("/workflow/agreement/queue", "Write"),
    ("/workflow/agreement/labels", "Write"),
    ("/workflow/agreement/report", "Admin"),
    // The wizard pack catalog: a plain Read on the global domain — the
    // ratified posture (PII-free schema templates; no role gate, no
    // PRE_GATE/empty-safe joins).
    ("/workflow/wizard/packs", "Read"),
    // The decision-run surfaces: the execute POST and the replay POST are
    // Writes on the run's domain (the role gate is pinned by the handler
    // source); the by-id trace read is a Read. POST and GET share the
    // base path and the scan maps the path to the LAST registered handler
    // (the DPO dual gate — the stricter check, the /accounts convention).
    ("/workflow/decision-runs", "Admin"),
    ("/workflow/decision-runs/{id}", "Read"),
    ("/workflow/decision-runs/{id}/replay-diff", "Write"),
    // Registration is an operator action; the single-row read is Read; the
    // listing carries the additional DPO role gate in its handler.
    ("/workflow/model-registry/register", "Admin"),
    ("/workflow/model-registry/{model_ref}", "Read"),
    ("/workflow/model-registry", "Admin"),
    // Evaluation creation and listing are Admin-on-global operator/DPO
    // actions; the detail uses the same conservative confidential posture.
    ("/workflow/decision-evals", "Admin"),
    ("/workflow/decision-evals/{id}", "Admin"),
    // The delivery loop's run writes: Write on the run's own domain. The
    // `workflow`-role gate lives in the handler (the role store reads from the
    // pool, so it cannot be a table row). Stated precisely, because an
    // earlier wording claimed a refusal that existed nowhere: the agent class
    // is NOT refused on these run writes — `authorize_role` passes principals
    // that hold no roles at all, so an agent preset holding `write:*` is
    // admitted here. The promotion family below is different: its writes
    // leave the host, and the agent class is refused in handlers/delivery.rs
    // — explicitly, before any work.
    // The two shared paths carry BOTH actions. The Read row comes FIRST on
    // purpose: the authz matrix pairs a path's first table row with its
    // last-registered method (the GET here), so the row it reads must be the
    // read's. A read that demanded Write would be a privilege nobody asked
    // for; the write rows below still pin the writes.
    ("/workflow/delivery/runs", "Read"),
    ("/workflow/delivery/releases", "Read"),
    ("/workflow/delivery/runs", "Write"),
    ("/workflow/delivery/runs/{id}/advance", "Write"),
    ("/workflow/delivery/runs/{id}/answer", "Write"),
    ("/workflow/delivery/runs/{id}/gates", "Write"),
    // the attestation round's read: Read, on the run's OWN domain, same role gate in-handler.
    ("/workflow/delivery/runs/{id}/attestations", "Read"),
    // the replay round's two reads: Read, on the run's OWN domain, same role
    // gate in-handler. Least privilege — neither surface mutates, so neither
    // demands Write. A read demanding Write would be a privilege no surface
    // asked for.
    ("/workflow/delivery/runs/{id}/replay-verify", "Read"),
    ("/workflow/delivery/runs/{id}/trace", "Read"),
    // the bindings read: Read, on the QUERIED domain, with the workflow role
    // checked in-handler (the role store reads from the pool). Least
    // privilege — the surface cannot mutate an authority, so it does not
    // demand Write.
    ("/workflow/delivery/bindings", "Read"),
    // the release family: Write on the run's own domain, the workflow role
    // checked in-handler, AND the agent-class refused in-handler (the one
    // write family whose consequences reach another system). File, approve,
    // and promote are separate requests by design — splitting approve from
    // promote is what makes the three-way binding (content, authority,
    // revision) re-verifiable at the moment of the act.
    ("/workflow/delivery/releases", "Write"),
    ("/workflow/delivery/releases/{id}/approve", "Write"),
    ("/workflow/delivery/releases/{id}/promote", "Write"),
    ("/workflow/delivery/due", "Write"),
    ("/workflow/delivery/runs/{id}", "Read"),
    ("/workflow/delivery/runs/{id}/steps", "Read"),
    // the derived read model: Read, on the QUERIED domain, with the workflow
    // role checked in-handler (the bindings shape — domain-scoped, not
    // run-scoped, so the agent cell really reaches it, and the matrix's
    // ROLE_GATED_FOR_AGENT list says so).
    ("/workflow/delivery/outcomes", "Read"),
    // The create loop. The base path carries BOTH actions and the Read row
    // comes FIRST on purpose: the authz matrix pairs a path's first table row
    // with its last-registered method (the GET here), so the row it reads must
    // be the read's. A gated read that demanded Write would be a privilege
    // nobody asked for.
    //
    // The `workflow`-role gate lives in the handlers, because the role store
    // reads from the pool and cannot be a table row — the delivery-runs
    // convention, stated precisely rather than by appeal to a refusal that does
    // not exist: an agent preset holding `write:*` is ADMITTED at the table
    // layer here. The binding human-artifact check is the promote handler's.
    ("/workflow/claims", "Read"),
    ("/workflow/claims/{id}", "Read"),
    ("/workflow/claims/{id}/verify", "Write"),
    ("/workflow/claims/{id}/promote", "Write"),
    ("/workflow/claim-schemas", "Write"),
    ("/workflow/claims", "Write"),
];
