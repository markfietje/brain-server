# v1.16.8 M1 — English strings (human-authored first cut).
# Simple FTL subset: `key = value`, `#` comment lines, blank lines skipped.
# A key missing in another locale falls back to this file via i18n.rs `t()`.

## Panel titles
review_title = Review queue
recall_title = Recall inspector
subjects_title = Subjects (DSAR)
security_title = Security
# v1.27.19 "Scrub" (D-7): quarantine action outcomes.
sec_released = released from quarantine
sec_deleted = deleted
audit_title = Audit
health_title = Health

## Navigation
nav_review = Review
nav_recall = Recall
nav_subjects = Subjects
nav_security = Security
nav_audit = Audit
nav_health = Health

## Shell / top bar
pending = pending
flags = flags
audit_chain = audit chain!
acting_as = acting as
loopback = loopback
sign_out = Sign out
connected = connected
reconnecting = reconnecting
disconnected = Disconnected — showing last-known state. Write actions disabled.
reverifying = Reconnected — verifying audit chain before enabling writes…
detail = Detail
close_drawer = close drawer
nothing_selected = nothing selected

## Connect
connect_title = Connect to brain-server
connect_welcome = Governed memory, on your hardware.
backend_url = Backend URL
token_label = Token
url_placeholder = blank = this page's origin (same-server)
token_placeholder = optional (loopback)
token_access_placeholder = access token (JWT)
jwt_pair = JWT pair (access + refresh) — enables silent refresh
refresh_token_label = Refresh token
refresh_token_placeholder = from `brain key mint` or an IdP
connecting = Connecting…
connect_button = Connect
plaintext_http = Plain http:// over a non-loopback address — the auth token would travel unencrypted. Use https:// or a loopback host.
install_hint = One-line install:  curl -fsSL … | sh   then  brain doctor

## Review
no_pending = No pending proposals.
approve = Approve
reject = Reject
edit = Edit
proposal = Proposal
review_help = Review keyboard shortcuts
review_help_toggle = Toggle shortcuts help
review_key_approve = Approve the focused proposal
review_key_supersede = Approve, superseding the conflicting proposal
review_key_reject = Reject the focused proposal
review_key_edit = Rewrite the focused proposal's content
review_key_next = Next proposal
review_key_prev = Previous proposal

## Subjects
deletion_certificate = Deletion certificate {0}
chain_verified = chain verified
chain_tampered = CHAIN TAMPERED

## Subjects — DSAR footprint preview (v1.20.21 M2)
dsar_preview_title = Preview DSAR footprint
dsar_preview_sub = See exactly what a purge would delete — nothing is erased.
dsar_preview_placeholder = subject to preview…
dsar_preview_button = Preview footprint
dsar_preview_note = Preview only — nothing deleted.
dsar_preview_owners = owners
dsar_preview_derived = derived
dsar_preview_export_rows = export rows
dsar_preview_tombstones = prior tombstones
dsar_preview_ledger_rows = ledger rows

## Subjects — DSAR request ledger clock (v1.20.22 M2)
dsar_clock_title = DSAR request ledger
dsar_clock_empty = No open requests — the window is clear.
dsar_clock_completed = completed
dsar_clock_retained = history retained per ledger policy
dsar_clock_deadline = deadline

## Settings
theme_label = Theme
locale_label = Language
density_label = Density
dark = dark
light = light
comfortable = comfortable
compact = compact

## Privacy posture (M6 — connect-screen data-flow transparency).
privacy_title = Privacy
privacy_sends = brain-client sends:
privacy_sends_1 = API requests to: [your backend URL]
privacy_sends_2 = Authorization: Bearer [your token]
privacy_stores = brain-client stores:
privacy_stores_1 = Your auth token (in OS keychain/keystore — not accessible to other apps)
privacy_stores_2 = Your theme/locale preference (non-sensitive)
privacy_not = brain-client does NOT:
privacy_not_1 = Send analytics, telemetry, or crash reports
privacy_not_2 = Contact any server other than the one you configure
privacy_not_3 = Store memory content locally
privacy_not_4 = Use third-party SDKs, CDNs, or external resources

