# Connectors — supervised external backfill

Connectors let Brain Server **backfill external sources** into the existing
source/revision pipeline, supervised by an operator — the same way you ingest
markdown or memories, but from a live external system (today: **GitHub**).

This page is verified against `src/connector/`, `src/bin/brain-connector-gh.rs`,
and the `connect`/`sync`/`connector-status` commands in `src/bin/brain.rs`.

## What a connector is

A connector is a supervised ingester. It fetches items from an external system
and feeds them through the **same** source + immutable-`revision` pipeline the
manual ingest paths use — so connector-loaded content carries full provenance,
participates in the knowledge graph and hybrid recall, and is reconciled like any
other source. The `connector` ingest kind is recorded on every chunk.

A **supervisor** process owns the lifecycle: register → authenticate → sync →
reconcile → report. The operator sees and controls it; nothing runs
autonomously.

## Today's connector: GitHub issues (App auth)

The shipped connector pulls **GitHub issues** for configured repositories,
authenticating as a GitHub **App** (installation access token), not a personal
token.

### Prerequisites

- A GitHub **App** with an installation on the target org/repos.
- The App's **App ID** and **Installation ID**.
- The App's **private key file** (PEM) — used to mint the short-lived
  installation token.
- (Optional) a **webhook secret file** for the issue webhook path.

### Register (authenticate)

```bash
brain connect github \
  --app-id 123456 \
  --install-id 9876543 \
  --key-file ./github-app.pem \
  --repo acme/widgets --repo acme/docs
```

The GitHub App flow is implemented in `src/connector/auth/github_app.rs`
(`GitHubAppConfig` / `GitHubAppProvider`) and the HTTP client in
`src/connector/github/client.rs` — an installation token is minted from the App
key and used for the fetch.

### Sync (backfill)

```bash
# backfill the registered instance(s)
brain sync github --config PATH            # explicit config file
brain sync github --instance NAME          # a named registered instance
```

- `brain sync` is backed by `brain-connector-gh`, a **separate feature-gated
  binary** (`--features connector-github`) because it pulls in the GitHub
  HTTP client.
- Backfill functions: `backfill_issues_for_repo` and `reconcile_github_sources`
  (`src/connector/github/`).

### Inspect

```bash
brain connector-status        # id, kind, instance, state, last_sync_at
```

`connector-status` reads `GET /connectors` and prints the registered
connectors; if none are registered it prints the `brain connect` usage line.
The `kind` column currently shows `github`.

## Feature gate

The `brain-connector-gh` binary is **feature-gated**:

```bash
cargo build --release --features connector-github --bin brain-connector-gh
```

The `brain connect`/`sync`/`connector-status` commands in the main `brain`
binary are always compiled (they delegate to the server / connector binary as
appropriate); only the standalone connector binary needs the feature.

## The `connector` ingest kind

Chunks loaded by a connector are tagged with the `connector` ingest kind
(alongside `github`, `web`, …), are stamped `imported` **origin** (per
`gate::origin_for_source`), and receive a confidence ×0.9 discount — the same
"imported content is trusted less than a human-authored manual fact" rule that
applies to other imported paths. See **[Memory lifecycle](./memory-lifecycle.md)**
for the origin mapping.

## Reconciliation

Like file/markdown sources, connector sources can be reconciled — orphans from
sources that were deleted are swept so the shared store doesn't answer from dead
material:

```bash
brain reconcile <path> [--kind vault]
# or over HTTP:
POST /sources/reconcile
```

## Security model

- Auth is **App-scoped**, never a personal token — least privilege, revocable,
  short-lived installation tokens minted per sync.
- Connector config lives under
  `~/.config/brain-server/connectors/github-{instance}.json` (mode-checked
  like other secrets; the server's fail-closed secret-permission check applies
  to the configured key/secret files).
- Sync is **operator-initiated**; there is no autonomous background fetch. The
  connector surfaces its state (`state`, `last_sync_at`) for operator review.

## CRM case connectors (v1.28.22 "Bridges")

`brain-connector-crm` (feature `connector-crm`) is one binary, three sources
(`--source zendesk|salesforce|genesys`), operator-cranked via cron — the same
discipline as GitHub: config-derived hosts only, redirects refused, bounded
timeouts, secrets in 0600 files, cursors in a connector-owned state file.
Case bodies enter through the UMP ingest path (proposals under
`BRAIN_WRITE_POSTURE=review`); case envelopes open governed runs and post
`crm/case/updated` / `crm/case/closed` outbox events; the `crm_cases`
table binds each stable `case_ref` to its run. Customer identity is stored
only as a salted SHA-256 subject ref. Cron recipes: [deployment](./deployment.md).
Custom CRMs (Freshdesk, ServiceNow, JSM): [connector-crm-custom.md](./connector-crm-custom.md).

## Honest ceiling

- Only `kind=github` is implemented (the CLI rejects any other kind with
  "other connectors land in v0.9.7+"). GitHub **issues** are the concrete
  backfill; the connector contract (`src/connector/mod.rs` +
  `src/connector/supervisor.rs`) is designed to be extensible to other kinds.
- It pulls issues via App auth over the GitHub REST API; it does not sync
  arbitrary repository content, PRs, or code.
- The CRM connectors are pull-only intake — there is no CRM **writeback**
  (posting resolutions back to the vendor is a later, separately-gated
  release), no background supervisor sync (cron only), and the custom-CRM
  path is docs + pure mappers, deliberately no generic JSONPath runtime.

## Next steps

- **[Source lifecycle](./features.md)** — provenance (`source` + immutable `revision`).
- **[Memory lifecycle](./memory-lifecycle.md)** — origin tiers and the `connector` kind.
- **[API reference](./api.md)** — `GET /connectors`, `POST /sources/reconcile`.