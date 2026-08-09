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