## Overview (v1.17.6 M2)
nav_overview = Overview
overview_title = Overview
overview_health = Health
overview_snapshot = Snapshot integrity
overview_retention = Retention
overview_ump = Server + UMP
overview_alerts = Alerts
no_alerts = No alerts — all quiet.
open_queue = open queue
view = view
kinds = kinds
alert_auth_failures = auth failures
alert_quarantine = quarantined chunks
alert_stale_sources = stale sources
alert_conflicts = unresolved conflicts
alert_decayed = decayed chunks
alert_near_duplicates = near-duplicate chunks
alert_tombstones = tombstones

## Command palette (v1.17.6 M1)
palette_recent = Recent
palette_go_to = Go to
palette_lookup = Lookup
palette_run = Run
confirm_destructive = Press Enter to confirm

## Graph (v1.17.7 M3)
nav_graph = Graph
graph_title = Graph

## v1.20.0 M3 (offline queue)
nav_queued = queued
nav_queued_title = Actions issued while offline; replay when the connection returns
graph_entity_ph = Search an entity…
graph_browse = Entity
graph_type = type
graph_relations = relations
graph_traverse = Traverse
graph_start = Start entity
graph_depth = Max depth
graph_kind = Edge kind (optional)
graph_at = Valid at (optional)
graph_cross_domain = Cross-domain
graph_run = Traverse
graph_rel = relation
graph_out = out
graph_in = in
graph_no_entity = No such entity
graph_paths = paths
graph_rows = rows
none = none

## Create (v1.17.7 M4)
nav_create = Create
create_title = Create
create_sub = Write tools: ingest memory, build procedures, consolidate.

## Ingest (v1.17.7 M4.1)
ingest_title = Ingest
ingest_tab_structured = Structured
ingest_tab_markdown = Markdown
ingest_tab_memory = Memory batch
ingest_content = Content
ingest_kind = Memory kind
ingest_domain = Domain
ingest_entities = Entities (JSON)
ingest_relations = Relations (JSON)
ingest_source_path = Source path
ingest_replace = Replace existing
ingest_submit = Ingest
ingest_bad_json = Entities/relations must be valid JSON
ingest_mem_hint = One memory per line, optional ## Title headers
outcome_created = Created
outcome_duplicate = Duplicate (already present)

## Procedures (v1.17.7 M4.2)
proc_title = Procedures
proc_step_title = Step title
proc_step_body = Step content
proc_add_step = Add step
proc_create = Create procedure
proc_steps = Steps
proc_is_decision = Decision rule
proc_created = Procedure created ({0} steps)
cls_title = Classify
cls_text = Text to classify
cls_run = Classify
dec_title = Evaluate decision
dec_id = Decision id
dec_vars = Variables (JSON)
dec_run = Evaluate

## Consolidate (v1.17.7 M4.3)
cons_title = Consolidate
cons_load = Load
cons_apply = Approve supersession
cons_undo = Undo
cons_empty = Nothing to consolidate.
cons_near_dup = near-dup
cons_conflict = conflict
cons_applied = Applied {0} supersessions
cons_undone = Undone {0}

## Data (v1.17.8 M5 — Rights group)
nav_data = Data
data_title = Data
data_sub = Rights and portability: purge, export, retention, registries.
data_status = Ready
data_purge = Purge
data_export = Export
data_exported = Export generated
data_retention = Retention
data_retention_state = Retention
data_retention_kind = Kind
data_retention_days = Days
data_retention_bad_days = Days must be a whole number
data_retention_set = Updated {0} override(s)
data_decayed = Decayed
data_next_expiry = Next to expire
data_tombstones = Tombstones
data_empty = Nothing to show.
data_purge_ids = Chunk ids (comma/space separated)
data_purge_owner = Or purge all for owner
data_purged = Purged {0} chunk(s)
data_purged_queued = Queued for replay (offline) — will purge when the connection returns
data_purge_empty = Provide chunk ids or an owner

## UMP (v1.17.8 M6 — Portability group)
nav_ump = UMP
ump_title = UMP
ump_sub = UMP 1.0 wire operations: capabilities, remember, recall, audit.
ump_caps = Capabilities
ump_remember = Remember
ump_recall = Recall
ump_audit = Audit
ump_bad_json = Invalid JSON
ump_remembered = Remembered
ump_chain_ok = Chain verified
ump_chain_bad = Chain tampered

