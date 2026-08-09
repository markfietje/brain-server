# v1.16.8 M1 — German strings (human-reviewed first cut). Keys missing here
# fall back to `en` (see i18n.rs `t()`).

review_title = Prüfliste
recall_title = Recall-Inspektor
subjects_title = Personen (DSAR)
security_title = Sicherheit
audit_title = Audit
health_title = Status

nav_review = Prüfen
nav_recall = Abruf
nav_subjects = Personen
nav_security = Sicherheit
nav_audit = Audit
nav_health = Status

pending = ausstehend
flags = Markierungen
audit_chain = Audit-Kette!
acting_as = angemeldet als
loopback = loopback
sign_out = Abmelden
connected = verbunden
reconnecting = verbinde neu
disconnected = Getrennt — zeige letzten Stand. Schreibaktionen deaktiviert.
reverifying = Wiederverbunden — prüfe Audit-Kette vor dem Freigeben von Schreibzugriff…
detail = Detail
close_drawer = Zeichnung schließen
nothing_selected = nichts ausgewählt

connect_title = Mit brain-server verbinden
connect_welcome = brain — verwaltetes Gedächtnis, auf Ihrer Hardware.
backend_url = Backend-URL
token_label = Token
url_placeholder = leer = Ursprung dieser Seite (gleicher Server)
token_placeholder = optional (loopback)
jwt_pair = JWT-Paar (Access + Refresh) — aktiviert stilles Refresh
refresh_token_label = Refresh-Token
refresh_token_placeholder = von `brain key mint` oder einem IdP
connecting = Verbinde…
connect_button = Verbinden
install_hint = Einzeilen-Installation:  curl -fsSL … | sh   dann  brain doctor

no_pending = Keine ausstehenden Vorschläge.
approve = Genehmigen
reject = Ablehnen
proposal = Vorschlag

deletion_certificate = Löschzertifikat
chain_verified = Kette verifiziert
chain_tampered = KETTE MANIPULIERT

theme_label = Design
locale_label = Sprache
density_label = Dichte
dark = dunkel
light = hell
comfortable = bequem
compact = kompakt

## Overview (v1.17.6 M2)
nav_overview = Übersicht
overview_title = Übersicht
overview_health = Status
overview_snapshot = Snapshot-Integrität
overview_retention = Aufbewahrung
overview_ump = Server + UMP
overview_alerts = Warnungen
no_alerts = Keine Warnungen — alles ruhig.
open_queue = Warteschlange öffnen
view = ansehen
kinds = Arten
alert_auth_failures = Auth-Fehler
alert_quarantine = unter Quarantäne
alert_stale_sources = veraltete Quellen
alert_conflicts = ungelöste Konflikte
alert_decayed = abgelaufene Blöcke
alert_near_duplicates = Fast-Duplikate
alert_tombstones = Tombstones

## Command palette (v1.17.6 M1)
palette_recent = Zuletzt
palette_go_to = Gehe zu
palette_lookup = Nachschlagen
palette_run = Ausführen
confirm_destructive = Enter drücken zum Bestätigen

## Graph (v1.17.7 M3)
nav_graph = Graph
graph_title = Graph
graph_entity_ph = Entität suchen…
graph_browse = Entität
graph_type = Typ
graph_relations = Beziehungen
graph_traverse = Durchlaufen
graph_start = Start-Entität
graph_depth = Maximale Tiefe
graph_kind = Kantentyp (optional)
graph_at = Gültig am (optional)
graph_cross_domain = Domänenübergreifend
graph_run = Durchlaufen
graph_rel = Beziehung
graph_out = aus
graph_in = in
graph_no_entity = Keine solche Entität
graph_paths = Pfade
graph_rows = Zeilen
none = keine

## Create (v1.17.7 M4)
nav_create = Erstellen
create_title = Erstellen
create_sub = Schreibwerkzeuge: Speicher erfassen, Prozeduren bauen, konsolidieren.

## Ingest (v1.17.7 M4.1)
ingest_title = Erfassen
ingest_tab_structured = Strukturiert
ingest_tab_markdown = Markdown
ingest_tab_memory = Speicher-Batch
ingest_content = Inhalt
ingest_kind = Speicherart
ingest_domain = Domäne
ingest_entities = Entitäten (JSON)
ingest_relations = Beziehungen (JSON)
ingest_source_path = Quellpfad
ingest_replace = Vorhandenes ersetzen
ingest_submit = Erfassen
ingest_bad_json = Entitäten/Beziehungen müssen gültiges JSON sein
ingest_mem_hint = Ein Speicher pro Zeile, optionale ## Titel-Überschriften
outcome_created = Erstellt
outcome_duplicate = Duplikat (bereits vorhanden)

## Procedures (v1.17.7 M4.2)
proc_title = Prozeduren
proc_step_title = Schritttitel
proc_step_body = Schrittinhalt
proc_add_step = Schritt hinzufügen
proc_create = Prozedur erstellen
proc_steps = Schritte
proc_is_decision = Entscheidungsregel
proc_created = Prozedur erstellt ({n} Schritte)
cls_title = Klassifizieren
cls_text = Zu klassifizierender Text
cls_run = Klassifizieren
dec_title = Entscheidung auswerten
dec_id = Entscheidungs-ID
dec_vars = Variablen (JSON)
dec_run = Auswerten

## Consolidate (v1.17.7 M4.3)
cons_title = Konsolidieren
cons_load = Laden
cons_apply = Ablösung genehmigen
cons_undo = Rückgängig
cons_empty = Nichts zu konsolidieren.
cons_near_dup = Fast-Duplikat
cons_conflict = Konflikt
cons_applied = {n} Ablösungen angewendet
cons_undone = {n} rückgängig gemacht
