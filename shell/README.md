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
pnpm install --frozen-lockfile # CI install; engines: node >=24 <27, pnpm >=12 <13 (CI Node 24)
pnpm gen:api          # intentional one-shot regeneration from ../openapi.yaml
pnpm dev              # vite dev server (CSP meta stripped for HMR only)
pnpm test             # vitest: engine, wire drift gate, token redaction, i18n, axe; never regenerates
pnpm check            # svelte-check, strict
pnpm lint             # eslint ({@html} is lint-banned; no-console banned)
pnpm build            # adapter-static SPA → build/ (strict CSP meta in place)
pnpm tauri build --no-bundle   # the Tauri core compiles (no signing, no bundle)
pnpm test:e2e         # Playwright: boots a REAL loopback kernel + drives the built shell
```

The e2e always builds the current kernel server binary: `cargo build --offline --locked --bin
brain-server` at the repo root.
The setup boots the SERVER (`target/debug/brain-server` — never the `brain`
client CLI) on a DEDICATED port **8799** with a temp data dir, refuses to
start if that port is already taken (a dev machine may run a live kernel on
the default 8765 — never test against a process you did not boot), declares
the preview origin in `CORS_ORIGINS` (the kernel's allowlist is explicit and
loopback-guarded), and stops the kernel after the run. The playwright web
server rebuilds the page with the e2e origin as the API base (build/ is
gitignored; the next plain `pnpm build` restores the default origin), and the
the test context sets `bypassCSP` because the built page's strict CSP pins the
default origin only — the pin itself is asserted by the spec against
`build/index.html`.

## The universal bundle and its kernel sidecar (III-1)

On macOS, `node shell/scripts/sidecar.mjs stage` MUST run before any
`cargo test`/`cargo clippy`/`cargo build` of `src-tauri` — the measured
failure otherwise is `resource path
'binaries/brain-server-aarch64-apple-darwin' doesn't exist`, because
`tauri.macos.conf.json` declares `externalBin` and `build.rs` resolves it at
compile time. Linux never reads the overlay, so the Ubuntu lane is unaffected
(the wiring deliberately does not live in the base `tauri.conf.json`).

`stage` builds the kernel for `aarch64-apple-darwin` and
`x86_64-apple-darwin`, lipos the universal binary, and writes
`src-tauri/binaries/SIDECAR_PROVENANCE.json` (commit, describe, version,
`x-api-version`, per-artifact sha256/size/archs). `src-tauri/binaries/` is
GENERATED and gitignored: the record names a commit and hashes bytes a fresh
clone does not have. `stage` refuses a dirty tree, because a build recorded
against a commit it was not built from is a false provenance claim.

```sh
node shell/scripts/sidecar.mjs stage
cd shell && pnpm tauri build --bundles app --target universal-apple-darwin
node shell/scripts/sidecar.mjs verify \
  "shell/src-tauri/target/universal-apple-darwin/release/bundle/macos/Brain Shell.app"
```

`verify` walks the bundle and FINDS every Mach-O by path (never by `file`
line count), checks the bundle seal, verifies each file with `--verify
--strict --ignore-resources`, requires the hardened-runtime flag, checks arch
parity and the sealed provenance record, and hashes the staged artifacts.
Exit 2 means it cannot verify; exit 1 names every failure. The sidecar is
ad-hoc signed (`signingIdentity: "-"`) with the hardened runtime ON; it has
no secure timestamp and `spctl` rejects it, which is the expected ad-hoc
result. A Developer ID signature, notarization, stapling and the clean-machine
run are the III-5 tail.

`pnpm gen:api` is the only command that intentionally rewrites the committed
`src/lib/api/schema.d.ts`. `pnpm test` is non-mutating; CI performs a temporary
regeneration and byte-compares the result. The kernel contract remains
`openapi.yaml`; the registry-lifecycle proposal wire gap is now closed by the
typed `{action,id,version,row_digest,row}` contract and the required
server-issued `row_digest` on the single-row detail response. M6-S2 remains
unbuilt.