## System (v1.17.8 M7 — System group)
nav_system = System
sys_title = System
sys_sub = Operator console: domains, snapshot, Art 30, sources, try-it.
sys_domains = Domains
sys_snapshot = Snapshot integrity
sys_art30 = Art 30 register
sys_reindex = Reindex
sys_reindexed = Reindexed {0} chunks
sys_sources = Sources & connectors
sys_console = Try-it console

## Operations (v1.20.6 M1–M3 — Memory Operations panel)
nav_ops = Operations
ops_title = Operations
ops_sub = The live HITL work surface: pending queue, SLA clocks, and the flagged screen output.
ops_queue = Live queue
ops_queue_summary = pending
# v1.27.19 "Scrub" (D-7): the offline-enqueue replay took over the action.
ops_queued_offline = queued (offline) — will replay on reconnect
ops_gate = Gate health
ops_flagged = Flagged & quarantined
ops_flagged_hint = Enter a probe query and Scan to surface screen-caught matches.
ops_flagged_empty = No flagged matches.
ops_decayed = Decayed (missed deadline)
ops_scan = Scan
ops_sourcing = sourcing prompt
ops_expired = expired (auto-rejected)
alert_queued = new proposal queued
alert_screen = injection flagged
alert_expiring = proposal expiring
sla_critical = critical
sla_warn = expiring soon
sla_remaining = remaining
gate_healthy = healthy
gate_over_rejecting = over-rejecting
gate_under_reviewing = under-reviewing

## Register (v1.20.9 M1–M2 — Agent Memory Register, read-only provenance ledger)
nav_register = Register

## Console (v1.27.11 — BPO dashboard views, role-gated by R9 presets)
nav_clients = Clients
console_client_title = Client dashboard
console_client_sub = Read-only overview of the clients granted to your auditor token. Domain-scoped to your JWT scopes.
console_ops_title = BPO operations board
console_ops_sub = All-clients register + connector + review workload. Read-only.
console_ops_board = Client register
console_connectors = Connectors
console_queue_depth = queue depth
console_empty = Nothing to show for this token.

## Replay (v1.20.20 M1–M3 — decision-path replay surface)
replay_title = Decision replay
replay_audit_link = open audit row
replay_export = export evidence

## Calibrate (v1.20.23 M2 — reviewer calibration strip)
cal_title = your recent gate
cal_approve_rate = approval rate
cal_latency = median decision
cal_edit_rate = edit rate
cal_override_rate = screen-override rate
cal_decisions = decisions
cal_last_200 = last 200 decisions
cal_warn_high = high approval rate — review the last few by hand


## v1.21.0 Profiles — wizard (connect flow) + Health panel
wizard_title = What best describes your team?
wizard_hint = Pick a starting posture — every knob stays editable later.
wizard_apply = Use this profile
wizard_applied = applied — defaults set, nothing re-ingested
wizard_skip = Skip for now
wizard_load_failed = could not load profiles
health_profile = Profile
health_profile_none = none — server defaults
health_profile_knobs = effective knobs

## v1.28.1 Holdall — M4 client-side destruction discipline
data_purge_preview = Preview footprint
data_purge_preview_note = Nothing deleted yet — this is the would-be scope.
data_purge_preview_stale = Input changed since this preview — run it again before purging.
data_purge_need_preview = Run the preview above first — erasure requires a rendered footprint.
data_purge_hint = Run the footprint preview to arm the purge.
purge_irreversible = This is irreversible.
reindex_irreversible = Rebuilds the vector store — this is irreversible.
quarantine_delete_irreversible = Hard-deletes the quarantined chunk — this is irreversible.
dsar_purge_confirm_title = Erase this subject? The footprint above is the confirmed scope.
dsar_purge_confirm = Erase now
dsar_purge_need_preview = Footprint not rendered for the current subject — run it first.
replay_title = Replay destructive actions
replay_sub = Queued while offline — they never auto-fire. Replay each in front of you, or skip.
replay_kind_approve = approve
replay_kind_reject = reject
replay_kind_edit = edit
replay_kind_purge = purge
replay_kind_dsar = dsar erase
replay_queued_ago = queued
replay_replay = Replay
replay_skip = Skip
replay_dismiss = Dismiss all
replay_subject_prompt = Re-enter the subject to erase (the offline form kept only its hash):
replay_subject_placeholder = subject / owner / principal…
replay_subject_required = Re-enter the subject first — the hash is one-way.

