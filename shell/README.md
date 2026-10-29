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
  Support/com.stewardmem.shell/kernel` (0700); consent marker `~/Library/
  Application Support/com.stewardmem.shell/consent-v1.json` (0600, OUTSIDE the
  data root); `auth-token` and `brain-server.log` 0600; the kernel's
  `brain.db` is 0600. Consent Cancel returns before the data root is created.
- **Exit.** `RunEvent::Exit` kills only the child this process spawned; an
  attached kernel is not ours to stop.
- **A rotation reaches the running kernel.** The login keychain holds the
  shell's bearer; a RUNNING kernel reads its bearer from
  `<data-root>/auth-token`, and the kernel's `TokenStore` reloads that file
  when the path's mtime advances. So the Settings save writes the keychain
  **and** rotates that file — atomically (a 0600 temp file, fsynced, then
  renamed over the target, so a reader never sees a half-written token) —
  and it says what happened: the panel reports "the running kernel is using
  it now", or that the kernel has not started yet, or that the running
  kernel was **not** updated. It never reports "saved" for a rotation that
  did not reach the kernel.

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
| A mismatch/foreign/timeout is named and blocks the app | P1–P3, P11, the drill list below | green locally; GUI drill green |
| Consent refused ⇒ no data directory | P5 + P16 + the drill (Cancel pressed on the real screen) | green |
| Data is private: 0700 root, 0600 auth/DB | P6/P7 + the drill's `stat` | green |
| No orphan: the child is killed on exit | P10 + the drill (an orderly quit left no child) | green |
| Expected is the sealed provenance, not `package.json` | S1 + S4 | green |
| No shell plugin / no new HTTP client | S2/S3 | green (text scans; see the register) |
| A rotation reaches the RUNNING kernel | the drill (the panel wrote the keychain; the running kernel took the new token and rejected the old one — no restart) | green |
| The bundle is staged and verified | `stage` + `tauri build` + `verify` at this round's commit | green at the tip; see below |
| The GUI drill was performed on a real bundle | §6 manual, seven steps, read off the screen | **green — all seven steps ran** |
| The webview's wire can reach the kernel | the spawn's 8th key (`CORS_ORIGINS=tauri://localhost`) + the drill (the wizard loaded "Choose a form" on the FIRST read) | **green — the cold start now works** |
| The kernel allows THIS app's origin, and only it | the live kernel answers `tauri://localhost` with the allow-origin and a foreign origin with none | green |
| The first read carries the credential | the shared, memoized bootstrap the client awaits; the drill's cold start loaded the packs | green |
| A wire failure is NAMED, not guessed | the drill: *"The running kernel does not allow this app's origin…"* and *"The running kernel rejected this app's credential…"* | green |

**Verified 2026-10-20 (second pass):** `node shell/scripts/sidecar.mjs stage`
(`source_dirty: false`, kernel and `x-api-version` both `1.29.23`), then
`pnpm --dir shell tauri build --bundles app --target universal-apple-darwin`
(48.82 MiB) and
`node shell/scripts/sidecar.mjs verify "…/Brain Shell.app"` →
`ok — bundle seal, 2 Mach-O files, hardened runtime, arch parity [x86_64
arm64], provenance 1.29.23 @ f52d4405e7e0` (the build at the round's CODE
commit, which is where the S4 seal drill ran).

The sealer checks the record's `kernel_commit` against the TREE's HEAD, so a
bundle staged before the round's own commits is stale at the tip — the first
pass's bundle was, after its own three commits. The bundle is therefore
staged and built AGAIN at the branch's tip, after this section: `verify` is
green there too, and that build's record names the tip's own commit, because
a record names the commit it was built from.

**The GUI drill, performed (2026-10-20):** all seven steps ran on this real
bundle, driven through the accessibility API and read back off the screen.

1. With the data root and the consent marker removed, the consent screen
   appears: "Start the local kernel?", the local-only sentence, the 0700/0600
   law, the data folder named, Continue and Cancel.
2. **Cancel** → no data root, no marker, no kernel; the blocked screen says
   "The kernel was not started and no data folder was created."
3. **Continue** → the sidecar starts; `curl -i` shows `x-api-version: 1.29.23`
   and `{"status":"ok","version":"1.29.23"}`; the data root is 0700,
   `brain.db` and `auth-token` are 0600; the unauthenticated `/recall` is
   401; and the live kernel's environment is exactly the **eight** allowlisted
   keys (the eighth is `CORS_ORIGINS` — see the next section).