## The kernel contract on the user's machine (III-3)

The macOS bundle carries a kernel sidecar, and the shell owns exactly three
edges around it: **detect**, **start**, **refuse**. The gate (`KernelGate`)
sits ABOVE the chrome: a refusal renders no nav and no app content.

- **Expected = the sealed provenance.** `x_api_version` is read from
  `Contents/Resources/binaries/SIDECAR_PROVENANCE.json` inside the bundle,
  never from `package.json` and never from the shell's own version. A missing
  or unparseable record is `MissingProvenance`, never "unknown → allow".
  The shell READS the sealed record; it does not re-derive it. The record's
  two version fields are sealed from two sources (the kernel's Cargo version
  and the wire's openapi version) and `sidecar.mjs stage` already refuses to
  seal them out of step, so a skew between them is reported as the version
  mismatch it is — both versions named — not as a missing record.
- **Detect is a raw loopback probe.** `GET /health` over `std::net::TcpStream`
  (300 ms connect/read), parsing the `x-api-version` response header and the
  JSON body. The body's `version` must equal the header. A 200 without the
  header is a foreign listener; it is never killed and never adopted.
- **Silence is not a wrong answer.** A port that is held but says NOTHING is
  `Unanswered`, and the two facts are kept apart. At preflight both refuse —
  a held port is never spawned over — but inside the bounded start only a
  wrong ANSWER ends the wait early. A kernel that has bound the port and is
  still booting says nothing, and a child that died on boot says nothing
  too: the first is waited for, the second becomes the named
  `StartTimeout` (with the exit status in `last`) instead of a false "that
  is not a Brain kernel".
- **Start is bounded in Rust.** Consent → token → auth file → spawn → wait
  at most 10 s (200 ms polls). The webview never owns the wait. A timeout
  kills the child this process spawned and names the binary, the data root,
  the wait and the last error.
- **Attach, never double-start.** A running kernel with the expected version
  is adopted — even when the bundled binary is absent, because adopting a
  running kernel does not need the bundle. A mismatch, a foreign listener or
  an inconsistent answer is a refusal: no spawn, no second port, no kill.
- **Env allowlist, no shell.** `env_clear()` + `BIND_HOST=127.0.0.1`,
  `BIND_PORT=8765`, `BRAIN_DATA_ROOT`, `BRAIN_DB_PATH`, `AUTH_TOKEN_FILE`,
  `BRAIN_REQUIRE_AUTH=1`, `RUST_LOG=warn`; no argv, no `sh -c`, no inherited
  operator credentials. The auth token is the shell's own (keychain →
  `BRAIN_SHELL_TOKEN`); it is never logged and never enters a DTO.
- **Paths and modes (macOS).** Data root `~/Library/Application
  Support/app.brain.shell/kernel` (0700); consent marker `~/Library/
  Application Support/app.brain.shell/consent-v1.json` (0600, OUTSIDE the
  data root); `auth-token` and `brain-server.log` 0600; the kernel's
  `brain.db` is 0600. Consent Cancel returns before the data root is created.
- **Exit.** `RunEvent::Exit` kills only the child this process spawned; an
  attached kernel is not ours to stop.

The gate's UI pins drive the state machine at the IPC seam: a refusal renders
no children; consent Cancel never calls `kernel_start`; Continue calls it once
with `consent: true`; a recorded consent starts without the dialog; a start
with no readable bearer refuses; the PWA bypasses without invoking Tauri.