## v1.27.20 Console — deep-link state honesty (F-39) + digest display (M1.6)
back_to_queue = ← back to the review queue
detail_loading = Loading proposal…
retry = Retry
detail_not_pending = No pending proposal #{0} (already decided?)
digest_label = digest
copy_digest = copy digest

# F-58 graph traverse block hints
graph_need_start = Enter an entity name to start from
graph_need_kind = Enter a valid relation kind

## Audit surface (v1.27.20 M3 — render-surface extraction)
audit_error = audit failed: {0}
audit_empty = No events.
audit_filtered_summary = {0} events loaded · {1} after filter (hash-only — no raw content)
audit_principal_placeholder = principal…
audit_filter_principal = filter by principal
audit_filter_kind = filter by kind
audit_all_kinds = all kinds
audit_filter_since = filter since date
audit_export = Export JSON
audit_rows_exported = {0} audit rows exported
audit_load_more = Load more ({0} loaded)

## Subjects/DSAR surface (v1.27.20 M3 — render-surface extraction)
dsar_subject_placeholder = subject / owner / principal…
dsar_subject_aria = subject to action
dsar_subject_required = enter a subject first
dsar_locate_export = Locate & export
dsar_locate_export_purge = Locate, export & purge
cancel = Cancel
dsar_queued = queued — will replay when the connection returns
dsar_previewing = previewing…
dsar_purge_note = Purge is irreversible: it writes a tombstone + hash-chain entry. The deletion certificate re-verifies the chain head live.
dsar_preview_failed = preview failed: {0}
dsar_action_failed = dsar {0} failed: {1}
cert_fetch_failed = certificate fetch failed: {0}
dsar_loading = loading…
dsar_back_link = ← back to subjects
dsar_completed_retained = {0} · {1}
dsar_subject_line = subject: {0}
cert_found = found
cert_purged = purged
cert_tombstone_root = tombstone root
cert_certified = certified
cert_chain_head = chain head

## Review/ops surface (v1.27.20 M3 — render-surface extraction)
verdict_clean = clean
verdict_quarantined = quarantined
edited = edited
batch_summary = batch: {0} approved · {1} already decided · {2} queued (offline) · {3} failed
select_visible = Select visible ({0})
clear = Clear
queue_failed = queue failed: {0}
select_proposal_aria = select proposal {0}
novelty_salience = novelty {0} · salience {1}
conflict_supersede = conflicts with chunk #{0} — approve to supersede
approved_chunk = ✓ approved → chunk #{0}
already_decided = already decided
queued_offline = queued (offline)
row_failed = failed: {0}
reject_title = Reject proposal #{0}
reingest_title = Re-ingest proposal #{0} as a new proposal
edit_title = Edit proposal #{0}
post_new_proposal = Post new proposal
reason_placeholder = reason (recorded in the audit log)…
approve_before_deadline = approve before the deadline
screen_label = screen: {0}
novelty_salience_created = novelty {0} · salience {1} · created {2}
approve_selected = Approve selected ({0})
expiry_first = expiry first
creation_order = creation order
shortcut_hint = keys (A/S/R/E/J/K)
sample_proposal_cta = Ingest a sample proposal to try the gate
cal_dismiss = dismiss
cal_dismiss_aria = dismiss calibration
suggest_reingest = suggest re-ingest
reject_modal_label = reject with reason
edit_modal_label = edit proposal
ops_ar_counts = A {0} · R {1}
time_until_expiry = time until expiry
expires_in = expires in {0}
auto_reject_title = server auto-rejects expired proposals
ops_tip_approve = approve (a)
ops_tip_reject = reject (r)

