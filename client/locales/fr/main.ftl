# v1.16.8 M1 — French strings (human-reviewed first cut). Keys missing here
# fall back to `en` (see i18n.rs `t()`).

review_title = File d'attente
recall_title = Inspecteur de rappel
subjects_title = Personnes (DSAR)
security_title = Sécurité
audit_title = Audit
health_title = Santé

nav_review = Réviser
nav_recall = Rappel
nav_subjects = Personnes
nav_security = Sécurité
nav_audit = Audit
nav_health = Santé

pending = en attente
flags = alertes
audit_chain = chaîne d'audit !
acting_as = agissant comme
loopback = loopback
sign_out = Se déconnecter
connected = connecté
reconnecting = reconnexion
disconnected = Déconnecté — affichage du dernier état. Écritures désactivées.
reverifying = Reconnecté — vérification de la chaîne d'audit avant d'activer les écritures…
detail = Détail
close_drawer = fermer le panneau
nothing_selected = rien de sélectionné

connect_title = Connecter à brain-server
connect_welcome = brain — mémoire gouvernée, sur votre matériel.
backend_url = URL du backend
token_label = Jeton
url_placeholder = vide = origine de cette page (même serveur)
token_placeholder = facultatif (loopback)
jwt_pair = Paire JWT (access + refresh) — active le rafraîchissement silencieux
refresh_token_label = Jeton de rafraîchissement
refresh_token_placeholder = depuis `brain key mint` ou un IdP
connecting = Connexion…
connect_button = Connecter
install_hint = Installation en une ligne :  curl -fsSL … | sh   puis  brain doctor

no_pending = Aucune proposition en attente.
approve = Approuver
reject = Rejeter
proposal = Proposition

deletion_certificate = Certificat de suppression
chain_verified = chaîne vérifiée
chain_tampered = CHAÎNE ALTÉRÉE

theme_label = Thème
locale_label = Langue
density_label = Densité
dark = sombre
light = clair
comfortable = confortable
compact = compact

## Overview (v1.17.6 M2)
nav_overview = Aperçu
overview_title = Aperçu
overview_health = Santé
overview_snapshot = Intégrité des snapshots
overview_retention = Rétention
overview_ump = Serveur + UMP
overview_alerts = Alertes
no_alerts = Aucune alerte — tout est calme.
open_queue = ouvrir la file
view = voir
kinds = types
alert_auth_failures = échecs d'authentification
alert_quarantine = blocs en quarantaine
alert_stale_sources = sources obsolètes
alert_conflicts = conflits non résolus
alert_decayed = blocs expirés
alert_near_duplicates = quasi-doublons
alert_tombstones = tombstones

## Command palette (v1.17.6 M1)
palette_recent = Récents
palette_go_to = Aller à
palette_lookup = Rechercher
palette_run = Exécuter
confirm_destructive = Appuyez sur Entrée pour confirmer

## Graph (v1.17.7 M3)
nav_graph = Graph
graph_title = Graph
graph_entity_ph = Rechercher une entité…
graph_browse = Entité
graph_type = type
graph_relations = relations
graph_traverse = Parcourir
graph_start = Entité de départ
graph_depth = Profondeur max
graph_kind = Type d'arête (optionnel)
graph_at = Valide le (optionnel)
graph_cross_domain = Toutes domaines
graph_run = Parcourir
graph_rel = relation
graph_out = sortant
graph_in = entrant
graph_no_entity = Entité introuvable
graph_paths = chemins
graph_rows = lignes
none = aucun

## Create (v1.17.7 M4)
nav_create = Créer
create_title = Créer
create_sub = Outils d'écriture : mémoriser, créer des procédures, consolider.

## Ingest (v1.17.7 M4.1)
ingest_title = Mémoriser
ingest_tab_structured = Structuré
ingest_tab_markdown = Markdown
ingest_tab_memory = Lot mémoire
ingest_content = Contenu
ingest_kind = Type mémoire
ingest_domain = Domaine
ingest_entities = Entités (JSON)
ingest_relations = Relations (JSON)
ingest_source_path = Chemin source
ingest_replace = Remplacer l'existant
ingest_submit = Mémoriser
ingest_bad_json = Entités/relations doivent être du JSON valide
ingest_mem_hint = Une mémoire par ligne, titres ## optionnels
outcome_created = Créé
outcome_duplicate = Doublon (déjà présent)

## Procedures (v1.17.7 M4.2)
proc_title = Procédures
proc_step_title = Titre de l'étape
proc_step_body = Contenu de l'étape
proc_add_step = Ajouter une étape
proc_create = Créer la procédure
proc_steps = Étapes
proc_is_decision = Règle de décision
proc_created = Procédure créée ({n} étapes)
cls_title = Classifier
cls_text = Texte à classifier
cls_run = Classifier
dec_title = Évaluer la décision
dec_id = ID de décision
dec_vars = Variables (JSON)
dec_run = Évaluer

## Consolidate (v1.17.7 M4.3)
cons_title = Consolider
cons_load = Charger
cons_apply = Approuver le remplacement
cons_undo = Annuler
cons_empty = Rien à consolider.
cons_near_dup = quasi-doublon
cons_conflict = conflit
cons_applied = {n} remplacements appliqués
cons_undone = {n} annulés