| Claim | Evidence | Verdict |
|---|---|---|
| Detect reads the kernel's version surface | P1–P3 + the real-sidecar curl (`x-api-version: 1.29.23`, body `{"status":"ok","version":"1.29.23"}`) | green |
| Start is bounded | P4 + P8 + the 10 s/200 ms/300 ms constants | green |
| A booting kernel is waited for, not refused as foreign | P13 (a stub that answers after the probe timeout is adopted) | green |
| A held port is never spawned over | P14 (a silent holder refuses at preflight) | green |
| A matching kernel is attached, never respawned | P15 (attached even with the binary absent; a differing one refuses) | green |
| A refusal writes nothing | P5 + P16 (no data root, no consent marker, no child) | green |
| A mismatch/foreign/timeout is named and blocks the app | P1–P3, P11, the drill list below | green locally; GUI OWED |
| Consent refused ⇒ no data directory | P5 | green |
| Data is private: 0700 root, 0600 auth/DB | P6/P7 + the curl run's `stat` | green |
| No orphan: the child is killed on exit | P10 | green |
| Expected is the sealed provenance, not `package.json` | S1 + S4 | green |
| No shell plugin / no new HTTP client | S2/S3 | green (text scans; see the register) |
| The bundle is staged and verified | `stage` + `tauri build` + `verify` at this round's commit | green at the tip; see below |
| The GUI drill was performed on a real bundle | §6 manual, seven steps | **OWED unless run** |

**Verified 2026-10-20 (second pass):** `node shell/scripts/sidecar.mjs stage`
(`source_dirty: false`, kernel and `x-api-version` both `1.29.23`, the record
naming the commit it was built from), then
`pnpm --dir shell tauri build --bundles app --target universal-apple-darwin`
(48.82 MiB) and
`node shell/scripts/sidecar.mjs verify "…/Brain Shell.app"` →
`ok — bundle seal, 2 Mach-O files, hardened runtime, arch parity [x86_64
arm64], provenance 1.29.23 @ f52d4405e7e0`.

The sealer checks the record's `kernel_commit` against the TREE's HEAD, so a
bundle staged at an earlier commit is stale at the tip — the first pass's
bundle was, after its own three commits. The bundle is therefore re-staged and
rebuilt at this round's tip: a record names the commit it was built from, and
`verify` is green there.

**OWED (not claimed):** the manual GUI drill on a real bundle — consent
screen, Cancel leaves no data root, Continue starts, `curl -i` shows the
header, quit leaves no child, a `9.9.9` stub names both versions, SIGKILL
restart, and a foreign listener refusal — needs a GUI session and was not run
unattended. Also OWED: the Settings token rotation writes the keychain but
does not update a RUNNING sidecar's `<data-root>/auth-token`; the kernel's
mtime watcher is not fed until the app is restarted.

## CI and supply-chain boundary

Shell CI runs from a clean checkout on `shell/**`, `openapi.yaml`, the canonical
invisible-class fixture, and the workflow itself. It installs the Linux Tauri
prerequisites, the pinned stable Rust toolchain, root and shell Rust caches, and
the existing Hugging Face model prefetch; explicitly builds the root
`brain-server` binary before the dedicated-port e2e harness. The harness uses
only port 8799, one CI worker, and `pnpm exec playwright install --with-deps
chromium webkit`. The WebKit project remains `bypassCSP: false`; the live-wire
suite is not mocked.

The shell workflow grants `contents: read`, uses immutable action commits,
runs frozen install/lint/strict check/non-mutating Vitest/build/CSP and
production audit, checks generated-file drift, and runs Tauri fmt, clippy, a
fail-closed `cargo-audit` 0.22.2 install/audit, and a no-bundle Tauri build.
These are technical controls only; they make no legal, compliance, release, or
public-publication claim.

## The typed wire (the no-guesses law)

`src/lib/api/schema.d.ts` is GENERATED from the kernel's `openapi.yaml`
(ONE contract, ONE source). A vitest test regenerates into a temp dir and
asserts **byte equality** with the committed file — contract drift = red
build. The typed client is `openapi-fetch` against that schema; the base
URL defaults to the pinned loopback origin `http://127.0.0.1:8765` and any
override is **loopback-enforced client-side** (non-loopback falls back to
the default).