4. An orderly quit leaves **no child**; the relaunch produces exactly one
   kernel on the port.
5. A **9.9.9 stub** in the bundle → the refusal names **both versions**
   ("Expected version 1.29.23", "Reported version 9.9.9"), renders no app
   content, and our own spawned child is killed.
6. `SIGKILL` the sidecar → the relaunch has it back in **1 s**.
7. A **foreign listener** on 8765 → refused, **not stopped**, and no second
   kernel is spawned.

The rotation was measured in the same session: the Settings Save wrote the
login keychain (`security find-generic-password` read it back), the running
kernel's `<data-root>/auth-token` was rewritten, the kernel took the NEW token
and rejected the old one with a 401 — **no restart** — and the panel said "the
running kernel is using it now."

## The webview's wire can actually reach the kernel (III-4)

**The app could not talk to its own kernel.** Three defects, none of them
visible to a drill that exercised the kernel with `curl` (no `Origin`, no
bearer) — and none of them to the e2e, which compensates for exactly what the
app omitted. They were found by driving the **real bundle** and reading the
captured request off the wire.

1. **CORS.** The webview's request carries `Origin: tauri://localhost` (the
   captured request says so). The kernel's CORS layer exact-matches origins
   and its default allowlist is loopback dev-ports only, and the shell's spawn
   passed `env_clear()` + seven keys with **no `CORS_ORIGINS` among them** — so
   the kernel answered the webview without `access-control-allow-origin`, the
   browser engine blocked the response, and the app reported *"Could not reach
   the kernel"* for a kernel that was running and healthy. The e2e sets
   `CORS_ORIGINS` for its own origin (`e2e/global-setup.ts`), so CI never saw
   the real app's case. **Fix:** the spawn's **eighth** key is
   `CORS_ORIGINS`, carrying **this app's own origin and nothing else**. The
   kernel stays loopback-bound and auth-required; the origin decides only
   whether the *webview* may read a response, never who may call it.
2. **The credential race.** The layout kicks the credential read off
   (`void bootstrapToken()`), the keychain read is async, and the route's
   mount effect fired its **one** read before it resolved: the captured
   request was a plain `GET` with **no `Authorization` header and no
   preflight**, against a kernel that requires one, with nothing to retry it.
   **Fix:** the bootstrap is memoized and shared, and the typed client's
   request middleware awaits it — the credential is a *precondition* of the
   first read, not a race with it. Every route benefits; no route changed.
3. **The unnamed refusal.** The browser engine reports a blocked response, a
   rejected bearer and a dead port identically, so one message covered all
   three. **Fix:** the Rust core asks the kernel what the webview would
   experience (`kernel_wire_status`) and the renderer **names** the fault.

The origin fact is read from the **auth-exempt** `/health` and the credential
fact from an authenticated route, because the auth middleware sits **outside**
the CORS layer: a 401 carries no allow-origin, so reading the origin off a
401 would blame CORS for an authorization fault. That is measured, not
assumed.

**The drill (2026-10-20, the real bundle):** from a zero state the app asked
for consent; `Continue` started the kernel; and the wizard loaded **"Choose a
form"** — the packs arrived on the **first** read. The live kernel's
environment is exactly **eight** vars (nothing inherited) with
`CORS_ORIGINS=tauri://localhost`; it answers that origin with the
allow-origin and a foreign origin with none. The two refusals are named on the
real screen:

- a kernel that does not allow the origin → *"The running kernel does not
  allow this app's origin, so the browser blocked its answer. Start the
  kernel with CORS_ORIGINS naming the shell's origin."*
- a kernel that holds a different bearer → *"The running kernel rejected this
  app's credential. Save the current token in Settings."*

**A drill lesson, recorded because it cost the round a stuck screen:** a
**rebuilt** app's signature no longer matches the keychain item's ACL, so
macOS asks the operator to allow the access — and an unattended drill waits on
that dialog forever. The grant is the operator's call; the drill's own data
root is removed afterwards, and the operator's launchd kernel is restored
(`launchctl bootstrap`, verified `200 OK`, `x-api-version: 1.29.21`).

