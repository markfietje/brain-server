# The "Complete" Operator Console (v1.17.6 → v1.17.8)

Brain Server's client control surface grew from a review/recall dashboard into a **full operator console** over three releases (v1.17.6, v1.17.7, v1.17.8 — the "Complete" line). It now has **12 panels** covering the entire lifecycle: write-back review, retrieval, the knowledge graph, the write workspace, governance, portability, and system operations.

This page is the map of that console. Everything below is client-side; the server + API contract stayed at **1.17.5** across the three releases (zero server changes, zero schema change).

## The three releases

| Release | Theme | What landed |
|---|---|---|
| **v1.17.6 "Complete 1/3"** | The spine | **Command palette v2** (fused nav + lookup + action, grouped, persisted recents, two-step destructive confirm) + the **Overview** home (4-card status row, DAR-chain alert list, top-5 pending queue) + Connect moved to `/connect` |
| **v1.17.7 "Complete 2/3"** | Graph + Create | **Graph panel** (entity lookup + typed hop-chain traversal) + **Create workspace** (ingest tabs, procedures step-builder, classify, decision evaluation, consolidate) |
| **v1.17.8 "Complete 3/3"** | Data + UMP + System | **Data & Rights** (purge, export, retention), **UMP panel** (capabilities, remember, recall, audit), **System panel** (domains, snapshot, Art 30, reindex, connectors, Try-it console) |

## The 12 panels

| Group | Panel | Route | Purpose |
|---|---|---|---|
| Overview | **Overview** | `/` | Decision-first home; status cards + alerts + pending queue |
| Review | **Review** | `/review` | Write-back approval queue (A/S/R/J/K) |
| Retrieve | **Recall** | `/recall` | Search + decision-path viewer |
| Explore | **Graph** | `/graph` | Knowledge-graph lookup + traversal |
| Write | **Create** | `/create` | Ingest / procedures / consolidate hub |
| Governance | **Subjects** | `/subjects` | DSAR certificates |
| Governance | **Security** | `/security` | Audit chain, quarantine, auth-failure feed |
| Governance | **Audit** | `/audit` | Audit filters + JSON export |
| Rights | **Data** | `/data` | Purge, export, retention, decayed, tombstones |
| Portability | **UMP** | `/ump` | Universal Memory Protocol operations |
| System | **System** | `/system` | Domains, snapshot, Art 30, reindex, connectors, Try-it |
| System | **Health** | `/health` | Service + corpus status |

## v1.17.8 in detail

**M5 — Data & Rights (`/data`).** The v1.14/v1.15 lifecycle surface in one place:
- **Purge** — `POST /purge` by comma/space/newline-separated ids or an owner email.
- **Portable export** — `GET /export` as JSON, UMP, or UMP-Markdown via the browser download seam.
- **Per-kind retention editor** — `GET /retention` → editable per-kind `days` overrides with a one-click `×` clear.
- **`/decayed`** review list and **`/tombstones`** deletion registry. Status region is `role="status" aria-live="polite"`.

**M6 — UMP panel (`/ump`).** The v1.17.3/v1.17.4 wire surface:
- **Capabilities card** with a `ump_integrity_badge` (L1–L3 conformance label).
- **Remember** — `POST /ump/remember` (JSON body → `{ok, id}`).
- **Recall** — `POST /ump/recall` with a kind filter and `max_recall` clamped to 1..100.
- **Audit** — load + verify the UMP audit chain.

**M7 — System panel (`/system`).**
- Domains list, snapshot integrity, the **Art 30** register (pretty-JSON).
- `POST /reindex`, connectors list (`kind · instance / state`), `POST /sources/reconcile`.
- A **Try-it console** with `get_raw` / `post_raw` / `delete_raw`, a request-line builder, and `redact_for_history` so the persisted in-memory history never stores a token-bearing body.

**M8 — wrap.** Three new routes (`/data`, `/ump`, `/system`) under the AppShell, all added to the sidebar rail + mobile tab bar + command palette (nav targets now **12**); new i18n keys in all five locales (each locale now **50 keys**).

## Version & quality

- Client `Cargo.toml` 1.17.0 → 1.17.8 across the line; server + API contract unchanged at **1.17.5**.
- **73 client tests** at v1.17.8 (was 49 at v1.17.6); clippy `-D warnings`, `fmt`, and wasm builds all green.
- The root cause of the Dioxus call-syntax build failures was fixed once in `api.rs`: `Clone` on the typed wire structs so `Signal<T>()` reads work.

## Deployment

```bash
cd client && ./deploy-web.sh   # builds wasm + tailwind, deploys to client/dist (served at /app)
```

## Related

- **[Client GUI](./client-gui.md)** — the full panel reference.
- **[Universal Memory Protocol](./universal-memory-protocol.md)** — the wire surface the UMP panel drives.
- **[Governance & Compliance](./compliance.md)** — the rights/retention surface Data exposes.
- **[Roadmap & Release History](./roadmap-and-release-history.md)** — the version line.
