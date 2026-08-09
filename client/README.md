# brain-client

The Dioxus control surface for brain-server — **one Rust codebase → web +
desktop + iOS + Android**. See `../IMPLEMENTATION_PLAN_v1.16.0_Client.md` and
`../DESIGN_v1.16.0_Client.md` for the full architecture and UX.

## Status — v1.17.7 "Complete 2/3" (shipped)

All panels are fully wired against the live brain-server API, behind a
connect-first onboarding screen (DESIGN §3). A typed client in `src/api.rs`
mirrors `openapi.yaml`; the panels drive re-fetch through `use_resource`. v1.17.7
adds the **Graph** (`/graph`) browse/traverse panel and the **Create**
(`/create`) workspace (Ingest / Procedures / Consolidate) — both reachable from
the sidebar rail, mobile tab bar, and command palette (9 nav targets).

**v1.17.0 ships the full client line** (49 tests, clippy `-D warnings` + fmt
clean). The M1 secure-storage + M2 responsive-UX halves of the v1.17.0 Mobile
plan shipped as v1.16.6 (OS-keyring/Keystore token seam; bottom-tab rail swap;
drawer→sheet; ≥44pt touch targets; safe-area insets); v1.17.0 completes the
mobile plan — **M2.4 portable refresh control** on Review/Audit/Health, **M3.3
deep-link intent filters** (`brain://` scheme on iOS + Android), **M3.4 offline
connect pre-fill** (last base URL persisted + specific failure, no crash), and
**M3.1 store-readiness privacy labels** (`STORE_READINESS.md`, "no data
collected" — accurate). Native iOS/Android bundling (`dx bundle --platform
{ios,android}`) is an operator step (signing + Android SDK; not in this env).

The v1.16.x line underneath:


| M | Feature | Detail |
|---|---|---|
| **M1** | Connection state machine | Single `use_future` probe with a false-offline guard (N failures before amber) + chain-verify-before-writes recovery. Dependency-free sleep via `document::eval`+setTimeout. Read-only degrade banner + mutation freeze when disconnected. |
| **M2** | Nav badges + principal + drawer | F-pattern `Pending: N` top-left, Security/Audit count badges, principal identity pillar, Esc-closable context drawer (ARIA dialog, hand-rolled focus trap). |
| **M3** | Honest-batch review | Per-row `RowOutcome` tracking (a failed call is surfaced, never silently dropped); 404-no-pending = success; `BatchGuard` DropGuard; A/S/R/J/K keyboard with a WCAG 2.1.4 toggle; reject-with-reason + suggest-re-ingest editors. |
| **M4** | Recall decision-path viewer | Per-retriever ranks, fused score, relevance tiers, `assertion_kind`/`confidence`/`decayed` tags; `min_relevance` slider; deep-linkable `?trace=true` artifact via `/recall/:trace_id`. |
| **M5** | DSAR certificate card | Structured card (found_count, purged_ids, tombstone_root, chain_head, certified_at) with a live green/red chain badge. |
| **M6** | Auth-failure feed | `GET /audit?kind=auth` filtered to denied rows; count badge on Security. |
| **M7** | Audit filters + export | Client-side principal/kind/since filters + JSON export; v1.16.7 adds server-side pagination (`?limit=&offset=`) with a Load-more button. |
| **M8** | Visual-token layer | Every panel uses the dark-first semantic tokens (`text-ink-muted`/`text-ok`/…) — zero ad-hoc color classes remain. |

**v1.16.x additions after v1.16.0:** v1.16.2 hardened + accessible (path-aware
CSP, ErrorBoundary, WCAG 2.2 AA pass, focus-to-heading, aria-live) · v1.16.4
shadcn-style design-system restyle · v1.16.5 JWT refresh lifecycle +
principal · v1.16.6 mobile: secure token storage (OS keyring) + responsive
bottom-tab layout · v1.16.7 deep links + PWA (offline shell, never the API)
+ ⌘K command palette + recall debounce · **v1.16.8 Global**: i18n (`en`/`de`/
`fr`/`es`/`nl` via zero-dep FTL-subset `t()`), light/dark theme + density toggles
(persisted, sanitized), RTL `dir` readiness, locale-aware number grouping, and
a privacy-transparency block on the connect screen. **v1.17.0 Mobile**: portable
refresh (Review/Audit/Health) + `brain://` deep-link intent filters + offline
connect pre-fill + store-readiness privacy labels.

| Panel | Backend route(s) | v1.16.0 additions |
|---|---|---|
| Review (approval queue) | `/ingest/proposal`, `/proposals`, `/proposals/{id}/approve\|reject` | per-row outcomes, A/S/R/J/K, reject reason, suggest re-ingest |
| Recall (decision viewer) | `/recall`, `/recall/{id}/trace` | richer hits, min_relevance slider, trace artifact |
| Subjects (DSAR) | `/dsar`, `/dsar/{id}/certificate` | certificate card + live chain badge |
| Security (quarantine + chain) | `/quarantine`, `/quarantine/{id}/release\|delete`, `/audit/verify`, `/audit?kind=auth` | auth-failure feed |
| Audit (hash-chain browser) | `/audit?limit=&offset=` | client-side filters + JSON export + Load-more paging |
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
 **43 tests**, clippy-clean), but shipping the web bundle needs `dx`.

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
the v1.16.x ceilings: wasm-split code-splitting (M3, blocked on Dioxus),
Radix/`dx components` focus restoration (registry unreachable), the simple
FTL-subset i18n (fluent/plurals/RTL-locale files are the upgrade path), and
system-color-scheme auto-follow.