## Connect/wizard/error surface (v1.27.20 M3)
connected_v = connected — v{0} · {1}{2}
connected_capacity = docs {0}/{1}, 
could_not_reach = could not reach {0}: {1}
bind_failed = bind failed: {0}
wizard_defaults_note = defaults only — an explicit row value always wins; bind target: global
wizard_preset_aria = profile preset
wizard_knob_scope = default scope
wizard_knob_pii = pii mode
wizard_knob_retention = retention
wizard_knob_audit = audit
wizard_knob_kinds = kinds
wizard_knob_hold = legal hold
err_title = Something went wrong
err_body = The client hit an unexpected error. Reload to retry.
err_dismiss = Dismiss
## Command palette (v1.27.20 M3.2)
system_title = System
register_title = Agent Memory Register
clients_title = Clients (BPO console)
palette_open = Open
palette_open_proposal = Open proposal
palette_open_chunk = Open chunk
palette_open_entity = Open entity
palette_export_audit = Export audit
palette_export_ump = Export UMP
reindex_title = Reindex
refresh_label = Refresh
palette_open_trace = Open trace
palette_modal_label = command palette
palette_placeholder = type a command… (↑↓ to move, Enter to run, Esc to close)
palette_filter_aria = command filter
palette_no_match = no match
## Recall surface (v1.27.20 M3.1)
recall_placeholder = query brain-server (min 5 chars)…
recall_query_aria = recall query
recall_trace_toggle = trace decision path
recall_min_relevance = min relevance
recall_rel_any = any
recall_rel_medium_plus = medium+
recall_rel_high = high
recall_summary = decision: {0} · {1} hits
recall_trace_link = decision-path trace #{0} ↗
recall_no_hits = no hits
recall_failed = recall failed: {0}
recall_chunk_id = chunk #{0}
recall_score = {0} score {1}
recall_via = via {0}
recall_relevance = relevance: {0}
recall_confidence = conf {0}
recall_decayed = decayed
recall_superseded = superseded
replay_back = ← back to recall
replay_sub = the recorded decision path for a past recall (replayable audit artifact)
replay_failed = trace failed: {0}
replay_loading = loading…

## Security surface (v1.27.20 M3.1)
sec_audit_chain = Audit chain
sec_chain_ok = chain ok
sec_chain_tampered = CHAIN TAMPERED
sec_trust_anchor = the trust anchor
sec_verify_chain = Verify audit chain
sec_quarantine_title = Quarantine ({0})
sec_chunk_id = chunk #{0}
sec_release = Release
sec_delete = Delete
sec_source = source: {0}
sec_no_quarantine = no quarantined chunks
sec_quarantine_failed = quarantine failed: {0}
sec_auth_failures = Auth failures ({0})
sec_no_auth_failures = no recent denied-auth events

## System surface (v1.27.20 M3.1)
sys_snapshot_ok = ok
sys_snapshot_degraded = degraded
sys_snapshot_count = {0} snapshots
sys_col_file = file
sys_col_size = size
sys_col_perms = perms
sys_col_integrity = integrity
sys_col_chain = audit chain
sys_perms_0600 = 0600
sys_world_readable = world-readable
sys_yes = yes
sys_no = no
sys_reindex_result = {0} · {1} re-embedded · {2} skipped
sys_reconcile = reconcile sources
sys_reconcile_result = {0} retired · {1} chunks

## Register surface (v1.27.20 M3.1)
register_title = Agent Memory Register
register_sub = Read-only provenance ledger — who wrote each memory and what it is based on.
register_all = All
register_owner_ph = owner…
register_source_ph = source…
register_kind_ph = memory kind…
register_failed = register failed: {0}
register_empty = no memories match the filter.
register_owner = owner {0}
register_evidence = evidence
register_evidence_modal = evidence for chunk
register_evidence_title = Evidence — chunk #{0}
register_src = src {0}
register_rev = rev {0}
register_lines = lines {0}–{1}
register_ev_failed = evidence failed: {0}
register_ev_loading = loading evidence…

## Graph surface (v1.27.20 M3.1)
graph_col_entity = entity
graph_col_depth = depth
graph_col_domain = domain

