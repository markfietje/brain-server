# brain-client

The Dioxus control surface for brain-server — **one Rust codebase → web +
desktop + iOS + Android**. See `../IMPLEMENTATION_PLAN_v1.16.0_Client.md` and
`../DESIGN_v1.16.0_Client.md` for the full architecture and UX.

## Status — v1.16.0 "Client" (shipped)

All six panels are fully wired against the live brain-server API, behind a
connect-first onboarding screen (DESIGN §3). A typed client in `src/api.rs`
mirrors `openapi.yaml`; the panels drive re-fetch through `use_resource`.

**v1.16.0 ships the DESIGN's UX + correctness hard-parts** (8 milestones,
25 tests, clippy `-D warnings` + fmt clean):

| M | Feature | Detail |
|---|---|---|
| **M1** | Connection state machine | Single `use_future` probe with a false-offline guard (N failures before amber) + chain-verify-before-writes recovery. Dependency-free sleep via `document::eval`+setTimeout (no tokio dep). Read-only degrade banner + mutation freeze when disconnected. |
| **M2** | Nav badges + principal + drawer | F-pattern `Pending: N` top-left, Security/Audit count badges, principal identity pillar, Esc-closable context drawer (ARIA dialog). |
| **M3** | Honest-batch review | Per-row `RowOutcome` tracking (a failed call is surfaced, never silently dropped); 404-no-pending = success; `BatchGuard` DropGuard; A/S/R/J/K keyboard with a WCAG 2.1.4 toggle; reject-with-reason + suggest-re-ingest editors. |
| **M4** | Recall decision-path viewer | Per-retriever ranks, fused score, relevance tiers, `assertion_kind`/`confidence`/`decayed` tags; `min_relevance` slider; deep-linkable `?trace=true` artifact via `/recall/:trace_id`. |
| **M5** | DSAR certificate card | Structured card (found_count, purged_ids, tombstone_root, chain_head, certified_at) with a live green/red chain badge. |
| **M6** | Auth-failure feed | `GET /audit?kind=auth` filtered to denied rows; count badge on Security. |
| **M7** | Audit filters + export | Client-side principal/kind/since filters + JSON export (no new server route). |
| **M8** | Visual-token layer | Every panel uses the dark-first semantic tokens (`text-ink-muted`/`text-ok`/…) — zero ad-hoc color classes remain. |

| Panel | Backend route(s) | v1.16.0 additions |
|---|---|---|
| Review (approval queue) | `/ingest/proposal`, `/proposals`, `/proposals/{id}/approve\|reject` | per-row outcomes, A/S/R/J/K, reject reason, suggest re-ingest |
| Recall (decision viewer) | `/recall`, `/recall/{id}/trace` | richer hits, min_relevance slider, trace artifact |
| Subjects (DSAR) | `/dsar`, `/dsar/{id}/certificate` | certificate card + live chain badge |
| Security (quarantine + chain) | `/quarantine`, `/quarantine/{id}/release\|delete`, `/audit/verify`, `/audit?kind=auth` | auth-failure feed |
| Audit (hash-chain browser) | `/audit` | client-side filters + JSON export |
| Health (capacity + corpus) | `/health`, `/stats` | — |

## Prerequisites

- Rust (stable)
- The Dioxus CLI: `curl -fsSL https://dioxuslabs.com/install.sh | bash`
- A running brain-server (loopback `127.0.0.1:8765`, or any reachable host)

## Run

```sh
# from client/
dx serve --platform web        # WASM/DOM — the primary pilot surface (v1.16.0)
dx serve --platform desktop    # macOS/Windows/Linux — same code, no glue (v1.16.0)
dx serve --platform ios        # v1.17.0 (Keychain seam + App Store)
dx serve --platform android    # v1.17.0 (Keystore seam + Play Store)
```

On first load the connect screen asks for the backend URL + token (token
optional on loopback). It probes `GET /health` live before navigating into
the panels, and writes the connected `ApiClient` into the root context so
every panel re-fetches through it.

## Operator note: `dx serve` + CSP

`dx serve` / `dx bundle` are **operator steps** — the Dioxus CLI is not part
 of this repo's CI (it must be installed via the curl one-liner above). The
 WASM build can be validated with `cargo check`/`cargo test` (both green here:
 **30 tests**, clippy-clean), but shipping the web bundle needs `dx`.

Tailwind v4 (v1.16.2): `styles/input.css` is the source and
`assets/tailwind.css` the `asset!`-ed output. `dx bundle`/`dx serve` auto-run
the Tailwind CLI, but the v4 CLI is the `@tailwindcss/cli` npm package, so a
one-time build is needed when `assets/tailwind.css` is absent:

```sh
npm install --save-dev tailwindcss
npx @tailwindcss/cli -i styles/input.css -o assets/tailwind.css
```

Until `assets/tailwind.css` exists, the `document::Stylesheet` `asset!` href
fails at build time (asset is missing) — the styles land once the file is
present.

When serving the bundled web app behind brain-server (or any host), set the
CSP so the WASM fetch works:

```
default-src 'self';
script-src 'self' 'wasm-unsafe-eval';
connect-src 'self' <brain-server-origin>;
object-src 'none';
```

`connect-src` must include the brain-server origin you entered on the connect
screen (same-origin when brain-server serves `/app/*`).

## Build / bundle

```sh
dx bundle --platform web       # static WASM bundle → brain-server serves /app/*
dx bundle --platform desktop   # native binary
dx bundle --platform ios       # .app + .ipa
dx bundle --platform android   # .apk / .aab
```

The web bundle honors `base_path = "app"` in `Dioxus.toml` so all asset URLs
are `/app/assets/...` under the server's `/app` mount. To deploy the bundle to
the server-served dir (`client/dist`, gitignored), copy the built output:

```sh
dx bundle --platform web --release
SRC=target/dx/brain-client/release/web/public
rm -rf dist && mkdir -p dist/assets
cp "$SRC/index.html" dist/
cp "$SRC/assets/brain-client-*.js" "$SRC/assets/brain-client_bg-*.wasm" dist/assets/
```

(wasm-opt may SIGABRT on Apple Silicon — cosmetic; dx still emits the bundle.)

## Constraints honored

- **DOM renderer only on web** (never the experimental wgpu web renderer — it
  breaks screen readers / WCAG).
- **No client-side memory cache** — the backend is the source of truth.
- **No third-party analytics/CDN** — all assets vendored; telemetry is
  brain-server's `/metrics`.
- **Bearer in a header** (not a cookie) → CSRF-safe by construction.

## Next

Accessibility + i18n (v1.18.0), integration (v1.19.0), polish (v1.20.0), and
the mobile bottom-tab responsive swap (v1.17.0).
