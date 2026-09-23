# brain-shell — the SvelteTauri shell (Phase A)

The typed-wire skeleton + the wizard renderer: a **pure protocol client**
for the brain-server kernel. Zero business logic lives here — every rule is
the kernel's; every read is a typed call generated from the kernel's
`openapi.yaml`. This is the SvelteTauri shell plan's M1 foundation + M5
renderer core, Phase A of the R23 round (`plans/EXECUTION_PLAN_R23_…` in the
spine repo).

**Deliberately NOT here (never stubbed — later rounds, gated):** intake
submission (the webhook-seam POST), kernel-side telemetry ingest, the pack
designer, cockpit surfaces (M3), keychain token storage + offline approval
queue (M2), installers/signing/auto-update/kernel sidecar (M4), the Dioxus
`client/` removal (frozen until Svelte parity gates pass), SvelteKit 3 (not
stable at this round's pin).

## Run it

```sh
pnpm install          # frozen-lockfile in CI; engines: node ≥24 <25, pnpm ≥12 <13
pnpm gen:api          # regenerate src/lib/api/schema.d.ts from ../openapi.yaml
pnpm dev              # vite dev server (CSP meta stripped for HMR only)
pnpm test             # vitest: engine, wire drift gate, token redaction, i18n, axe
pnpm check            # svelte-check, strict
pnpm lint             # eslint ({@html} is lint-banned; no-console banned)
pnpm build            # adapter-static SPA → build/ (strict CSP meta in place)
pnpm tauri build --no-bundle   # the Tauri core compiles (no signing, no bundle)
pnpm test:e2e         # Playwright: boots a REAL loopback kernel + drives the built shell
```

The e2e needs the kernel server binary: `cargo build --offline --locked --bin
brain-server` at the repo root (the e2e global setup builds it if absent).
The setup boots the SERVER (`target/debug/brain-server` — never the `brain`
client CLI) on a DEDICATED port **8799** with a temp data dir, refuses to
start if that port is already taken (a dev machine may run a live kernel on
the default 8765 — never test against a process you did not boot), declares
the preview origin in `CORS_ORIGINS` (the kernel's allowlist is explicit and
loopback-guarded), and stops the kernel after the run. The playwright web
server rebuilds the page with the e2e origin as the API base (build/ is
gitignored; the next plain `pnpm build` restores the default origin), and the
test context sets `bypassCSP` because the built page's strict CSP pins the
default origin only — the pin itself is asserted by the spec against
`build/index.html`.

## The typed wire (the no-guesses law)

`src/lib/api/schema.d.ts` is GENERATED from the kernel's `openapi.yaml`
(ONE contract, ONE source). A vitest test regenerates into a temp dir and
asserts **byte equality** with the committed file — contract drift = red
build. The typed client is `openapi-fetch` against that schema; the base
URL defaults to the pinned loopback origin `http://127.0.0.1:8765` and any
override is **loopback-enforced client-side** (non-loopback falls back to
the default).

## Security posture

- **Tauri least privilege (D6):** ONE capability (`capabilities/main.json`)
  holding `core:default` ONLY; NO fs/shell/http/process plugins installed;
  `withGlobalTauri: false`; the updater is ABSENT (M4). Strict CSP
  (`default-src 'self'`; `connect-src 'self' ipc: http://ipc.localhost
  http://127.0.0.1:8765`) in `tauri.conf.json`, and the equivalent
  `<meta http-equiv="Content-Security-Policy">` rides the static build in
  `app.html` (the dev server strips it for HMR; every build keeps it).
  No remote content is loadable; `{@html}` is lint-banned; `no-console` is
  lint-banned; `#![forbid(unsafe_code)]` on the Rust core.
- **The token (D7):** never in the bundle — the token module reads NOTHING
  from `import.meta.env` (VITE_* is compile-time-exposed and forbidden for
  this). In Tauri it arrives via the `kernel_token` command (the operator's
  own `BRAIN_SHELL_TOKEN` env, read at runtime by the Rust core). In the
  PWA build the operator's serving origin calls `setBearer` from its own
  bootstrap. Held in memory only; never logged, never rendered, never
  persisted (keyring = M2 deferral). The redaction test proves it.
- **The renderer (D1/D3):** branch-on-answer is CLIENT-SIDE over the
  kernel-validated packs; unknown/ambiguous answers hit the abstain card —
  nothing is invented, no free text exists in the schema. No intake
  submission this round (the webhook-seam POST is a later, separately
  preregistered round).
- **Telemetry (D8):** three preregistered reads (drop-off question id,
  time-to-first-action, error recurrence per question id) accumulate in a
  session buffer, carry IDS never text, and export ONLY inside the
  answers-JSON download. No kernel write surface exists.
- **i18n + a11y (D9):** en/de/fr/es/nl catalogs with a keys-parity red
  test; axe-core AA checks in vitest (the manual AA checklist port is M2's
  gate). All rendered pack text passes the invis-char/spoof sanitizer.
- **Supply chain:** committed lockfiles (pnpm + src-tauri Cargo.lock);
  `--frozen-lockfile` in CI; `pnpm audit --prod --audit-level high` as a
  gate; `cargo clippy -D warnings` + `cargo audit` on the core; postinstall
  scripts enumerated in `pnpm-workspace.yaml` with recorded reasons
  (`esbuild: allowed — binary wiring`; `es5-ext: denied — funding notice`).
- **Privacy by default:** saved progress is sessionStorage (dies with the
  tab); the export is the user's explicit download action; answers are
  typed values, never free text.

## The wizard packs

The three packs (`support-ticket`, `tele-health`, `capture-pre-screen`) are
operator-ratified DATA living in the kernel repo at
`crates/brain-fuzz/corpus/accounts/packs/`. The kernel embeds and
re-validates them; the route serves them; the shell's tests read the SAME
corpus files as fixtures — one source of truth everywhere.