## Health/Data/Graph/misc labels (v1.27.20 M3.1)
data_ids_lbl = Chunk ids
data_owner_lbl = Owner
data_json = JSON
data_ump = UMP
data_ump_md = UMP Markdown
graph_no_paths = no paths
dsar_running = running…
review_sourcing_prompt = sourcing prompt
review_approve_supersede =  & supersede
ump_verify_chain = verify chain
sys_multi_domains =  · multi
health_dl_service = Service
health_dl_status = status
health_dl_version = version
health_dl_docs = docs
health_dl_rss = rss
health_dl_capacity = capacity
health_dl_unavailable = unavailable
health_dl_unsafe = unsafe blocks
health_dl_panics = panics caught
health_dl_corpus = Corpus
health_dl_chunks = chunks
health_dl_embeddings = embeddings
health_dl_entities = entities
health_dl_relationships = relationships
health_dl_model = model
health_dl_profile = profile
health_dl_scope = default scope
health_dl_pii = pii mode
health_dl_retention = retention
health_dl_audit = audit level
health_dl_kinds = kinds
health_dl_hold = legal hold default
health_dl_note = note
health_failed = health failed: {0}

## Confirm dialog + HTTP method labels (v1.27.20 M3.1)
confirm_cancel = Cancel
sys_http_get = GET
sys_http_post = POST
sys_http_delete = DELETE

## Input placeholders (v1.27.20 M3.1) — wire-value examples, locale-invariant
ump_content_ph = content...
ump_query_ph = query…
ump_kind_ph = kind (opt)
ingest_kinds_ph = fact · procedure · step · decision
ingest_domain_ph = global
data_ids_ph = 1, 2, 3
data_owner_ph = user@example.com
data_kind_ph = fact
data_days_ph = 90
# v1.28.4 approval dock
approval_dock_title = Approvals
pending_suffix = pending
dock_empty = Queue clear — nothing awaiting a decision.
dock_sla = {0} left to decide
dock_approve_aria = Approve proposal #{0}
dock_reject_aria = Reject proposal #{0}
dock_load_failed = Could not load the approval queue.
dock_invisible_removed = invisible characters removed: {0}

# v1.28.19 Witness — the run conversation surface
runs_title = Run {0}
runs_askhuman = Answer owed
runs_answer_placeholder = Your answer to the run…
runs_submit = Send answer
runs_transcript = Transcript
runs_empty = No events yet — the stream is listening.
runs_steer = Steer the run (advisory)
runs_steer_placeholder = Guidance for the engine's next step…
runs_send = Steer
runs_branches = {0} branch(es) in this run's history
connect_needed = Connect to brain-server to follow this run.

# v1.28.20 Cockpit
close = Close
loading = Loading…
nav_scoreboard = Scoreboard
runs_timeline = Timeline
runs_lineage = Run lineage
runs_streaming = streaming…
runs_tool_running = running
runs_tool_settled = done
runs_tool_error = error
runs_delivery = Delivery packet
runs_delivery_done = complete
runs_handoff_title = Handoff packet (I-PASS)
runs_crank_label = Crank (steps)
runs_crank_unwired = No HTTP crank yet — run `brain workflow crank <run>` until the engine-pull worker ships.
runs_help_title = Keyboard & commands
runs_help_keys = Keys: J/K walk nodes · A approve · R reject · ? this sheet
runs_help_commands = Commands: /crank [steps] · /handoff · /scoreboard · /help
tl_checkpoint = checkpoint
tl_branch = branch
tl_askhuman = needs human
ev_findings = Findings
ev_contradictions = Contradictions
ev_evidence = Evidence digests
ev_questions = Verification questions
sb_title = Workflow scoreboard
sb_fcr = First-contact resolution
sb_repeat = Repeat contact rate
sb_correctness = Correctness
sb_override = Override rate
sb_gap = Gap rate
sb_abstention = Abstention rate
sb_guidance = Guidance acceptance
sb_handoff = Handoff completeness
sb_escalation = Escalation honored
sb_audit_ok = Audit chain green
sb_audit_notok = Audit unverified
sb_runs = {0} runs scored
sb_calibrated = Calibration report emitted
