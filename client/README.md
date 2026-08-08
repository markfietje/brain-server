# brain-client

The Dioxus control surface for brain-server — **one Rust codebase → web +
desktop + iOS + Android**. See `../IMPLEMENTATION_PLAN_v1.16.0_Client.md` and
`../DESIGN_v1.16.0_Client.md` for the full architecture and UX.

## Status (scaffold)

This is the project skeleton — the router, AppShell, the typed API client, and
two fully-wired panels (**Recall** + **Health**) that work against a live
brain-server today. The other four panels are honest stubs that document which
backend release gates them:

| Panel | Works now? | Needs |
|---|---|---|
| Recall | ✅ | — (hits `/recall`) |
| Health | ✅ | — (hits `/health`) |
| Review | stub | brain-server v1.14.0 (`/ingest/proposal`, `/proposals`) |
| Subjects (DSAR) | stub | brain-server v1.15.0 (`/dsar`, `/tombstones`) |
| Security | stub | quarantine/audit exist; chain-verify is client recompute |
| Audit | stub | `/audit` exists; read-events need v1.15.0 |

## Prerequisites

- Rust (stable)
- The Dioxus CLI: `curl -fsSL https://dioxuslabs.com/install.sh | bash`
- A running brain-server on `127.0.0.1:8765` (loopback) with a bearer token

## Run

```sh
# from client/
dx serve --platform web        # WASM/DOM — the primary pilot surface (v1.16.0)
dx serve --platform desktop    # macOS/Windows/Linux — same code, no glue (v1.16.0)
dx serve --platform ios        # v1.17.0 (Keychain seam + App Store)
dx serve --platform android    # v1.17.0 (Keystore seam + Play Store)
```

The scaffold defaults the `ApiClient` to `http://127.0.0.1:8765` (see
`src/api.rs`). The connect-first onboarding screen (DESIGN §3) replaces this
with an interactive URL + token entry before v1.16.0 ships.

## Build / bundle

```sh
dx bundle --platform web       # static WASM bundle → brain-server serves /app/*
dx bundle --platform desktop   # native binary
dx bundle --platform ios       # .app + .ipa
dx bundle --platform android   # .apk / .aab
```

## Constraints honored

- **DOM renderer only on web** (never the experimental wgpu web renderer — it
  breaks screen readers / WCAG).
- **No client-side memory cache** — the backend is the source of truth.
- **No third-party analytics/CDN** — all assets vendored; telemetry is
  brain-server's `/metrics`.
- **Bearer in a header** (not a cookie) → CSRF-safe by construction.

## Next

Build out Review (v1.14.0), then Subjects/Security/Audit (v1.15.0), then the
connect-first onboarding + visual system (DESIGN doc), then iOS/Android native
(v1.17.0), then accessibility + i18n (v1.18.0), integration (v1.19.0), polish
(v1.20.0).