**Still owed, not claimed:** the first Ubuntu lane run with this lock, the
`bundle-macos` lane's first runner execution, and the `cargo metadata --no-deps`
conversion of the S2/S3 Cargo arms (they are text scans today).

## The updater — where the bytes come from, and whose they are (III-4)

**The endpoint is WHERE; the signature is WHO.** The manifest, the URL and the
announced version are untrusted input: they steer where the bytes come from
and nothing else. The minisign signature against the **one public key pinned
in the app's own configuration** is the only thing that decides whose bytes
they are. There is no path, anywhere, to change that key at runtime.

**The trust anchor** (`shell/src-tauri/tauri.conf.json`, `plugins.updater`):

- `pubkey` — the **literal base64 public key**, never a path. The CLI reads a
  `pubkey` that happens to be an existing file as that file's contents, while
  the plugin base64-decodes the same string at runtime, so a path would verify
  against a different key on the machine that built it than on the machine
  that runs it. The measured shape: `untrusted comment: minisign public key:
  C27C1E11196062D5` and a 42-byte packet beginning `Ed`.
- `endpoints` — **https only**. The plugin refuses a non-HTTPS endpoint in a
  release build, and no `dangerous*` flag is set: an updater that accepts
  invalid certificates is not an updater.
- `requireSignedVersion: true` — the endpoint's response is fetched over TLS
  but is **not itself signed**, so anyone who can serve it could pair an
  inflated version with an older release's artifact and signature. The signed
  version is what stops that, and it is free here: no release has ever been
  published.
- `allowDowngrades: false` — the key is the continuity, the version is the
  monotonic axis. A rollback is a **rebuild** of the previous code under a new
  version, signed by the same long-lived key — never a re-sign.

**Two version axes, kept apart.** The shell's own semver (`0.1.0` → `0.1.1`)
and the kernel's version, which travels in the manifest and never in the app
version. The update replaces the whole `.app` — **including the bundled kernel
sidecar** — so the manifest carries two extra fields: `kernel_version` (the
kernel the artifact carries) and `kernel_versions` (the kernels the release
vouches for). The gate refuses unless the manifest has both, the shell's own
**embedded** kernel version (read from the sealed provenance, never from
`package.json`) is among the vouched ones, and the offered kernel is too.

**The webview holds no updater authority.** The capability grants
`core:default` and nothing else; the Rust core owns the check and the
install, and the renderer only ever sees a DTO. There is no npm package, no
`tauri-plugin-process`, and no second transport (the plugin owns the HTTP).

**`createUpdaterArtifacts` is not in the tree's configuration.** It lives in
`shell/src-tauri/tauri.release.conf.json`, which only the operator's signing
step passes. The measured reason: the CLI's `sign_updaters` requires
`TAURI_SIGNING_PRIVATE_KEY` whenever the bundler's updater settings exist, so
a flag in the shared macOS overlay would make the **public** `bundle-macos`
job fail with "A public key has been found, but no private key" — and public
CI must not sign.

**The manifest writer** (`shell/scripts/updater.mjs`) writes the
`latest.json` a release publishes, and **refuses rather than guesses**: a
dirty tree, a staged record from another commit, a manifest that does not
answer for **both** Apple arches (the bundle is a universal build, so the
plugin looks the manifest up by the compile-time arch of the slice that is
running — a one-arch manifest serves half the machines and fails silently on
the other half), a release that vouches for no kernel version, a missing or
mismatched signature, and an announced version the artifact does not carry.
The last one is the anti-relabel law: the writer **reads** the artifact's own
`CFBundleShortVersionString`, because a re-signed old artifact would install,
report the old version, and offer itself the same update forever.

### The claims ledger (measured, 2026-10-20)

| Claim | How it is measured | Measured value |
|---|---|---|
| The lock delta of the one new edge | `git diff --stat f1ea6f63 -- shell/src-tauri/Cargo.lock` | **+540 / -18 lines, 49 new packages** (`tauri-plugin-updater` 2.14 + `tauri` with the `test` feature, a dev-dependency) |
| The new i18n keys, in five catalogs | the key count per catalog | **326 in each of `en/de/fr/es/nl`** (317 → 326: nine strings) |
| The manifest's arch key set | the writer's own output, read | **`darwin-aarch64` and `darwin-x86_64`**, both pointing at the one universal artifact |
| The pinned public key parses | P7's own output | **`untrusted comment: minisign public key: C27C1E11196062D5`**, a 42-byte packet beginning `Ed`, and it is not a path |
| The drill's steps, each with its evidence | the drill record | **six of the seven steps ran on the real bundle**; the seventh is recorded below |
| The bundle verified at this round's tip | `sidecar.mjs verify`'s own line | see the round note (the seal is re-staged at the tip) |
| The shell suite / `src-tauri` counts | the two test runs | **220 passed / 2 skipped** (was 212 / 2) and **58 passed** (was 54) |
| The red-proof texts | each mutant's observed failure | all twelve are in `docs/gate-law-register.md` |

