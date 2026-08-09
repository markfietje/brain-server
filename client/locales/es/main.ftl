# v1.16.8 M1 — Spanish strings (human-reviewed first cut). Keys missing here
# fall back to `en` (see i18n.rs `t()`).

review_title = Cola de revisión
recall_title = Inspector de recuerdo
subjects_title = Personas (DSAR)
security_title = Seguridad
audit_title = Auditoría
health_title = Salud

nav_review = Revisar
nav_recall = Recuerdo
nav_subjects = Personas
nav_security = Seguridad
nav_audit = Auditoría
nav_health = Salud

pending = pendientes
flags = avisos
audit_chain = ¡cadena de auditoría!
acting_as = actuando como
loopback = loopback
sign_out = Cerrar sesión
connected = conectado
reconnecting = reconectando
disconnected = Desconectado — mostrando el último estado. Escrituras desactivadas.
reverifying = Reconectado — verificando la cadena de auditoría antes de habilitar escrituras…
detail = Detalle
close_drawer = cerrar panel
nothing_selected = nada seleccionado

connect_title = Conectar a brain-server
connect_welcome = brain — memoria gobernada, en tu hardware.
backend_url = URL del backend
token_label = Token
url_placeholder = vacío = origen de esta página (mismo servidor)
token_placeholder = opcional (loopback)
jwt_pair = Par JWT (access + refresh) — activa actualización silenciosa
refresh_token_label = Token de refresco
refresh_token_placeholder = de `brain key mint` o un IdP
connecting = Conectando…
connect_button = Conectar
install_hint = Instalación de una línea:  curl -fsSL … | sh   luego  brain doctor

no_pending = No hay propuestas pendientes.
approve = Aprobar
reject = Rechazar
proposal = Propuesta

deletion_certificate = Certificado de eliminación
chain_verified = cadena verificada
chain_tampered = CADENA ALTERADA

theme_label = Tema
locale_label = Idioma
density_label = Densidad
dark = oscuro
light = claro
comfortable = cómodo
compact = compacto

## Overview (v1.17.6 M2)
nav_overview = Resumen
overview_title = Resumen
overview_health = Salud
overview_snapshot = Integridad del snapshot
overview_retention = Retención
overview_ump = Servidor + UMP
overview_alerts = Alertas
no_alerts = Sin alertas — todo tranquilo.
open_queue = abrir cola
view = ver
kinds = tipos
alert_auth_failures = fallos de autenticación
alert_quarantine = bloques en cuarentena
alert_stale_sources = fuentes obsoletas
alert_conflicts = conflictos sin resolver
alert_decayed = bloques caducados
alert_near_duplicates = casi duplicados
alert_tombstones = tombstones

## Command palette (v1.17.6 M1)
palette_recent = Recientes
palette_go_to = Ir a
palette_lookup = Buscar
palette_run = Ejecutar
confirm_destructive = Pulsa Intro para confirmar

## Graph (v1.17.7 M3)
nav_graph = Grafo
graph_title = Grafo
graph_entity_ph = Buscar una entidad…
graph_browse = Entidad
graph_type = tipo
graph_relations = relaciones
graph_traverse = Recorrer
graph_start = Entidad inicial
graph_depth = Profundidad máx
graph_kind = Tipo de arista (opcional)
graph_at = Válido en (opcional)
graph_cross_domain = Entre dominios
graph_run = Recorrer
graph_rel = relación
graph_out = saliente
graph_in = entrante
graph_no_entity = No existe la entidad
graph_paths = rutas
graph_rows = filas
none = ninguno

## Create (v1.17.7 M4)
nav_create = Crear
create_title = Crear
create_sub = Herramientas de escritura: memorizar, crear procedimientos, consolidar.

## Ingest (v1.17.7 M4.1)
ingest_title = Memorizar
ingest_tab_structured = Estructurado
ingest_tab_markdown = Markdown
ingest_tab_memory = Lote de memoria
ingest_content = Contenido
ingest_kind = Tipo de memoria
ingest_domain = Dominio
ingest_entities = Entidades (JSON)
ingest_relations = Relaciones (JSON)
ingest_source_path = Ruta de origen
ingest_replace = Reemplazar existente
ingest_submit = Memorizar
ingest_bad_json = Entidades/relaciones deben ser JSON válido
ingest_mem_hint = Una memoria por línea, títulos ## opcionales
outcome_created = Creado
outcome_duplicate = Duplicado (ya presente)

## Procedures (v1.17.7 M4.2)
proc_title = Procedimientos
proc_step_title = Título del paso
proc_step_body = Contenido del paso
proc_add_step = Añadir paso
proc_create = Crear procedimiento
proc_steps = Pasos
proc_is_decision = Regla de decisión
proc_created = Procedimiento creado ({n} pasos)
cls_title = Clasificar
cls_text = Texto a clasificar
cls_run = Clasificar
dec_title = Evaluar decisión
dec_id = ID de decisión
dec_vars = Variables (JSON)
dec_run = Evaluar

## Consolidate (v1.17.7 M4.3)
cons_title = Consolidar
cons_load = Cargar
cons_apply = Aprobar sustitución
cons_undo = Deshacer
cons_empty = Nada que consolidar.
cons_near_dup = casi-duplicado
cons_conflict = conflicto
cons_applied = {n} sustituciones aplicadas
cons_undone = {n} deshechas
