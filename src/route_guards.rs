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
//! Test-only data: the module is compiled nowhere outside test builds.

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
    // v1.23.0 Roles
    "/roles",
    "/roles/{name}",
    // v1.22.0 Regulated
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
    "/sources/reconcile",
    "/sources/{id}",
    // v0.9.6 Bridge
    "/connectors",
    // profile-gated registration (Admin).
    "/connectors/register",
    // v1.5.0 Epistemic
    "/verify",
    // v1.9.0 Suggest
    "/suggest",
    "/suggest/feedback",
    "/suggest/metrics",
    // v1.10.0 Procedural
    "/procedure",
    "/procedure/{id}/steps",
    "/classify",
    "/decision/{id}/evaluate",
    // v0.9.7 Guard
    "/webhooks/{kind}",
    // Switchboard (HMAC self-authenticating like /webhooks/*)
    "/webhooks/channel/{kind}",
    "/webhooks/channel/{kind}/drain",
    // Herald (the bridge-relayed operator console; same seam)
    "/webhooks/channel/{kind}/console",
    "/audit",
    "/audit/verify",
    "/metrics",
    "/quarantine",
    "/quarantine/{id}/release",
    "/quarantine/{id}/delete",
    // v0.9.8 Evidence
    "/consolidate/propose",
    "/consolidate/apply",
    "/consolidate/undo",
    // v1.14.0 Gate
    "/ingest/proposal",
    "/proposals",
    "/proposals/{id}/approve",
    "/proposals/{id}/reject",
    "/proposals/{id}/edit",
    "/decayed",
    "/export",
    "/purge",
    // v1.15.0 Observe
    "/recall/{trace_id}/trace",
    "/dsar",
    "/tombstones",
    "/dsar/{id}/certificate",
    // v1.2.0 AuthN
    "/auth/refresh",
    "/auth/logout",
    "/auth/revoke",
    "/.well-known/openid-configuration",
    "/.well-known/jwks.json",
    "/.well-known/ai-notice",
    "/.well-known/ai-literacy",
    "/.well-known/cop-notice",
    // v1.17.1 Govern
    "/retention",
    "/art30",
    "/snapshot/status",
    // v1.17.3 UMP
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
    // v1.28.36 "Keystone": governed human translation filing.
    "/kcs/translate",
    // v1.28.25 "Watchbill": the shift ring (follow-the-sun data).
    "/ops/shifts",
    // v1.28.26 "Crew": the roster, the skills proposal, and the DPO switch.
    "/ops/crew",
    "/ops/skills",
    "/ops/crew/config",
    // Workload + competence visibility (the Handshake milestone).
    "/ops/workload",
    "/ops/coverage",
    // v1.28.27 "Relay": the one-click handover.
    "/workflow/runs/{id}/handover/offer",
    "/workflow/runs/{id}/handover/{offer_id}/accept",
    "/workflow/runs/{id}/handover/{offer_id}/decline",
    "/ops/handovers",
    // v1.28.28 "Channel": the case gets a room.
    "/workflow/runs/{id}/notes",
    "/workflow/runs/{id}/notes/{invite_id}/accept",
    // v1.28.45 "Herald": the user-map proposal filing (approval is
    // the only writer of the table).
    "/workflow/channel/user-map",
    // Mesh: agents as named colleagues — signed cards + delegation.
    "/ops/agents/cards",
    "/workflow/runs/{id}/delegations",
    "/workflow/runs/{id}/delegations/{delegation_id}/result",
    // v1.28.34 "Goodwill": the complaint lifecycle surface.
    "/workflow/runs/{id}/complaint/lifecycle",
    "/workflow/runs/{id}/complaint/remedy",
    "/workflow/runs/{id}/complaint/adr-packet",
    "/workflow/runs/{id}/complaint/ack",
    "/workflow/complaints/ack-sweep",
    // v1.28.35 "Outreach": consent-first outbound contact.
    "/workflow/outreach/campaign",
    "/workflow/outreach/campaign/{id}",
    "/workflow/outreach/consent",
    "/workflow/runs/{id}/outreach/followup",
    // v1.28.36 "Keystone": public case-status refs.
    "/workflow/runs/{id}/status-ref",
    // v1.28.30 "Parcels": signed site-to-site knowledge crossings.
    "/parcels",
    "/parcels/export",
    "/parcels/import",
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
    // domain; S2-08).
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
    // the monthly human-signed calibration gate: Admin + DPO role.
    ("/workflow/calibration/sign", "Admin"),
    // the governed-workflow run surfaces: reads on the run's domain,
    // steering is a Write + approve-class role gate.
    ("/workflow/runs/{id}", "Read"),
    ("/workflow/runs/{id}/steps", "Read"),
    ("/workflow/runs/{id}/steering", "Write"),
    ("/workflow/runs/{id}/suggestions", "Read"),
    // v1.28.42 "Valet": the due crank + consent registry are
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
    // v1.28.34 "Goodwill": the complaint lifecycle — transitions and
    // remedy proposals are Writes + `workflow` role; the ADR packet
    // is a Read on the run's domain.
    ("/workflow/runs/{id}/complaint/lifecycle", "Write"),
    ("/workflow/runs/{id}/complaint/remedy", "Write"),
    ("/workflow/runs/{id}/complaint/adr-packet", "Read"),
    ("/workflow/runs/{id}/complaint/ack", "Write"),
    ("/workflow/complaints/ack-sweep", "Write"),
    // v1.28.35 "Outreach": campaign propose/export + the consent
    // read are global-scope (no run binds them); follow-up rides
    // the run's domain.
    ("/workflow/outreach/campaign", "Write"),
    ("/workflow/outreach/campaign/{id}", "Read"),
    ("/workflow/outreach/consent", "Read"),
    ("/workflow/runs/{id}/outreach/followup", "Write"),
    // Keystone: status-ref actions are approve-role writes on the
    // run's domain.
    ("/workflow/runs/{id}/status-ref", "Write"),
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
];