## Decision Explorer (M6-S1 / R32)

`/decisions` lists the kernel's bounded decision-run summary page and
`/decisions/[id]` shows a digest/reference-only trace projection. The list
client sends only `limit` (1–50, default 20) and optional `run_id`; mode,
model, outcome, date, escalation, cursor, and total-search filters are not
server-backed and are not simulated in the UI.

`src/lib/decision-run.ts` is the runtime boundary for the generated free-form
wire. It validates closed enums and finite numeric fields, bounds rows,
references, stages, labels, opaque previews, and the 1 MiB raw capture, and
passes displayed strings through the canonical invisible-character sanitizer.
The detail view offers exact-byte JSON download only inside that capture ceiling.
The stored trace does not contain the original query, evidence text, config,
or rules.

`src/lib/model-registry.ts` is the runtime boundary for the generated
model-registry wire, and `src/routes/models/+page.svelte` is its surface. The
parser proves closed enums (`kind`, `status`, output vocabulary), requires
digests to be EXACT lowercase 64-hex, epoch-guards every timestamp before any
`Date` conversion, and bounds every collection and string. The list carries the
artifact digest as a **presence boolean only**; the value is available on the
single-row read. `row_digest` is a content **pin**, not a signature and not
authentication, and the surface says so in those words. The only actions are
lifecycle **proposals** drawn from the kernel's own transition table — there is
no approve/reject control and no direct status write anywhere. Display text is
sanitized on the way to the DOM; outbound lifecycle bytes are the **verbatim**
wire values, and the proposal `row` is exactly the kernel's `RegistryRow` field
set with the pin travelling beside it, never inside it. An evaluation join is
data and can never move a status.

Replay is deliberately operator-supplied and transient: config, rules, and
input are never prefilled, reconstructed, stored, logged, or placed in a URL.
The stored mode is read-only. A replay response renders config/input/stage
agreement as data only; it never promotes, approves, or changes the trace.
Exploratory runs are visibly non-promotable. The dedicated-port e2e seeds a
synthetic candidate rules model and exploratory trace through the existing
public kernel routes, verifies the trace over the real wire, and runs with one
worker on port 8799; the WebKit project remains the static no-bypass CSP leg.

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
- **The token (D7/III-2):** never in the bundle — the token module reads NOTHING
  from `import.meta.env` (VITE_* is compile-time-exposed and forbidden for
  this). In Tauri it is supplied at runtime from the login keychain first (the
  generic-password item `app.brain.shell/token`, account `default`), then the
  shell's own `BRAIN_SHELL_TOKEN`. The kernel's operator credentials
  (`BRAIN_TOKEN_FILE`, `AUTH_TOKEN`, …) are NEVER read by the shell: that is
  the privilege boundary, not a naming preference. The Settings dialog writes
  and clears the keychain item through `store_kernel_token` /
  `clear_kernel_token`, and every keychain access runs on the blocking pool so
  a system prompt cannot freeze the UI. In the PWA build the operator's serving
  origin still calls `setBearer` from its own bootstrap, and the keychain
  section does not exist. The webview holds the token in memory only; never
  logged, never rendered, never in browser storage. The redaction tests prove
  it. Ad-hoc bundles (III-1) have no stable Team ID, so a keychain ACL prompt
  across rebuilds is expected rather than a failure; III-5's Developer ID is
  what makes the ACL stable. The pins drive the mock store; the app-level
  keychain drill is the external acceptance step.
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
  gate). All rendered pack text passes the canonical **invisible-character**
  sanitizer. That sanitizer **removes** canonical invisible code points and
  nothing else: it is **not** homoglyph or confusable detection, and it does not
  normalize word boundaries — stripping a zero-width space from `Acme models`
  yields `Acmemodels`, not `Acme models`. Confusable/spoof detection is a named
  follow-up, deliberately not claimed here.
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
