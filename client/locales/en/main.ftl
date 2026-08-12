# v1.16.8 M1 — English strings (human-authored first cut).
# Simple FTL subset: `key = value`, `#` comment lines, blank lines skipped.
# A key missing in another locale falls back to this file via i18n.rs `t()`.

## Panel titles
review_title = Review queue
recall_title = Recall inspector
subjects_title = Subjects (DSAR)
security_title = Security
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
install_hint = One-line install:  curl -fsSL … | sh   then  brain doctor

## Review
no_pending = No pending proposals.
approve = Approve
reject = Reject
proposal = Proposal
review_help = Review keyboard shortcuts
review_help_toggle = Toggle shortcuts help
review_key_approve = Approve the focused proposal
review_key_supersede = Approve, superseding the conflicting proposal
review_key_reject = Reject the focused proposal
review_key_next = Next proposal
review_key_prev = Previous proposal

## Subjects
deletion_certificate = Deletion certificate
chain_verified = chain verified
chain_tampered = CHAIN TAMPERED

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
proc_created = Procedure created ({n} steps)
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
cons_applied = Applied {n} supersessions
cons_undone = Undone {n}

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
data_retention_set = Updated {n} override(s)
data_decayed = Decayed
data_tombstones = Tombstones
data_empty = Nothing to show.
data_purge_ids = Chunk ids (comma/space separated)
data_purge_owner = Or purge all for owner
data_purged = Purged {n} chunk(s)
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
sys_reindexed = Reindexed {n} chunks
sys_sources = Sources & connectors
sys_console = Try-it console

## Operations (v1.20.6 M1–M3 — Memory Operations panel)
nav_ops = Operations
ops_title = Operations
ops_sub = The live HITL work surface: pending queue, SLA clocks, and the flagged screen output.
ops_queue = Live queue
ops_queue_summary = pending
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
