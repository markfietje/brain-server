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
