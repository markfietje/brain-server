# v1.16.8 M1 — Dutch strings (human-reviewed first cut). Keys missing here
# fall back to `en` (see i18n.rs `t()`).

review_title = Reviewwachtrij
recall_title = Recall-inspecteur
subjects_title = Personen (DSAR)
security_title = Beveiliging
audit_title = Audit
health_title = Status

nav_review = Review
nav_recall = Recall
nav_subjects = Personen
nav_security = Beveiliging
nav_audit = Audit
nav_health = Status

pending = in behandeling
flags = vlaggen
audit_chain = auditketen!
acting_as = aangemeld als
loopback = loopback
sign_out = Afmelden
connected = verbonden
reconnecting = opnieuw verbinden
disconnected = Verbinding verbroken — laat laatste bekende status zien. Schrijven uitgeschakeld.
reverifying = Opnieuw verbonden — auditketen controleren alvorens schrijven in te schakelen…
detail = Detail
close_drawer = tekening sluiten
nothing_selected = niets geselecteerd

connect_title = Verbinden met brain-server
connect_welcome = Gereguleerd geheugen, op jouw hardware.
backend_url = Backend-URL
token_label = Token
url_placeholder = leeg = oorsprong van deze pagina (zelfde server)
token_placeholder = optioneel (loopback)
jwt_pair = JWT-paar (access + refresh) — schakelt stil refresh in
refresh_token_label = Refresh-token
refresh_token_placeholder = van `brain key mint` of een IdP
connecting = Verbinden…
connect_button = Verbinden
install_hint = Eénregelige installatie:  curl -fsSL … | sh   daarna  brain doctor

no_pending = Geen wachtende voorstellen.
approve = Goedkeuren
reject = Afwijzen
proposal = Voorstel

deletion_certificate = Verwijderingscertificaat
chain_verified = keten geverifieerd
chain_tampered = KETEN GEMANIPULEERD

theme_label = Thema
locale_label = Taal
density_label = Dichtheid
dark = donker
light = licht
comfortable = comfortabel
compact = compact

## Overview (v1.17.6 M2)
nav_overview = Overzicht
overview_title = Overzicht
overview_health = Status
overview_snapshot = Snapshot-integriteit
overview_retention = Bewaarbeleid
overview_ump = Server + UMP
overview_alerts = Waarschuwingen
no_alerts = Geen waarschuwingen — alles rustig.
open_queue = wachtrij openen
view = bekijken
kinds = soorten
alert_auth_failures = auth-fouten
alert_quarantine = in quarantaine
alert_stale_sources = verouderde bronnen
alert_conflicts = onopgeloste conflicten
alert_decayed = verlopen blokken
alert_near_duplicates = bijna-duplicaten
alert_tombstones = tombstones

## Command palette (v1.17.6 M1)
palette_recent = Recent
palette_go_to = Ga naar
palette_lookup = Zoeken
palette_run = Uitvoeren
confirm_destructive = Druk op Enter om te bevestigen

## Graph (v1.17.7 M3)
nav_graph = Grafiek
graph_title = Grafiek
graph_entity_ph = Zoek een entiteit…
graph_browse = Entiteit
graph_type = type
graph_relations = relaties
graph_traverse = Doorlopen
graph_start = Startentiteit
graph_depth = Max. diepte
graph_kind = Randtype (optioneel)
graph_at = Geldig op (optioneel)
graph_cross_domain = Over domeinen
graph_run = Doorlopen
graph_rel = relatie
graph_out = uit
graph_in = in
graph_no_entity = Bestaat niet
graph_paths = paden
graph_rows = rijen
none = geen

## Create (v1.17.7 M4)
nav_create = Maken
create_title = Maken
create_sub = Schrijfhulpmiddelen: geheugen vastleggen, procedures bouwen, consolideren.

## Ingest (v1.17.7 M4.1)
ingest_title = Vastleggen
ingest_tab_structured = Gestructureerd
ingest_tab_markdown = Markdown
ingest_tab_memory = Geheugenbatch
ingest_content = Inhoud
ingest_kind = Geheugensoort
ingest_domain = Domein
ingest_entities = Entiteiten (JSON)
ingest_relations = Relaties (JSON)
ingest_source_path = Bronpad
ingest_replace = Bestaande vervangen
ingest_submit = Vastleggen
ingest_bad_json = Entiteiten/relaties moeten geldige JSON zijn
ingest_mem_hint = Eén geheugen per regel, optionele ## titels
outcome_created = Aangemaakt
outcome_duplicate = Duplicaat (al aanwezig)

## Procedures (v1.17.7 M4.2)
proc_title = Procedures
proc_step_title = Staptitel
proc_step_body = Stapinhoud
proc_add_step = Stap toevoegen
proc_create = Procedure maken
proc_steps = Stappen
proc_is_decision = Beslisregel
proc_created = Procedure aangemaakt ({n} stappen)
cls_title = Classificeren
cls_text = Te classificeren tekst
cls_run = Classificeren
dec_title = Beslissing evalueren
dec_id = Beslissings-ID
dec_vars = Variabelen (JSON)
dec_run = Evalueren

## Consolidate (v1.17.7 M4.3)
cons_title = Consolideren
cons_load = Laden
cons_apply = Vervanging goedkeuren
cons_undo = Ongedaan maken
cons_empty = Niets te consolideren.
cons_near_dup = bijna-duplicaat
cons_conflict = conflict
cons_applied = {n} vervangingen toegepast
cons_undone = {n} ongedaan gemaakt

nav_data = Gegevens
nav_ump = UMP
nav_system = Systeem
data_title = Gegevens
data_sub = Rechten & portabiliteit: wissen, export, bewaartermijn, registers.
data_status = Gereed
data_purge = Wissen
data_export = Exporteren
data_exported = Export gegenereerd
data_retention = Bewaartermijn
data_retention_state = Bewaartermijn
data_retention_kind = Soort
data_retention_days = Dagen
data_retention_bad_days = Dagen moet een geheel getal zijn
data_retention_set = {n} overschrijving(en) bijgewerkt
data_decayed = Verlopen
data_tombstones = Tombstones
data_empty = Niets te tonen.
data_purge_ids = Chunk-ID's (gescheiden door komma/spatie)
data_purge_owner = Of alles wissen voor deze eigenaar
data_purged = {n} chunk(s) gewist
data_purge_empty = Geef ID's of een eigenaar op
ump_title = UMP
ump_sub = UMP 1.0-bewerkingen: capabilities, remember, recall, audit.
ump_caps = Mogelijkheden
ump_remember = Remember
ump_recall = Recall
ump_audit = Audit
ump_bad_json = Ongeldige JSON
ump_remembered = Opgeslagen
ump_chain_ok = Ketting geverifieerd
ump_chain_bad = Ketting gemanipuleerd
sys_title = Systeem
sys_sub = Operatorconsole: domeinen, snapshot, Art 30, bronnen, try-it.
sys_domains = Domeinen
sys_snapshot = Snapshot-integriteit
sys_art30 = Art 30-register
sys_reindex = Opnieuw indexeren
sys_reindexed = {n} chunks opnieuw geïndexeerd
sys_sources = Bronnen & connectoren
sys_console = Try-it-console
