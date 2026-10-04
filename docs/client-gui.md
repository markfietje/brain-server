# Client GUI

Brain Server has **two** operator GUIs over the same HTTP API.

- **`client/` — the Dioxus control surface (Rust).** A single Rust codebase
  running as a **web app and a desktop app**, with 16 routed panels. This is
  the bundle the server serves at `/app` today (`BRAIN_CLIENT_DIST` defaults
  to `client/dist`). Its `mobile` feature is a compile-smoke target only — no
  store submission has shipped.
- **`shell/` — the SvelteKit + Tauri shell (the active successor).** A
  typed-wire SvelteKit SPA with a Tauri desktop core, built on its own CI
  workflow (`shell.yml`: lint, strict type-check, unit tests, byte-stable
  generated client, strict CSP build, dependency audit, Tauri fmt/clippy/audit
  and build, plus a Playwright e2e against a real loopback kernel). Its API
  client is **generated from the kernel's `openapi.yaml`**, and CI byte-compares
  a regeneration against the committed output, so contract drift is a red build.
  It ships 8 routes today (`/`, `/overview`, `/recall` + trace, `/search`,
  `/decisions` + detail, `/models`).

**Which one is live:** `/app` serves one bundle, chosen by `BRAIN_CLIENT_DIST`
(default `client/dist`). The Dioxus client's removal is **frozen until the
shell's parity gates pass** — see `shell/README.md`. So the Dioxus client is
what ships today and the SvelteKit + Tauri shell is what is being built toward.

Everything below documents the Dioxus client (`client/`), which is the surface
currently served.

## What the GUI provides

The client has **16 wired panels**, plus a connect-first onboarding flow, grouped under a sidebar rail (desktop) / bottom tab bar (mobile):

| Panel | Route | What it shows |
|---|---|---|
| **Overview** | `/` | Decision-first home: a 4-card status row (Health / Snapshot / Retention / UMP), a DAR-chain alert list, and a top-5 pending-proposal queue with one-click Approve/Reject |
| **Review** | `/review` | The human-in-the-loop write-back queue — approve, reject, or suggest re-ingest with the A/S/R/J/K keyboard (WCAG 2.1.4 toggle for sticky keys) |
| **Recall** | `/recall` | Search + the decision-path viewer: per-retriever ranks, fused score, relevance tiers, `min_relevance` slider, deep-linkable trace artifact |
| **Graph** | `/graph` | Browse + traverse the knowledge graph: debounced entity lookup and typed multi-hop hop-chains with a `kind` filter |
| **Create** | `/create` | The write workspace hub — Ingest (structured / markdown / memory), Procedures step-builder + classify + decision evaluation, and Consolidate propose/apply/undo |
| **Subjects** | `/subjects` | The DSAR certificate card — found/purged/tombstone-root/chain-head/certified-at + a live green/red chain badge |
| **Security** | `/security` | The audit chain card, quarantine review, and the auth-failure feed |
| **Audit** | `/audit` | Audit filters + JSON export |
| **Data** | `/data` | Data & Rights: purge (by ids or owner), portable export (JSON / UMP / UMP-Markdown), per-kind retention editor, the `/decayed` review list, and the `/tombstones` deletion registry |
| **UMP** | `/ump` | Universal Memory Protocol: capabilities card + integrity badge, remember, recall (kind filter + max_recall), and audit + verify chain |
| **System** | `/system` | The operator console: domains, snapshot integrity, Art 30 register, reindex, connectors + reconcile, and a Try-it console with request-line building + secret redaction |
| **Health** | `/health` | Service + corpus status |
| **Ops** | `/ops` | The live alert feed (SSE) + Memory Operations panel with per-proposal SLA clocks and the gate-health strip |
| **Register** | `/register` | The Agent Memory Register: provenance ledger by `origin` (`human` / `model` / `imported`) with owner/source/kind filters and drill-down evidence |
| **Clients** | `/clients` | BPO client register (role-gated): the console renders only the client(s) your token is granted (client-auditor) or the all-clients operations board (bpo-ops/admin) |
| **Scoreboard** | `/scoreboard` | Outcome/efficiency KPIs from `GET /workflow/scoreboard` over closed runs — FCR, resolution mix, goodwill ledger (server-side Admin+DPO gated; the panel is presentation only) |

### Detail routes

Beyond the panel grid, deep-linkable detail routes exist: `/review/:proposal_id` (share a single review card), `/runs/:run_id` and `/runs/:run_id/timeline` (a workflow run's transcript fed by the persistent stream), `/subjects/certificate/:dsar_id` (a chain-verifiable deletion certificate), and `/recall/:trace_id` (the decision-path artifact below). Unauthenticated visitors land on `/connect`.

### Command palette

The ⌘K / Ctrl+K overlay (v1.16.7) was upgraded to a fused **nav + lookup + action** palette (v1.17.6): grouped Recent/Go to/Lookup/Run rows, 5-per-group cap, persisted recents, `/` re-focus, a two-step destructive confirm, and per-row aria-labels.

### Honest-batch review

The Review panel tracks every row's outcome individually — a failed call is **surfaced, never silently dropped**. A 404 with nothing pending is treated as success. You can reject with a reason and suggest re-ingest. It is one surface of the human-in-the-loop control room — alongside the Memory Operations panel (live SLA clocks + gate health + flagged inventory) and the Agent Memory Register (provenance ledger). See [**Human in the loop**](./human-in-the-loop.md) for how to evaluate proposals as a critical operator, not a queue-clearer.

### Recall decision-path viewer

With `?trace=true`, `/recall` returns a `trace_id`; the GUI opens a deep-linkable artifact at `/recall/:trace_id` showing exactly which chunks were injected and why.

## Connection state machine

The client has a robust connection layer:

- A single probe with a **false-offline guard** — N failures before the indicator turns amber.
- **Chain-verify-before-writes** — writes stay frozen until `/audit/verify` confirms the audit chain is intact, then they re-enable.
- Reads degrade gracefully when the connection is amber; mutations freeze.

## Accessibility

The client is built to **WCAG 2.2 AA**:

- Focus-to-`<h1>` on navigation + per-route document titles.
- No `<div onclick>` — every interactive element is a real `<button>` or `<link>` (grep-guarded in CI).
- Aria-live regions, `dir="auto"` RTL, `scroll-margin-top`, and ≥44px touch targets.
- A hand-rolled drawer focus trap with Tab/Shift+Tab cycling.

See `client/a11y-checklist.md` in the repo for the manual VoiceOver/NVDA checklist.

## Deployment

```bash
# In the client/ directory — build the web bundle and deploy it
./deploy-web.sh
```

The web build ships as a PWA with an offline shell (the service worker caches only the shell + assets, never the API). The desktop build uses the same codebase.

For the SvelteKit + Tauri shell, `pnpm build` emits a static SPA into
`shell/build/`. It is built root-absolute (`/_app/...`), so serving it from the
`/app` seat needs a base-path build first; run it as its own origin (or as the
Tauri desktop app) as-is. See `shell/README.md` for the build, CI, and
security posture.

## Next steps

- **[Complete Operator Console](./client-complete-console.md)** — the 12-panel v1.17.6→v1.17.8 line in detail.
- **`shell/README.md`** (in the repo) — the SvelteKit + Tauri successor shell: run/build commands, the generated-wire drift gate, and its security posture.
- **[Installation](./deployment.md)** — serving the GUI at `/app`.
- **[API Reference](./api.md)** — the API the GUI talks to.
- **[Security](./security.md)** — how the GUI authenticates (JWT pairs, silent refresh).