### The drill (the real bundle, read off the screen through the accessibility API)

The drill's transport is **real HTTPS**: the updater's TLS uses the platform
trust store (reqwest 0.13 with `rustls-no-provider`), so the drill's CA was
generated in a temp dir and trusted in the **login keychain**. The app ran
from a temp directory (the plugin replaces the bundle in place), and every
line below is the text the accessibility API read off the screen.

1. **The app launches and the kernel gate still works.** The consent screen
   appeared — "Start the local kernel?", the local-only sentence, the
   0700/0600 law, the data folder named — and **Continue** started the
   bundled kernel (`x-api-version: 1.29.23`; the data root 0700, the
   database and the bearer 0600). The updater did not break III-3.
2. **A dead endpoint is its own state.** "Check for updates" against a
   refused port rendered **"The update endpoint could not be reached."** —
   never "you are up to date". The panel also stated the current facts:
   **"Running version 0.1.0 — bundled kernel 1.29.23."**
3. **The live endpoint, a manifest signed with the pinned key.** The panel
   rendered **"Update 0.1.1 is available (kernel 1.29.23)."** and offered
   the install.
4. **Install → the app replaces itself.** The bundle's
   `CFBundleShortVersionString` went **0.1.0 → 0.1.1**, the app relaunched
   under the new bundle (a new process), and the next check rendered
   **"You are up to date."** — the version axis, live: the app at 0.1.1 is
   not offered 0.1.1 again.
5. **A second key is refused.** The same release, signed with a **second
   keypair**, was offered and then refused on install: **"Refused: the
   artifact's signature is not the pinned key's"** — and the bundle's version
   was **unchanged**, because the refusal happens before the install.
6. **A release that does not know this app's kernel is refused, naming
   both.** **"Refused: unknown_local_kernel: expected 1.29.22, found
   1.29.23"** — the vouched kernel and this app's own, in one sentence.
7. **The rollback.** The pinned key signed a **second, later** artifact, and
   the app accepted **older code under a new version**: the pre-round tree
   (the commit before this round) was rebuilt as **0.1.5** and the running
   0.1.2 app installed it in **5 s** (`t=5s version=0.1.5`). The installed
   bundle's binary is **7,183,184 bytes** — the pre-round build's, against
   the tip's 10,044,160 — and the screen shows **no "Software update"
   section at all**, because the older code has no updater. Two independent
   observables, not a claim. The key is the continuity, the version is the
   monotonic axis, and the signature binds the two.

   **The road to it was measured, and it is worth keeping.** The pre-round
   tree has no `plugins.updater`, and the CLI **refuses**
   `createUpdaterArtifacts` without one ("failed to get updater configuration:
   plugins > updater doesn't exist"), so its artifact had to be tarred by
   hand — and **a hand-made tarball stalls the install**: the artifact
   downloads, the bundle is never replaced, and the app reports no outcome.
   The same stall was reproduced with the **tip's own code** re-tarred by
   hand, so the fault is in the hand-made archive and not in the older code;
   the bundler's own artifact installs in seconds. The pre-round tree was
   therefore given the same trust anchor (in the drill's copy only) so the
   **bundler** could make the artifact — which is also the shape a real
   release takes.

**The drill's machine was restored**: the drill's CA is out of the login
keychain (verified by the **output** — the count of `BEGIN CERTIFICATE` blocks
and the trust-settings references, because `security find-certificate` exits 0
even when it matches nothing), the drill's data root and consent marker are
gone, the operator's launchd kernel is back and healthy, and the
`BRAIN_SHELL_UPDATE_ENDPOINT` override is unset.

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
  generic-password item `com.stewardmem.shell/token`, account `default` ), then the
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
