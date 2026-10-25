# Agent Execution Log — brain-server

> **R140 "Recut" (2026-10-25): the release lane was the FIRST machine to
> compile brain-afm from a clean tree, and it is red — because the bundle
> job stayed on an image whose SDK cannot see the macOS 27 FoundationModels
> surface.** The v1.29.32 tag push failed `build-app-macos` at the stage
> step: `LanguageModelCapabilities`, `SystemLanguageModel.variant/.capabilities`
> and `PrivateCloudComputeLanguageModel` do not exist in the 26.x SDK the
> `macos-latest` image's newest Xcode carries (measured off the run log:
> Xcode 26.6, SDK 26.5). `shell-macos.yml` already gates the Swift package on
> the `xcode-27` image; the bundle job — which now stages brain-afm beside
> the kernel — was left behind. The job now runs `xcode-27`, the same label,
> so the two lanes cannot drift onto different SDK generations. The
> last green `shell-macos.yml` run PREDATES the brain-afm target, so the
> re-run is brain-afm's first CI execution anywhere; watched, not assumed.
>
> **THE LOCAL-BUILD SHORTCUT WAS CONSIDERED AND REFUSED, and the reason is
> the lane's own contract.** Building brain-afm on the operator's machine and
> handing the binary to the lane would keep the queue short and the SDK
> modern — and would make the bundle job's green a statement about PACKING
> rather than about the tagged source. The bundle is the proof that the tag
> builds; a prebuilt artefact pasted in is the hand-made-archive class the
> updater round recorded, plus a supply-chain seam into the shipped app.
>
> **THE INSTALLER'S OWN OK WAS THE SECOND HALF OF THE BUG.** With the release
> lane red and the operator installing from main, `install-service.sh`
> printed `OK installed app ships kernel 1.29.31 (sealed provenance)` beside
> a 1.29.32 service — reading the mismatch aloud and calling it green, and
> the app's version gate then refused exactly as designed (expected 1.29.31,
> reported 1.29.32). The script now reads the STAGED bundle's seal BEFORE
> touching /Applications and refuses a stale bundle naming both versions and
> the re-stage command; a bundle without a seal is refused too. The drill
> ran on the REAL stale bundle that caused the screenshot — `staged app
> ships kernel 1.29.31 but this install runs kernel 1.29.32 — the app's
> version gate would refuse it at launch`, exit 1, /Applications untouched,
> service not restarted. **Owed, named:** a behavioral pin driving the
> refusal against fixture bundles (the installer has no test harness —
> recorded, not hidden).
>
> **AND THE RE-STAGE EXPOSED A THIRD DEFECT, the staging law's own twin.**
> brain-afm was staged ONLY as `brain-afm-universal-apple-darwin`, while the
> macOS overlay's `externalBin` needs the kernel's documented three-file
> shape per binary — the two thin slices each compile step resolves plus the
> universal the bundle copy takes — and the FIRST app-bundle build since
> that entry landed failed on it: `resource path
> binaries/brain-afm-aarch64-apple-darwin doesn't exist`. The law was
> already written in `sidecar.mjs`'s own header, measured when the KERNEL
> hit the identical failure; the bridge's staging simply didn't obey it. The
> thin slices are now THINNED out of the verified universal (`lipo -thin` —
> a copy would put fat bytes under a thin name), arch-checked, and sealed
> into the provenance record like every other artifact.
>
> **THE SECOND WAVE WAS THE SAME LESSON TWICE MORE, found by the round's own
> push.** The bundle job's move to `xcode-27` was GREEN — brain-afm's first
> CI compile — and the assemble job then refused at the green gate, which is
> where the tag's own double force-push had left a CANCELLED ci.yml run for
> the gate to read (the gate refusing a cancelled run is its law working;
> the cancelled run was this round's own mess). And the fix's push to main
> executed two more lanes that had never seen brain-afm: (1) `shell.yml`'s
> `bundle-macos` job — whose own comment said "FIRST RUN IS OWED: it
> executes on the next mirror push that carries a shell/** change" — ran and
> hit the IDENTICAL Xcode 26.6 wall; it now runs `xcode-27` like the release
> job. (2) `shell-macos.yml` (already on `xcode-27`) ran brain-afm's 28
> protocol pins on a machine WITHOUT the model, and the one red is gold:
> `changed instructions on a live session are refused` expected
> `instructions_locked` and got `model_unavailable` — the lock lived AFTER
> the budget measurement and inside the model-backed session, so on a
> model-less machine turn 1 died before the session existed and turn 2's
> lock had nothing to lock against. **A refusal whose order depends on the
> machine's posture is not a contract.** The registry now carries the
> session's identity (instructions + gate) from the id's first use; the
> lock is a registry decision decided with no model at all, and
> `materialize` is the only model-dependent step. The pin is red on the
> runner pre-fix (recorded in the shell-macos run), green post-fix 28/28
> locally — the lane's next runner execution is the proof.
>
> **AND THE ROUND'S OWN PROSE HIT TWO OF THE TREE'S LAWS, which is the
> process working.** (1) Inside a version section, `###` is RESERVED for the
> Release notes block: the lane-notes paragraph arrived as a `###` heading,
> the notes-gate pin embeds the real section into its fixture, and the plant
> therefore broke the shape it was testing — the refusal came out as
> "produced no bullets" where the pin demanded the ID-rule text. The
> paragraph is prose now, and the gate's three pins are green against the
> real CHANGELOG. (2) The shell lane's ci-gates pin hardcoded
> `runs-on: macos-latest` for the bundle job — a pin encoding the exact law
> this round fixed; it now demands the `xcode-27` label with the reason
> inline, so the next runner-label change is a named act too.
>
> **THE TAG MOVED, AND NOTHING COULD OBSERVE IT:** the failed run's
> `Assemble release + notes` job was skipped, so v1.29.32 had published
> NOTHING — the tag was force-moved to the fix commit and the lane re-ran on
> it, the green-gate re-verifying every publication workflow on the new SHA.

> **R139 "Grounded" (the chat spine, 2026-10-25): one chat turn became
> recall-as-evidence plus streamed on-device generation, and NOTHING is
> mocked anywhere in the verification chain.** The wire shipped first, per
> the substrates-before-consumers precedent: `POST /chat/turns` streams SSE
> `grounding` → `delta`\* → `final`/`error`, and the app's streaming-composer
> surface is Round 2, named below. The release that carries it is 1.29.32 —
> three rounds in one release (R-GOV's row-level governance integrity, the
> installer's app-shipping + version-matched health check, and this spine).
>
> **THE BRIDGE IS THE MODEL LAYER, UNREBUILT.** `shell-macos` grew a
> `brain-afm` executable (JSONL over stdio) reusing `ChatModelLayer` as-is —
> ChatRouter (never escalate turn 1; conversation+evidence vs the MEASURED
> budget verses measured at startup), the HistoryBudget watermark trimmer
> through `historyTransform`, and ChatTurnPlanner refusing BEFORE the
> request leaves. The framework has no seed-history API, so conversation
> continuity lives in the BRIDGE, keyed by the kernel's session id. **PCC is
> measured, not assumed, and the measurement is the refusal:** the window
> reads 32,768 while generation is REFUSED (`ModelManagerError` 1046 — the
> missing APP ENTITLEMENT, not hardware), so `.deep` stays nominal, `escalationPermitted`
> rides the probe (never `isAvailable`), and when an entitled build exists
> the same startup probe flips the 32K tier with zero code changes.
>
> **THE GROUNDING IS THE RECALL PATH'S OWN CONTRACT.** The route runs the
> SAME `run_recall` core as `/recall`, submodularly packed to 1,024 tokens,
> every hit `wrap_fenced` — recalled memory enters the model as DATA, never
> instructions. Cancel is structural: dropping the receiver sends `op:cancel`,
> the TURN dies, the SESSION survives. Locale is fail-closed against the
> model's MEASURED roster (the `de` retrieval-only precedent, now enforced at
> the model seam), and `chat_bridge_unavailable` is a POSTURE, not a crash —
> an absent binary answers by name and every other surface is unaffected.
> The bundle stages `brain-afm` beside `brain-server` (universal,
> arch-checked, sealed in the provenance record).
>
> **THE E2E IS REAL ON-DEVICE GENERATION** (`tests/chat_grounding_flow.rs`,
> the green-tea question over the composed app, nothing scripted): the
> grounding event carries the hit ids, the answer speaks in the memory's own
> terms (catechins/EGCG/Camellia), deltas precede the final, `tier:
> on_device`, zero error events. Beside it the Swift protocol pins drive the
> REAL binary (capabilities key-set EXACT, `instructions_locked`,
> turn-before-capabilities refused by name, malformed-line survival), and
> the kernel client pin proves the 4096/32,768 readings + the named
> refusal. The tree's own guards red on the way in and are green with their
> updates: dup-guard (the second `router()` joins the allowlist), the
> doc-truth gate (openapi + api.md rows), the router census (OPEN_GATES 203
> → 204, a named act), the authz matrix, the error taxonomy
> (`BridgeError`/`provider`/`display`), and the census re-derivation.
>
> **THE RELEASE LANE'S OWN FINDING (2026-10-25).** The updater endpoint
> `releases/latest/download/latest.json` has been 404 since v1.29.30 shipped
> without the chain — found by fetching the published URL from the open
> internet, not by any local gate. R136's law now runs in the lane itself:
> `release.sh` waits for the release, then stages → signs → publishes the
> chain → verifies the endpoint ANSWERS, all in one tool.
>
> **OWED, named in the register row, not hidden:** the composer streaming
> surface (Svelte + a Tauri streaming Channel — Round 2's opener); session
> persistence to `agent_session_events` + the listing endpoints (Round 2,
> paired with the sidebar); the Settings Advanced relocation and the plugin
> settings screen (Round 2 scope); the `LlmProvider` adapter for LoopDriver
> adoption (when tools join chat); the bundle's first staging EXECUTION of
> brain-afm (the code is staged; the tree was dirty at the cut, so the lane
> runs it on the first clean-tree stage); and the PCC `.deep` drill on an
> entitled machine — the one measurement this tree cannot make until Apple
> grants the entitlement.
>
> **ponytail:** one stdio protocol, one Swift executable, one serde client,
> one route; the bridge holds no keys, no tokens, no DB access, and no
> network egress except Apple's framework; no new dependency in the kernel,
> no env knob, no wire contract break (additive tables), no second test
> file (the e2e is one file), and no mocked model anywhere.
>

> **R138 "Governed" (R-GOV, 2026-10-25): the governance state of a memory
> row is now tamper-evident at row level — and on pre-round main, corrupting
> it broke NOTHING.** arXiv:2610.30698 (PREPRINT, 2026-10-22:
> "Governance-State Corruption in Long-Term Memory for LLM Agents") found
> the gap class: content-integrity does not cover governance-integrity.
> Measured on clean main (`78a18990`) before a line was written: a row
> inserted through `store_record`, then edited out-of-band with
> `SET access_scope='public'`, produced **audit delta 0, a still-matching
> `content_hash`, and no consumer anywhere that could see it** — the
> corruption's only visible effects were SERVING effects (any permit set
> containing `public` now admits the row; the role-less owner gate, which
> is `{private}∩own-sub`, actually LOCKS the owner out of her own row —
> corruption in both directions, invisible either way).
>
> **THE TUPLE AND THE HASH.** `knowledge.governance_hash` (v1.29.32
> migration, guarded ALTER + a RUST backfill in one tx — SQL never
> computes): SHA-256 over `(owner, access_scope, domain)` in a FIXED
> order, presence-marked + length-prefixed under a `brain-governance-v1`
> format tag, so NULL and "" are distinct and a future tuple EXTENSION is
> a tag bump, never a silent reinterpretation. `domain` is IN the tuple
> even though the content-hash dedup is domain-scoped, because the hash
> value covers content bytes only (measured) and moving a row between
> domains is a governance act. Content is excluded — `content_hash` owns
> it, and the isolation pin holds the boundary in BOTH directions.
>
> **TEN WRITE PATHS MAINTAINED, one pin and one guard to keep them
> honest.** The Phase 0 inventory (the pii guard's own scanner semantics
> over production regions): `store_record`, the KCS draft, the
> proposal-approve promotion, the procedure root+steps, `/add`, the
> `/ingest/memory` capture, the markdown import (insert + its post-commit
> domain move), and the admin `domain-move`. The relabel and the markdown
> domain move now recompute per-row in the SAME tx (read affected → hash
> in Rust → update domain+hash together). The structural sibling
> `every_knowledge_write_names_the_governance_hash` (the pii guard's
> scanner, extracted shared) holds every INSERT to naming the column and
> every governance-field UPDATE to recomputing in the same statement —
> born of the pii history: a column an INSERT omits is silently valid, and
> for this column the default is NULL, the `legacy_unhashed` class
> forever.
>
> **THE DETECTION SURFACE, and the read-path decision.** `brain
> verify-governance [--db PATH]`: counts `ok`, lists tampered ids, counts
> `legacy_unhashed`, exit ≠ 0 on tampering, and ERRORS when it cannot
> measure (measured on a pre-migration DB copy: `no such column:
> governance_hash`, exit 1 — never clean). Recall does NOT check per row
> (bounds/perf law): the scan is the detection surface, fail-closed lives
> in its reporting. Legacy rows are DISCLOSED with a count and never
> failed on — and the migration backfill necessarily LAUNDERS any
> tampering that predates it (the R108 legacy disclosure shape, stated in
> the migration, the CHANGELOG, and the register row).
>
> **FOUR DRILLS, RUN and REVERTED, both teeth on one plant:** (D1) the
> relabel recompute reverted → the pin reds `a domain move is a governance
> act: the hash must follow the NEW tuple, recomputed in the SAME
> transaction as the relabel` (stale-vs-recomputed hex pair) AND the
> structural guard names `domains_admin.rs:569`; (D2) the scan stubbed
> always-ok → pin 1 reds `left: [] right: [1]` while the probe drill
> prints `ok=1 tampered=[]` over a REALLY tampered row — the exact miss
> the scan exists to prevent; (D3) the backfill removed → `3 were
> waiting, left: 3 right: 0` (the known-mix count the prompt required);
> (D4) the pins grafted onto clean main → `E0425: cannot find function
> 'verify_governance'` — the red is the surface's absence, and the
> probe's recorded silence rides in pin 1's failure text so the red on
> main is behavioral evidence, not just a compile error. THE CLI DRILL
> RAN END TO END on the shipped binary: clean fixture → exit 0; one
> tampered row → `tampered: 1 (ids: [1])`, exit 1; pre-migration copy →
> fail-closed error, exit 1. (The operator's live DB was only read as a
> COPY.)
>
> **THE HONEST BOUNDARY, verbatim:** out-of-band tampering with these
> columns requires local DB access, which is the same attacker class as
> the stated wipe-everything ceiling. NOT a remote-attacker fix. The round
> makes the product's "governed" guarantee self-verifying at row level,
> the way the keyed audit chain makes the event log self-verifying —
> tamper-EVIDENT, not tamper-proof.
>
> **THE PROMPT'S OWN CLAIM WAS MEASURED FALSE, the R137 lesson applied.**
> The prompt said "the Oct 2026 sweep already carries the paper" —
> `docs/research/20-22` carried no `2610.30698` anywhere (measured); the
> cross-reference line was ADDED by this round to sweep 22, with the
> correction recorded there. Also corrected by measurement: the probe's
> gate arm — the expected "recall gate serves the row under the widened
> scope" holds only for permit sets containing `public` (role/unrestricted
> readers); the role-less gate is deny-by-default BOTH ways, so the same
> tamper can also silently LOCK AN OWNER OUT — the serving claim in the
> round's artifacts states the precise shape.
>
> **OWED, named:** the `/health/db` `governance_rows_tampered` wire field
> is DEFERRED (CLI-only this round — the field pulls openapi +
> route-coverage + authz guard-table + shell regen + an `x-api-version`
> bump, its own additive round); the structural guard's behavioral
> conversion for the three pre-convergence router paths (lands with their
> service-line move); a future tuple EXTENSION (lawful_basis/purpose/
> origin/pii are write-once-at-ingest with no update path today, so the
> drift class cannot arise for them yet) is a `brain-governance-v2` tag
> bump.
>
> **TWO STANDING GATES MOVED AS NAMED ACTS, and one round-law bit back
> correctly:** the router's frozen SQL census 49 → 52 (the markdown
> domain-move arm grew its own SELECT/UPDATE prepares — the hash recompute
> is in-tx by construction); the write-discipline pin flagged both new
> transactions as DEFERRED read-modify-write seams, and both were converted
> to `BEGIN IMMEDIATE` (the needle the discipline wants) rather than
> raising the frozen ceilings; the comments gate refused the round's own
> "v1.29.32"/"R108" labels in code comments (labels dropped, invariant
> sentences kept) — three gates that red on the round's own diff are the
> process working.
>
> **ponytail:** one column + one hash fn + one scan in the ingest core,
> ten maintained call sites, one CLI subcommand, five pins + one
> structural guard, one migration block; no new dependency (sha2 was
> already the audit module's), no trigger, no read-path change, no recall
> SQL change, no wire change, no second test file.

> **R137 "Lantern" (Phase III-8, 2026-10-22): the plugin's suite ran NOWHERE —
> and the integration is now proved against the binary that actually ships.**
> The OpenClaw plugin carried 1,768 lines of tests across seven files and
> nothing executed any of them: no workflow ran `plugin/test/**`, the
> plugin's `package.json` had no `test` script, and `npm install` in
> `plugin/` failed outright (`workspace:*` in devDependencies is an
> unresolvable URL outside the host's monorepo), so the tree could not fetch
> its own toolchain. Run by hand: five of seven files failed to LOAD, the two
> that loaded ran 70 tests, in a developer's terminal and in no lane at all.
>
> **THREE MEASUREMENTS DECIDED THE ROUND, and the first corrected the prompt.**
> (1) The claim that the invisible-set fixture "names a path that no longer
> exists" was FALSE — `plugin/src/format.test.ts` exists, is tracked, and was
> one of the two files that loaded. The real defect was DARKNESS: the
> fixture's consumer (2) ran nowhere, so its "changing any consumer without
> the server fails its lane" was a false claim. The prompt's §3 said to
> re-measure before coding if a failure was not what it claimed; the
> re-measure is what found this. (2) The install shape was chosen from FOUR
> measured candidates, not from the plausible one: removal loses the host
> monorepo's link; `optionalDependencies: "*"` resolves to the REGISTRY in a
> pnpm 12 workspace (measured with a synthetic three-package workspace:
> NOT linked) and costs every consumer a 404; `workspace:*` in an
> install-time field is published VERBATIM by npm — measured with `npm pack`
> — which would hand every CONSUMER the same EUNSUPPORTEDPROTOCOL the round
> exists to fix. The winner is the optional peer with the `workspace:*`
> specifier preserved: installs standalone (measured), keeps the monorepo
> link (measured: linked), and the published tarball installs (measured) —
> npm skips an optional peer. (3) The minisign format was measured against
> the `minisign` CLI's own verdicts, and the FIRST reading was wrong: the
> signature file's SECONDARY line decodes to the RAW 64-byte signature, with
> no algorithm marker and no key id — slicing it as a 74-byte blob truncates
> it, and five plausible payloads (trusted comment alone, comment then
> signature, and their hashed forms) all refused a perfectly good chain. The
> truth, from the Rust `minisign` crate the app itself uses: the primary
> signs the BLAKE2b-512 prehash, and the secondary signs the primary
> signature followed by the trusted comment.
>
> **D2 WAS DECIDED ON A FACT, AND THE TRADE IS RECORDED.** Shape (a) —
> injecting the entry helper into `index.ts` as a parameter — would need the
> HOST's call site to move: the plugin is loaded BY the host
> (`openclaw.extensions: ["./index.ts"]`), and the SDK is not published
> (`@openclaw/plugin-sdk` is 404 on npm — measured). So shape (b): the lane
> aliases that one specifier to a local stub, the shipped code does not move
> AT ALL, and the cost — the suite proves the plugin against a STUB of the
> host's helper — is named in the register row rather than blurred. The stub
> is deliberately STRICTER than the host is known to be (it validates the
> five fields the ambient `PluginEntry` declares): a stricter stand-in can
> false-red, a laxer one false-greens, and only one of those ships broken
> code.
>
> **GATE 9 IS CLOSED, AND THE OBSERVATION IS RECORDED.** The new drill
> (`plugin/scripts/integration-drill.mjs`) fetches the manifest the APP
> fetches — the endpoint from the app's own configuration, not a URL written
> beside it — downloads the artefact it names, verifies the minisign
> signature against the app's OWN pinned key (both halves), checks the
> binding and the announced kernel version, extracts the bundle, runs the
> kernel that ships INSIDE it, and drives the plugin's REAL `BrainClient`
> against that process. Exit 0 pass, 1 REFUSED, 2 COULD NOT RUN — never
> green when it could not measure. **The full drill ran, exit 0:** release
> `v1.29.29`, manifest version 0.1.1, pinned key `D5626019111E7CC2`, binding
> `file:Brain-Shell.app.tar.gz version:0.1.1`, kernel `brain-server 1.29.29`
> extracted from the verified bundle, `/health` answering `status ok` at
> `1.29.29`, the plugin's client STORING id 1 and RECALLING it (decision
> `ok`, 1 hit, `untrusted: true`), review queue read 0 rows. **A unit suite
> that mocks `fetch` proves the plugin's request shapes; only the shipped
> binary proves gate 9.**
>
> **ELEVEN DRILLS, every one RUN and REVERTED, and the observed red:** the
> `workspace:` devDependency restored → the vitest pin reds TWICE (the
> structural arm naming the field, and npm's own `EUNSUPPORTEDPROTOCOL`) and
> the Rust pin reds beside it; the lane alias removed → five arms red, the
> first naming the exact failure the round started from (`Cannot find
> package 'openclaw/plugin-sdk/plugin-entry'`); a stale path in the fixture's
> comment → the consumer-list arm; a named consumer moved aside → `a consumer
> that is never consulted is a dark tree, and the comment is a false claim`;
> the suite step excised → `the lane must run the plugin's suite`; the lane
> paths-filtered → `the plugin lane must not be paths-filtered — D1 says
> every push`; the include glob maimed → `src/brain-client.contract.spec.ts
> is no longer covered`; the drill's key check disabled → the pin reds naming
> the missing refusal; and on the REAL published chain, the genuine artefact
> renamed → `the signature names another file`, one byte changed → `the
> artefact's signature is not the pinned key's signature`.
>
> **AND A DRILL DISCIPLINE NOTE THAT COST TWO REVERTS, which is why it is
> recorded.** A guard that lives in a file the round EDITS cannot be reverted
> with `git checkout` — during the P2 drill it silently restored the HEAD
> manifest (the suite then failed with `Missing script: "test"`), and during
> the P4 drill it restored the HEAD fixture comment. Both were caught by
> READING the suite's red rather than trusting the revert. A round that edits
> tracked files must restore from the round's own content, not from git.
>
> **AND A PIN THAT REACHED FOR THE REGISTRY FLAKED ONCE IN FIVE RUNS, and the
> fix is the same lesson R135 recorded for the keychain.** The install pin
> drove `npm install --package-lock-only` against the live registry, and one
> run in five failed for a reason that had nothing to do with the tree. The
> pin now resolves `--offline` from the cache the lane's own `npm ci` has
> just warmed — measured: the LAW is intact, because npm refuses a
> `workspace:` URL before it looks at any cache, so the restored devDependency
> still reds with npm's own EUNSUPPORTEDPROTOCOL under `--offline` too. Five
> consecutive green runs after the change. A pin that shells out past the
> machine must be bounded, or it is a flake.
>
> **THE CENSUS AND THE BADGE MOVED, and both were re-derived rather than
> hand-typed:** `docs/test-census.md` re-derived (+3 structural — the new
> Rust pin), 3,521 → 3,524; the README badge re-derived 3470 → 3473 and
> `--verify-count` agrees. **The kernel, the shell, the client, the wire and
> the plugin's BEHAVIOUR are unchanged**: `src/` is untouched except the new
> test file, `shell/` and `client/` are untouched, and the plugin's
> production TypeScript is byte-identical (the only file it ships that moved
> is `package.json`'s dependency metadata).
>
> **OWED, named, not hidden:** the lane's first RUNNER execution (the same
> owed shape the bundle-macos lane carried until its first run — the workflow
> is written and its shape is pinned, and no runner has executed it); the
> full integration drill is an OPERATOR act per release (it downloads a
> 26 MB macOS bundle and runs a macOS app — the drill's refusal arms are what
> CI holds); the plugin's fidelity against the REAL host helper (the stub's
> residual, closed only by the drill); the fork host's consumer (4), which
> lives in another repository and can only be named; the approve action
> (needs its own authorization proof); and the Apple identity
> (revenue-gated).
>
> **AND THE RELEASE LANE'S AUDIT WAS RED ON THE TREE AS IT STOOD, found
> before the push rather than by it.** The `cargo audit` job on the mirror's
> `main` (`dc7a571b`) failed on `tokio 1.54.0` — a race condition in
> `tokio::time::Interval` (RUSTSEC-2026-0384), and a version the registry has
> since YANKED — carried by SIX of the seven TRACKED lockfiles: the root, the
> tauri shell, the steward harness, the channel bridge, the signal gateway and
> the client. **The fix is an update, not an ignore**: `cargo update` per
> workspace moved every lock to its current compatible versions (`tokio
> 1.54.1` among them, measured to be the registry's newest 1.x), and all
> seven then pass the EXACT command CI runs (`cargo audit --file <lock>`,
> exit 0 each). The remaining findings are the allowed unmaintained warnings
> the root `.cargo/audit.toml` already names — none is a vulnerability, and
> none was newly silenced.
>
> **THE SWEEP'S SECOND PASS FOUND AN EIGHTH LOCK, AND THE HONEST RECORD IS
> THAT IT IS NOT A CI SUBJECT.** `fuzz/Cargo.lock` is GITIGNORED
> (`fuzz/.gitignore:5`) — there are SEVEN tracked lockfiles, exactly the set
> the audit job scans — so a claim of "eight tracked lockfiles" would have
> been false; the fuzz workspace's lock was still refreshed (it now pins
> `rustls-webpki 0.103.16`, the post-advisory line, and audits clean), because
> a dev tool's stale lock is a local hazard even when it is not shippable.
> **And two held-by-choice dependencies left for maintained ones**: the
> signal gateway's `serde_yaml` (its `unsafe-libyaml` backend is
> RUSTSEC-2024-0449, unmaintained) moved to `serde_norway 0.9.42` once that
> fork replaced the unsafe backend (0.9.41+, measured on the index
> 2026-10-23) — one call site, API-compatible; and the plugin's `typebox`
> moved 1.3.33 → 1.4.3 with its lockfile. The shell's `pnpm-lock.yaml` was
> refreshed too, and still frozen-installs.
>
> **EVERY HALF WAS THEN VERIFIED BY THE COMMAND ITS OWN LANE RUNS**, because
> an audit fix that breaks a build has only moved the red: the root's
> clippy/test in default, bench and otel features plus all six feature lanes;
> the engine crates (including the harness-kernel arm); the steward harness;
> both tools with `--locked`; the tauri core; the client; the shell frontend
> (`svelte-check` 0 errors/0 warnings, `vitest` 240 passed); and the plugin
> (`npm ci` from the lock, `tsc --noEmit` clean, 232 tests).
>
> **And one flake was seen and is recorded, not claimed away:** the full
> `--features otel` run failed `capability_soft_forget_lane_is_unchanged`
> once, while the same test passed alone and passed inside its own file, and
> the full otel run then passed on the post-update tree. This round changed NO
> kernel code; the observation is recorded as a flake under load, and if it
> recurs it is a candidate for the bounded-treatment R135 recorded for the
> keychain pin.
>
> **ponytail:** one workflow, one Rust pin file, two vitest pin files, one
> stub, one vitest config, one drill script, one resolver hook, one ambient
> test surface, one manifest, one lockfile, one fixture comment, one census
> re-derivation, one badge and one dependency refresh (a security update, not
> a feature); no kernel change, no wire change, no shell
> change, no client change, no plugin behaviour change, no new runtime
> dependency (vitest and typescript are dev-only; typebox was already there),
> and no key in the tree.

> **R134 "Artefact" (Phase III-7, 2026-10-21): the release shipped NOTHING the
> updater could fetch, and the manifest pointed at a tag that could never
> exist.** The updater landed in `v1.29.26` and it can update from nothing: the
> published release carries 35 assets, 33 of them kernel binaries plus
> `SHA256SUMS` and `PLATFORMS.txt` — **no `.app`, no `.sig`, no `latest.json`,
> in ANY release** (measured: `gh release view v1.29.26 --json assets`). And the
> manifest the writer would have produced named `releases/download/v0.1.1/…` —
> the APP's version, not the release tag — a tag that does not exist and never
> will, because the release tags are the KERNEL's line (`v1.29.26`).
>
> **THE THREE DEFECTS, AND NONE OF THEM NEEDS THE APPLE CERTIFICATE.** (1) The
> URL named a tag that could not exist. (2) The signing path was unreachable —
> not because the key was missing (it was not) but because the tool was absent,
> the stored form was not the form a tool reads, the secret is passphrase-locked
> and no release published anything to sign. (3) The release published no
> artefact the updater could fetch. **The Apple Developer ID is DEFERRED to the
> first paying customer** (operator, 2026-10-21), so III-5 and the Dioxus
> decommission it gates are REVENUE-GATED and `client/` STAYS. The minisign
> updater key is NOT that key: it is OURS, free, self-generated, and it is the
> one the app's own updater verifies. **The round is therefore entirely about
> the self-signed chain, and the Apple half stays named as owed.**
>
> **THE KEYPAIR WAS NEVER RE-KEYED, AND THAT IS A MEASUREMENT, NOT A SHRUG.**
> The operator's `~/.config/brain-server/updater.key.pub` is BYTE-IDENTICAL to
> the key pinned in `shell/src-tauri/tauri.conf.json` — same keynum
> `C27C1E11196062D5`, same key id `d5626019111e7cc2`, same decoded payload. (An
> earlier draft of the plan compared the wrong halves — a minisign SECRET key
> does not embed the public key — and concluded the opposite. That correction
> is recorded, and it is the reason this round is a PROOF rather than a
> re-key: **re-keying would destroy a long-lived keynum to fix nothing.**)
>
> **D1 · THE TAG IS ITS OWN INPUT.** `shell/scripts/updater.mjs` now takes
> `--release-tag`, resolves it with `git rev-parse`, and REFUSES a tag the
> repository does not have — naming the tag AND the URL it refused. The
> manifest carries both facts (`version` = the app's `0.1.1`, `release_tag` =
> `v1.29.26`) and refuses a URL that does not name its own declared tag. The
> app's version and the release tag are two lines, and the URL fetches the
> RELEASE.
>
> **D2 · THE PAIRING IS PROVED, AND THE PROOF CANNOT BE SATISFIED BY A FIXTURE.**
> `scripts/release-app-sign.sh` compares the two PUBLIC halves — the pinned one
> and the operator's — and refuses a mismatch naming both keynums and both key
> ids. **The anti-vacuity arm is the pin**: a FRESHLY MINTED keypair (minted by
> the Tauri CLI itself, so it is a real second key) is driven through the
> signing lane and REFUSED. Every drill in III-4 minted a fresh pair and threw
> it away, which is why the shipped pair was never proved and why a future
> re-key would have been done for no reason.
>
> **D3 + D6 · THE CHAIN, AND WHERE THE KEY LIVES.** The artefact, its signature
> and the manifest travel together or not at all — one gate, one
> implementation, two callers (`--check-chain` and the publish path). The
> PUBLIC workflow builds the universal app bundle and walks every Mach-O in it,
> and it holds NO key and runs NO signing step; the SIGNING LANE signs what the
> public lane proved, on a machine that holds the key. **The artefact is
> built by the BUNDLER in the signing lane, because the updater artifacts
> cannot be built without the key — and a hand-made tarball stalls the
> install (III-4), so the packaging is never done by hand.**
>
> **D4 + D5 · SELF-SIGNED ONLY, AND AD-HOC IS STRUCTURE, NOT SHIPPABILITY.**
> The Apple identity is out of scope and named as owed. The workflow says
> WHERE the Developer ID signature would go — a separate step on the bundle's
> own signing, a different key with a different owner and a different clock.
>
> **FIVE DRILLS, all RUN and REVERTED, and the observed text is the proof.**
> P1 (the writer stops refusing a tag that does not resolve): `AssertionError:
> expected '' to contain 'v0.0.0-nope'` — the writer produced NO refusal at
> all; the pin's URL assertion is ordered FIRST so the drill names the URL
> rather than a bare status. P2 (the pairing check stops comparing the two
> public halves): `AssertionError: the refusal must name BOTH halves — the app
> pins C27C1E11196062D5, the offered key is 433CBD8FED7E6B94 — got: error: the
> working tree is dirty…`. P3 + P4 (the chain gate stops refusing a partial
> chain): TWO reds, `expected +0 not to be +0 // Object.is equality`. P5 (the
> public workflow gains a `minisign -S` step): TWO reds, `the public release
> workflow must never sign — it names minisign`. P6 (the updater stops
> verifying): `a manifest with no signature must be refused, got Ok(())` — and
> the STANDING III-4 pin red with it, which is the evidence that the tree's
> older teeth still bite.
>
> **TWO FINDINGS THE PINS MADE ON THE DAY THEY WERE WRITTEN, and the second
> is the kind that survives review.** (a) The URL template added its own `v` to
> a tag that already carried one, and the manifest pointed at
> `releases/download/vv0.0.0-nope/…` — caught by the pin's own URL assertion
> on the first run, before any drill. (b) The anti-vacuity arm was **VACUOUS**:
> the `.pub` FILE carries the BASE64 of the key text and the fixture read the
> keynum out of the base64, so it read `""` — and `"" !== shipped` passes for
> the wrong reason. The fixture now decodes, and the pin refuses an EMPTY
> keynum outright.
>
> **AND THE FIXTURE WAS UPGRADED, because it was a fixture of the SHAPE and
> not of the THING.** The writer's `.sig` fixture SYNTHESIZED the minisign
> text — and **nobody had ever run the writer against a signature the Tauri
> CLI actually produces.** It now MINTS a keypair with the CLI and signs the
> artefact with it, so the writer's binding parser reads the real trusted
> comment (`timestamp:…\tfile:…\tversion:…`). The III-4 drills were RE-RUN
> against the new fixture (a green pin after a refactor is not a passed drill)
> and all three red with the same texts the register already records. **And
> one drill was aimed at the WRONG LAW and stayed green** — the writer refuses
> a re-labelled version for TWO reasons, and III-4's own red-proof had already
> recorded that a pin asserting only "it refused" passes on the wrong one. The
> drill has to name WHICH law it turns off.
>
> **THE MEASUREMENT, ON THE CLEAN TREE, WITH THE SHIPPED KEY.** The signing
> lane was driven with the operator's real key and a deliberately wrong
> passphrase. It printed **`the signing key IS the key the app ships: keynum
> C27C1E11196062D5`**, loaded the key, and stopped at
> `incorrect updater private key password: Wrong password for that key` — the
> DECRYPTION failing, not the key being malformed. **So the key is SIGNABLE
> and the chain is reachable up to the passphrase: the reason the signed path
> is not live is "not signed", NOT "not signable", and the release notes say
> which.** The round trip itself (sign → verify against the PINNED public key)
> is the ONE thing the operator's passphrase gates — and it RAN: see below.
>
> **AND THE ROUND TRIP RAN (2026-10-22), AND THE CHAIN IS LIVE — and its
> FIRST PUBLICATION WAS BROKEN IN THE EXACT WAY THIS ROUND EXISTS TO CATCH.**
> The operator's signing lane built the updater artefact with the bundler,
> signed it with the SHIPPED key, and wrote the manifest; the chain was
> published to `v1.29.27` and the whole path was measured END TO END from
> the public internet: `latest.json` → **HTTP 200**; the artefact URL the
> manifest names → **HTTP 200, 26,370,074 bytes**; and **the bytes the
> updater would download verify against the application's OWN pinned key**
> (`Signature and comment signature verified`, trusted comment
> `file:Brain-Shell.app.tar.gz  version:0.1.1`). **THE PAIRING IS PROVED ON
> THE TWO PUBLIC HALVES, AND THE ROUND TRIP IS PROVED ON THE BYTES.**
>
> **BUT THE FIRST PUBLISHED URL 404'd, and the cause was the FILENAME.**
> GitHub replaces a SPACE in a release asset's name with a DOT: the artefact
> was published as `Brain.Shell.app.tar.gz` while the manifest's URL named
> the space (`Brain%20Shell.app.tar.gz`) → **HTTP 404 (9 bytes)**, so the
> updater would have downloaded NOTHING. **It is D1's defect one level down
> — a URL naming something that does not exist — and it was found only by
> FETCHING the published URL, not by any local check.** The fix is at both
> ends, and the ORDER matters: the writer REFUSES an artefact whose name a
> release asset cannot keep, and the signing lane RENAMES the artefact
> BEFORE it signs it, so the signature's `file:` binding names the artefact
> as it will be PUBLISHED. Renaming AFTER signing would have produced a
> manifest the writer refuses and a chain the app cannot verify.
>
> **OWED, named, not hidden:** the Developer ID, notarization, stapling,
> `spctl` and the clean machine (III-5, REVENUE-GATED — the Apple identity
> is a DIFFERENT key with a different owner and a different clock); the two
> owed register conversions (the updater's config arms' behavioural conversion
> and the `cargo metadata --no-deps` conversion of the S2/S3 Cargo arms);
> and the hand-made archive (upstream's to answer).
>
> **AND THE DE-SLOP WATCHDOG NOW COVERS THE WHOLE TAURI APP — and the
> ceiling this round named turned out to be MIS-DIAGNOSED, which is worth more
> than the fix.** The watchdog's `SCAN_PATHS` covered the server, the client,
> the shared crates and (from the previous round) the Tauri app's Rust CORE —
> and NOT the app's FRONTEND (`shell/src`, 93 tracked files) and NOT the Rust
> crate's TEST MODULES. **The reason the test modules were dark was
> lipstyk 0.3.0's `is_test_file`, and it is a FILE-NAME rule**: a `.rs` file
> whose stem is `tests`, `test`, or ends with `_test`/`_tests` is skipped
> WHOLE. **This round's own claim — "the shell's are exempt while the root's
> are scanned" — was WRONG, and the measurement said so**: a diagnostic
> planted inside the ROOT's inline `#[cfg(test)] mod` in `src/role.rs` IS
> caught (`src/role.rs:510 [warn] two-arm match could be simpler`), because
> the root's test code lives INSIDE a production file and the tool's
> attribute check never sees a test FILE. The Tauri crate's six test modules
> ARE files named `<name>_tests.rs`, and the whole file went dark. **So the
> fix is the NAME** — `<name>_pins.rs`, the repo's own word for a test that
> pins a property — with the `#[cfg(test)]` attribute still on the `mod`
> declaration in `lib.rs`, which is what actually marks them as tests, and a
> NEW PIN ARM (d) that refuses any tracked `.rs` file in that tree whose
> name the tool would skip. **Without that arm the next `foo_tests.rs` — the
> name Rust's own convention invites — would re-enter the scope with the whole
> file invisible, and nothing would say so.**
>
> **THE FRONTEND JOINED THE SCOPE TOO, and its first drill found a rule that
> cannot be scoped to changed lines.** Changing ONE UI component's import
> header made `cross-file-imports` fire on TWELVE files — ELEVEN of them
> UNCHANGED — which is the same class the config already disables for Rust
> (`cross-file-duplicate`). It is disabled with the same reason; the
> LINE-scoped rules stay armed, which is where a finding about NEW code
> matters. `ts-structural-repetition` was checked SEPARATELY and is
> diff-SAFE (one finding per group, at the group's first member), so it
> stays armed. **THE GENERATED i18n TREE IS SAFE TO HAVE IN SCOPE**:
> `shell/src/paraglide` is gitignored, so the gate's `git add -N` stages
> nothing and the diff only ever sees tracked, changed lines.
>
> **AND THREE PLANTS WERE WRONG PLANTS BEFORE ONE WAS RIGHT**, which is the
> lesson worth keeping: a two-arm match with a block-bodied arm does NOT fire
> `verbose-match`, and a `0 => ..` / `_ => ..` match does not either — only
> the `Some`/`None` shape does. **A drill that does not fire proves nothing
> about the DRILL**, and the natural reading of a green drill is "the gate
> is blind". Here it was the drill, twice, and once more in a different tree.
>
> **ponytail:** one script, one writer input, one workflow job, one test file,
> one Rust pin; no new dependency, no new crate, no kernel change, no plugin
> change, no contract change, no key in the tree, no public CI job that signs,
> and no re-key.

> **R135 "Deslop" (2026-10-22): the Tauri app is at ZERO slop — and almost
> none of what the analyser reported was ever slop.** The question was whether
> the desktop app carries much slop, and the honest answer was: the Rust core
> carried **six** findings (all one rule), the frontend carried **88**, and
> **86 of the 88 were not defects.** They were the analyser's blind spots, and
> each one was MEASURED rather than assumed:
>
> **THE SIX IN THE RUST CORE WERE REAL, and are fixed** — six two-arm `match`es
> rewritten as `map_or`, `unwrap_or_else`, `if let` and a three-arm match. Two
> of them could not be rewritten the way the tool suggests: in `updater.rs` both
> arms MOVE the DTO, so a `map_or_else` would not compile, and the comment in
> the code says so. The crate is now at **ZERO**.
>
> **THE 88 IN THE FRONTEND WERE 6 CLASSES OF NON-DEFECT.** Eighteen were
> documented fallbacks (`catch { return null }`, `catch { return 'unknown' }`,
> `catch { setBearer(null) }`) — the rule counts a catch that discards the error
> and cannot see that the catch IS the handling, and each one carries a comment
> saying why (sessionStorage is unavailable in private mode). Twenty-seven were
> TYPE GUARDS (`value is Record<string, unknown>` — the narrowing IS the value)
> and named seams beside their twins. Nine were icon-only buttons whose text is
> a **paraglide call the analyser cannot resolve** — and the app's real
> accessibility gate is `tests/route-a11y.test.ts`, **axe-core 4.14 over the
> RENDERED DOM**, where the i18n text IS resolved. Nine were route pages with
> no `<main>` — and **the AppShell already provides one**
> (`AppShell.svelte:342`); the rule analyses the page FILE in isolation, and
> **"fixing" it would have NESTED a `<main>` inside the shell's, which is
> invalid HTML.** Thirteen were `?` characters in TYPE positions
> (`agentId?: string`) counted as ternary operators. And six were the delay in
> an eval harness's POLL loop, between observable-condition checks.
>
> **SO THE DESLOP IS: fix the real ones, and disable the eleven rules that
> report the tree's deliberate posture — each with the MEASURED reason it
> cannot see, and each with ZERO real findings in the plugin tree.** Eleven
> disables, and every one of them has a Rust twin already disabled in the same
> config for the same reason (`error-swallowing`, `trivial-wrapper`,
> `structural-repetition`, `generic-naming`, `comment-clustering`,
> `restating-comment`). The plugin's two genuinely restating comments were
> REMOVED BY HAND rather than disabled.
>
> **"NO DEAD CODE" IS NOW A COMPILER LAW, and that is the only enforcement
> that cannot be forgotten.** The app carried **nine** unused declarations —
> one dead parameter in production code (`branchOf`'s `pack`, which six call
> sites were still passing) and eight unused imports and consts in the tests —
> and NO GATE COULD SEE THEM, because `noUnusedLocals` and
> `noUnusedParameters` were OFF. Both are ON now: an unused declaration is a
> compile error.
>
> **"MEMORY SAFE" WAS ALREADY TRUE, and it is enforced by the COMPILER, not
> by a test.** The Tauri crate carries `#![forbid(unsafe_code)]` in BOTH
> `lib.rs` and `main.rs` — so a single `unsafe` would fail the build — and
> clippy reports **zero** dead-code findings across the crate. There is nothing
> to add: the strongest guarantee available is the one already in place, and a
> pin behind it would be a weaker guard wearing a stronger name.
>
> **AND THE CHURN WAS HELD TO 161 INSERTIONS AND 56 DELETIONS ACROSS 22
> FILES**, with no behaviour change anywhere: the six Rust rewrites are
> semantics-preserving, the nine removals are removals, and the three
> comments are deletions.
>
> **AND THE PIN I ADDED FOR THE KEYCHAIN FLAKED ONCE, which is the finding.**
> One suite run in a series failed and the failure was not captured; nine
> subsequent runs — two of them under a concurrent cargo build — were green.
> The cause was mine: the pin read the login keychain with an UNBOUNDED
> `execFileSync`, and `security` can BLOCK on a keychain ACL prompt, so a
> machine with a locked keychain would fail the suite for a reason that has
> nothing to do with the tree. **The read is bounded now** (a three-second
> timeout, and a timeout means "this arm cannot run here"). **A pin that
> shells out to the user's keychain must be bounded, or it is a flake.**
>
> **ponytail:** six Rust rewrites, nine dead declarations removed and the
> compiler turned on to hold them, three comments deleted, eleven rules
> disabled with measured reasons; no behaviour change, no new dependency, no
> new knob, and 161 insertions across 22 files.

> **R136 "Endpoint" (2026-10-22): every release must now carry the update
> chain — and the finding was made by PUBLISHING, not by reading.** The
> `v1.29.28` cut published with its 35 assets and a green matrix, and the
> update manifest's address returned **"not found"** — while the `v1.29.27`
> chain was still verified end to end, byte for byte, from the public internet
> (`latest.json` → HTTP 200; the artefact → HTTP 200, 26,370,074 bytes; and
> those bytes verifying against the application's own pinned key). **THE CHAIN
> WAS CORRECT; THE RELEASE WAS INCOMPLETE.**
>
> **THE ENDPOINT IS THE LATEST RELEASE'S MANIFEST**
> (`releases/latest/download/latest.json`), so the moment a new release is
> published, the manifest the previous release carried stops being the one
> that is served. **A release that ships without the chain breaks the
> updater's path for EVERYONE.** The failure mode stays SAFE — the app
> reports the endpoint as unreachable rather than claiming to be up to date,
> which is exactly what R130 made the tree do — but the path is dead.
>
> **SO FROM HERE ON, EVERY RELEASE CARRIES THE CHAIN**: the updater artefact,
> its signature and the manifest, published together, with the artefact's name
> able to survive a release asset's name rewrite and the signature binding
> the artefact AS PUBLISHED (the rename-before-sign fix, which is what makes
> the URL resolve at all).
>
> **AND IT WAS PUBLISHED TO `v1.29.28`, and MEASURED FROM THE PUBLIC
> INTERNET**: the lane printed `renaming the artifact for publication: Brain
> Shell.app.tar.gz -> Brain-Shell.app.tar.gz` (the rename fix doing its work)
> and `the round trip holds: the artifact's signature verifies against the
> pinned key`; then `latest.json` → **HTTP 200** naming `release_tag:
> v1.29.28`, the artefact it names → **HTTP 200, 26,371,332 bytes**, and
> **those fetched bytes verifying against the application's OWN pinned key**
> (`Signature and comment signature verified`, trusted comment
> `file:Brain-Shell.app.tar.gz  version:0.1.1`). **THE SIGNED PATH IS LIVE
> FOR `v1.29.28`.**
>
> **AND THE OBLIGATION IS RECORDED WHERE IT WILL BE READ** — the register
> carries the row, the release notes carry the note, and the plans repo's
> ledger now says the same. **What is OWED is a guard that makes it
> IMPOSSIBLE TO MISS**: the release workflow cannot sign (it holds no key),
> and the chain lands AFTER the release is published, so the workflow's own
> fail-closed gate cannot require it. A script that must be remembered is a
> script that will be forgotten; the honest guard is a check on the RELEASE's
> published assets — "does this release carry a manifest?" — and it belongs
> beside the release's other gates.
>
> **ponytail:** one register row, one release-note section, one ledger line;
> no behaviour change, and the finding is the whole value.

> **R133 "Witness" (2026-10-21): the release helper said GREEN while both
> public runs were still running.** The v1.29.26 cut shipped, and the helper
> that watches the matrix told the operator *"all public runs for v1.29.26 are
> green"* **about twenty minutes before the matrix finished**. Nothing was
> mis-published — the ENFORCEMENT is `release.yml`'s own fail-closed check on
> the ci.yml verdict, not the helper's — but the helper is the operator's
> WITNESS, and a witness that cries wolf on success teaches you to ignore the
> one signal the release line has.
>
> **The hole was a loop that could only clear its flag for a run it SAW.**
> `DONE=1` before the poll, `[[ "$ST" != "completed" ]] && DONE=0` inside it,
> `[[ "$DONE" == 1 ]] && break` after. When `gh run list` returned NOTHING —
> a transient API failure, a rate limit, a query matching no rows — the loop
> body never ran, `DONE` stayed 1, and the script printed green. The same
> script's own header already records the lesson for the loop BESIDE it (it
> queried the tag OBJECT rather than the COMMIT and waited its full ten
> minutes on releases that had published): the same class of defect, in the
> other loop.
>
> **The fix is ONE implementation with two callers.** The verdict moved into
> `scripts/release-watch.sh`: `scripts/release.sh` (watching a real release)
> and `tests/release_witness_gate.rs` (driving it against fixtures). The law
> is one sentence — **no runs observed is UNKNOWN, and UNKNOWN is never
> GREEN** — and the only GREEN is every observed run completed with a
> `success` conclusion. A completed run with NO conclusion is not a success:
> fail closed. The helper no longer queries the runs' verdicts itself; that
> query moved into the witness, so the pin that drives it with fixtures and
> the helper that watches a real release cannot drift apart.
>
> **Two drills, both RUN and REVERTED, and the observed text:** (1) the
> vacuous pass planted back into the witness (an empty list answering
> `GREEN`) → `assertion left == right failed: an EMPTY run list is UNKNOWN,
> never GREEN. The witness returned Some(0) with stdout "GREEN" — the old
> witness read "no runs observed" as "all green", and it did so on the
> v1.29.26 release while both public runs were still in progress. left:
> Some(0) right: Some(2)`; (2) the OLD inline loop planted back into
> `scripts/release.sh` → `scripts/release.sh must no longer query the runs'
> verdicts itself (the .[] | select(...) | [.status, (.conclusion // "")] |
> @tsv query) — that query is where the empty list read as green, and it
> belongs to the witness now`.
>
> **And the fix was verified against the REAL `gh`, not only against
> fixtures:** the witness driven on the published v1.29.26 tag answers `GREEN`
> (exit 0), and the same witness driven on a tag with no runs answers
> `UNKNOWN` (exit 2) — the old witness would have said green.
>
> **THE RELEASE RECORD'S TWO OPEN DECISIONS ARE CLOSED, and the first is a
> security decision that is worth stating plainly.** (1) **v1.29.24 stays
> NOTES-ONLY — no binaries — and the reason is MEASURED, not a shrug.** Its
> tree pinned `rustls-webpki 0.103.15`, the release affected by
> RUSTSEC-2026-0382 (CVE-2026-111943), where a certificate path through an
> intermediate whose private key is now public could be accepted; the fix
> (`0.103.16`) landed in v1.29.25, which is the tag that carries the
> binaries. So a 1.29.24 build would have shipped that flaw, and nobody can
> install it from the release. **The release page now SAYS this** — a reader
> who lands on a release with zero assets reads a decision, not a failed
> publication. (2) The stale `## [Unreleased] - 1.29.24 (never published: its
> shell lane was red)` heading never existed in this tree, in any commit, or
> in the published body: the CHANGELOG carries `## [1.29.24] — 2026-10-20`
> with its 8 bullets and its compare link. **Nothing to decide and nothing to
> fix — the premise was stale, and the tree was already in the safe state.**
>
> **AND THE APPLE CERTIFICATE IS DEFERRED TO REVENUE, which puts the critical
> path's tail on the commercial clock (operator, 2026-10-21).** There is no
> Apple Developer ID certificate, and there will not be one until after the
> **first paying customer** — the Developer Program membership is a
> first-customer decision, and the operator's posture for now is **self-signed
> only, no Apple signing, no App Store**. So III-5 (the Developer ID signature,
> notarization, stapling, `spctl`, the clean machine) is **revenue-gated**, not
> "waiting to be bought", and everything it gates moves with it: Plan 20's P4
> gate 8, P5's gate 9, and the **Dioxus decommission itself** — so `client/`
> STAYS, because it is what ships today and the shell is not installable
> without a signed bundle.
>
> **THE TWO KEYS ARE NOT THE SAME KEY, and that is the trap.** The **Apple
> Developer ID** is Apple's, and it is deferred. The **minisign updater key** is
> **OURS** — free, self-generated, needing neither a purchase nor a customer —
> and it is the key the updater's manifest is signed with.
>
> **AND A CORRECTION, because this round's own first measurement was WRONG and
> the correction is the finding.** An earlier reading compared the wrong
> halves — a minisign **secret** key does not embed the public key, so
> comparing the secret's encrypted payload against the pinned public payload
> compares two unrelated byte strings — and concluded that **the pinned key had
> no private half on this machine**. It does. Measured properly:
> `~/.config/brain-server/updater.key.pub` is **byte-identical** to the public
> key pinned in `shell/src-tauri/tauri.conf.json` (same keynum
> `C27C1E11196062D5`, same payload), and `~/.config/brain-server/updater.key`
> is its **passphrase-protected** minisign secret key (`minisign -R` reaches the
> passphrase prompt, which proves the file is well-formed). **So the keypair is
> CORRECT and no re-key is needed** — what is missing is everything AROUND it:
> `minisign` was not installed (installed 2026-10-21), the stored key is
> double-base64'd so a tool cannot read that path (a correctly-formed 0600 copy
> now sits beside it), and **no release publishes an artefact, a signature or a
> manifest**. The tree's gates still cannot see the pairing — P6 scans the
> tracked tree, P7 checks the key's SHAPE, and every III-4 drill minted a fresh
> keypair — so **the shipped pair was never proved**, and a future re-key would
> have been done for no reason.
>
> **ponytail:** one script, one test file, six arms; no new dependency, no
> new crate, no new knob (the poll interval and the attempt count are
> ARGUMENTS, not seams), no change to what a release publishes, and no change
> to the release's fail-closed enforcement.

> **R132 "Queue" (Phase III-6, 2026-10-21): the review queue is DISPLAYED,
> and that is the whole of it.** The shell could not see the kernel's human
> review queue at all — the one surface the Dioxus app had and the shell did
> not, and the app that has it is being decommissioned. The shell now reads
> `GET /proposals` and shows what is waiting: the proposal's identity, its
> domain and owner, its four scores, the times, the injection screen's
> verdict, the text under review, and the `content_digest` (the reviewer's
> COPY HANDLE — the approve verb is digest-bound, so a display that showed the
> text without the digest could not support a decision made elsewhere with
> any confidence).
>
> **The law is DISPLAY-ONLY, and it is pinned as a shape.** No approve, no
> reject, no edit, and no code path that posts a decision: `P3` scans the
> route and EVERY module it imports for a decision path, for a write verb and
> for a decision symbol, and names the file when it finds one. The kernel's
> write verbs stay where they are, behind the `approve`/`reject` roles.
>
> **The offline cache holds METADATA, NEVER THE CONTENT, and the TYPE is the
> guard.** `QueueRowMetadata` declares `content?: never`, so a `QueueRow`
> (which carries `content: string`) is not assignable to it — the compiler
> refuses a row-with-content at the cache's own boundary. The reader is the
> second layer: it takes only the fields it knows, so a tampered entry cannot
> put the candidate text on the screen either. sessionStorage only, tab-local.
>
> **Four states that never share a rendering.** live, empty (the kernel said
> so), offline (named, with the age of what it last saw) and refused (with the
> reason). "Nothing is waiting" and "we could not ask" are different facts,
> and the surface keeps them apart.
>
> **The census's `renamed` verdict was FALSE, and it was TWO rows.** The
> shell's `/decisions` has always read `/workflow/decision-runs` — the
> decision-RUN listing, Admin-gated — while the parity table claimed it was
> the Dioxus `/review` (the proposal queue). `/decisions/[id]` claimed the
> Dioxus `/review/:proposal_id` (the single-PROPOSAL view) while the page
> reads the decision-RUN trace. The table's own doc used `/decisions` as its
> `renamed` EXEMPLAR. A false `renamed` reads as COVERAGE, and coverage that
> is false is worse than a gap — so both rows were CORRECTED (not extended
> around), the exemplar with them, and `P6` holds the correction. The new
> `/review` row is `exact`: the Dioxus path is the same.
>
> **The census now derives TWO module trees.** The queue's typed read lives in
> `shell/src/lib/queue/`, not under `#lib/api/`, so the derivation hops there
> too. The alternative — naming the module in `NON_ROUTE_CALL_SITES` — would
> have been a LIE: that slot's reason field is "why it is not
> route-reachable", and the module IS.
>
> **Seven drills, every one RUN and REVERTED, and the observed text:**
> P1 `expected '42' to contain 'aaaa…'` (the rendered digest replaced by the
> row id); P2(a) `expected '{"cachedAt":…' not to contain 'lead-safe'` AND the
> `content?: never` shape arm — TWO reds, the bytes and the type; P2(b)
> `Unable to find an element by: [data-testid="review-list-row"]` (the offline
> arm emptied); P3 `src/lib/queue/client.ts names a decision path` and, on the
> SAME plant, the census's closure arm `exports drillApproveProposal (→
> /proposals/{id}/approve), which NO route imports`; P4 `the render proof
> "review-refused" never appeared` — and the pin's wait bound was FIXED after
> the first drill reported only `Test timed out in 5000ms`, because a proof arm
> whose failure message is a stopwatch has not spoken; P5 THREE reds (the
> census's exact set, the sweep's coverage, P5 itself) when the route was
> added and the census row skipped; P6 `/decisions's row still claims the
> proposal queue`.
>
> **The kernel, the plugin, the wire contract and the dependency set are
> UNCHANGED** — the typed client is byte-identical to `openapi.yaml` after a
> regeneration, and the kernel's suite is untouched. The queue's own gates were
> already in place: Read scope, the `approve` role, owner-bound rows, and
> `MAX_PROPOSALS = 200`; the shell reads with the OPERATOR's bearer and the
> kernel decides what comes back.
>
> **OWED, named, not hidden:** the approve action — the Dioxus panel can
> approve and this surface cannot, so decommissioning the app costs the
> operator the in-app approve affordance unless a LATER round adds one, and
> that round needs its own authz proof (a webview posting a decision under a
> shared bearer is a different question from a display); III-5's cert tail
> (blocked on a purchase, not on code); the signing lane; and the two owed
> register conversions (the updater's config arms' behavioural conversion and
> the `cargo metadata --no-deps` conversion of the S2/S3 Cargo arms).
>
> **ponytail:** one route, two modules, one test file, one nav entry, one
> i18n key set × five catalogs; no new dependency, no new crate, no new
> capability, no `dangerous*` flag, no kernel change, no plugin change, no
> contract change, no approve path.

> **R131 "Updater" (Phase III-4, 2026-10-20): the endpoint is WHERE;
> the signature is WHO.** The shell can now update itself — and the whole
> round is the distance between those two facts. The manifest, the URL and
> the announced version are UNTRUSTED input: they steer where the bytes come
> from and nothing else. The minisign signature against the ONE public key
> pinned in the app's own configuration is the only thing that decides whose
> bytes they are, and there is **no path, anywhere**, to change that key at
> runtime. `shell/src-tauri/src/updater.rs` owns the gate (a pure `decide()`
> over the manifest's kernel facts, the local kernel read from the SEALED
> provenance and never from `package.json`), `shell/scripts/updater.mjs`
> writes the release's `latest.json` and **refuses rather than guesses**, and
> the webview is granted **no** updater permission at all.
>
> **Two decisions the measurement moved, and both are recorded rather than
> argued.** (1) `createUpdaterArtifacts` is **not** in the tree's
> configuration: it lives in `shell/src-tauri/tauri.release.conf.json`, which
> only the operator's signing step passes, because the CLI's `sign_updaters`
> requires `TAURI_SIGNING_PRIVATE_KEY` whenever the bundler's updater
> settings exist — so a flag in the shared macOS overlay would make the
> **public** `bundle-macos` job fail with "A public key has been found, but
> no private key", and public CI must not sign. (2) The updater's config
> lives in the **base** `tauri.conf.json`, not the macOS overlay, because
> registering the plugin **without** a `plugins.updater` entry makes the app
> FAIL TO START on that platform (the plugin's `Config` has a non-defaulted
> `pubkey`, and an absent entry deserializes as `null`) — and the base
> config is the one the Linux lane reads. The updater's facts are
> platform-neutral; the sidecar's are not, and they stay in the overlay.
>
> **Twelve pins, twelve red-proofs, every one RUN and REVERTED, and the
> failing text:** P1 the pinned key swapped for a second key's `an update
> signed by a key that is not the pinned key must be refused, got Ok(())`;
> P2 the gate stops refusing `a build whose kernel version the release does
> not know must be refused, got Allowed { offered_kernel: "1.29.23" }`; P3
> `a manifest without a kernel version must be refused, got Allowed {…}`;
> P4 one Apple arch `expected [ 'darwin-aarch64' ] to deeply equal [
> 'darwin-aarch64', 'darwin-x86_64' ]`; P5 a stale record `expected +0 to be
> 2`; P6 the real private key planted in a tracked file `carries the private
> key marker`; P7 a truncated pubkey `expected 'untrusted comment: minisign
> pu' to match …`; P8 `allowDowngrades: true` `expected true to be false`;
> P9 an `http://` endpoint `is not https`; P10 a second environment read
> `it reads ["env::var(\"BRAIN_SHELL_UPDATE_PUBKEY\")", "env::var(
> ENDPOINT_ENV)"]`; P11 `updater:default` `the webview must not hold the
> updater permission`; P13 the anti-relabel law `expected +0 to be 2`.
>
> **Two of the pins found their own weakness, and the drill found two
> defects the pins could not.** P13 asserted only that the writer REFUSED
> and that both versions appeared — and the writer refuses for TWO laws in
> that fixture, so the pin passed on the wrong refusal and the red-proof
> that turns the version law off left it GREEN. The drill, on the real
> bundle, found that the install **dropped its DTO** (a refusal during an
> install rendered as nothing at all) and that a signature which does not
> verify was reported as UNREACHABLE rather than as a refusal. All three are
> fixed, and the register records the fix, not the intention.
>
> **The drill ran on the real bundle, read off the screen through the
> accessibility API, over REAL HTTPS** (a CA generated in a temp dir and
> trusted in the login keychain — the updater's TLS uses the platform trust
> store, so the transport is real and no `dangerous*` flag is needed). All
> seven steps: the consent gate still works and the app starts its bundled
> kernel; a dead endpoint renders "The update endpoint could not be reached."
> and NEVER "you are up to date"; a manifest signed with the pinned key
> renders "Update 0.1.1 is available (kernel 1.29.23)."; the install
> REPLACED THE APP (0.1.0 → 0.1.1, a new process) and the next check
> rendered "You are up to date."; a manifest signed with a SECOND key was
> refused with "Refused: the artifact's signature is not the pinned key's"
> and the bundle's version was unchanged; a release that does not know this
> app's kernel was refused with "Refused: unknown_local_kernel: expected
> 1.29.22, found 1.29.23" — BOTH versions named; and the ROLLBACK: the
> pre-round tree rebuilt as 0.1.5, signed with the same pinned key, was
> installed in 5 s, and the installed binary is 7,183,184 bytes (the
> pre-round build's, against the tip's 10,044,160) with NO update section
> on screen — older code under a newer version, witnessed twice over.
>
> **The machine was restored, and the restore is part of the evidence:** the
> drill's CA is out of the login keychain (verified by the OUTPUT — the count
> of `BEGIN CERTIFICATE` blocks and the trust-settings references — because
> `security find-certificate` exits 0 even when it matches NOTHING, a
> measured trap from the prompt), the drill's data root and consent marker
> are gone, the operator's launchd kernel is back and healthy, and the
> endpoint override is unset.
>
> **OWED, named not hidden:** the lane that SIGNS (a self-hosted macOS
> runner, a private workflow, or a manual local release step — the public
> Actions must not hold the key, and III-4 does not invent a trusted lane);
> the real public endpoint (publishing is the operator's call); Developer ID,
> notarization, stapling and the clean-machine tail (III-5 — the updater
> does not change that: an unsigned update artifact installs, but the
> installed app is still the ad-hoc one); the behavioural conversion of the
> config arms (a pin that reads the BUILT bundle's `CFBundleShortVersionString`
> and holds it to the manifest's announced version); the first Ubuntu lane
> run with this lock and the `bundle-macos` lane's first runner execution
> (carried from III-3) — **both of the lane items in this list are CLOSED (2026-10-21) — the `bundle-macos` lane ran all thirteen steps green on the PUBLIC runner at the v1.29.26 release commit (`3b6bb8f0`), and the release's `Assemble release + notes` job ran all fourteen green on `ubuntu-latest`; the billing failures were the PRIVATE MIRROR only**;
> the `cargo metadata --no-deps` conversion of the
> S2/S3 Cargo arms; and the **hand-made archive** the drill measured: a
> tarball made outside the bundler STALLS the install (the artifact
> downloads, the bundle is never replaced, the app reports no outcome) —
> reproduced with the tip's own code re-tarred by hand, so it is the
> archive, not the code, and it is upstream's to answer.
>
> **ponytail:** one crate edge (`tauri-plugin-updater` 2.14) and one dev-edge
> (`tauri` with the `test` feature, which is empty); one module, one test
> module, one script, one test file; two commands, one DTO; nine strings ×
> five catalogs; no npm package, no second plugin, no `unsafe`, no new
> capability, no `dangerous*` flag, no key in the tree, and no lane that does
> not exist.

> **R127 "Kernel Contract" (Phase III-3, 2026-10-20): the shell's
> detect · start · refuse, pinned before it starts.** The macOS bundle now
> owns its kernel's lifecycle: `shell/src-tauri/src/sidecar.rs` reads the
> expected version from the sealed `SIDECAR_PROVENANCE.json` (never
> `package.json`), probes `GET /health` over a raw `TcpStream`, attaches to a
> matching kernel, and refuses a mismatch/foreign listener/inconsistent
> answer/missing sidecar/provenance/keychain/insecure auth file. Only
> `NotRunning` reaches a spawn; only the child this process created is killed
> on `RunEvent::Exit`. `KernelGate` sits above the chrome, so a refusal
> renders no nav and no app content; the PWA still bypasses. The spawn is
> `current_exe().parent()/brain-server` with `env_clear()` + seven keys; the
> wait is 10 s in Rust (200 ms polls, 300 ms probes); consent is a 0600 marker
> OUTSIDE the 0700 data root; `auth-token` and the log are 0600.
>
> **Twelve drills run, every one reverted; the failing text:**
> P1 expected→observed `left: Mismatch { expected: "9.9.9", observed:
> "9.9.9" }`; P2 parser trusts the body `left: Probe(Probe { api_version:
> "1.0.0", body_version: "2.0.0" })`; P3 accepts a headerless 200 `left:
> Probe(...) right: Foreign`; P4 deadline removed **HUNG** and was killed
> after 12 s (no text — the hang is the red); P5 create above the gate `a
> refusal created the data root`; P6 chmod removed `the data root is not
> owner-only` (0755); P7 silently chmodding `a 0644 auth file must refuse,
> got Ok(...)`; P8 `BIND_HOST=0.0.0.0` `left: Some("0.0.0.0") right:
> Some("127.0.0.1")`; P9 wrong subdir `.../Contents/MacOS/Resources/
> brain-server`; P10 kill dropped `shutdown waited 5.005256875s; the child
> was not killed`; P11 children on refusal `expected <p
> data-testid="host-content"></p> to be null`; P12a start-on-mount never
> rendered the consent dialog (Cancel had no button); P12b `consent: false`
> `expected { consent: false } to deeply equal { consent: true }`.
> S1 package.json fallback `sidecar.rs must not read package.json`; S2 a
> comment naming the plugin `the shell plugin must not be a dependency`;
> S3 a comment naming reqwest `an HTTP client edge was added: reqwest`; S4
> hand-edited version `provenance kernel_version: record 9.9.9 != Cargo.toml
> 1.29.23`.
>
> **Two findings worth keeping.** (1) `let state = $state(...)` is a
> svelte-check RED: svelte2tsx reads `$state` as the store subscription of a
> variable named `state` (`Block-scoped variable '$state' used before its
> declaration`), so the gate's variable is `gate`; the component still uses
> runes normally. (2) S2/S3 are TEXT scans — a comment naming `reqwest` turns
> them red. A real `shell:default` grant is refused EARLIER by the Tauri
> build script (`Permission shell:default not found`), a stronger witness;
> the `cargo metadata --no-deps` conversion is owed in the register, not
> claimed. P10's first pin only asserted "no process", which a bare `wait()`
> satisfies after the sleeper exits; it now bounds the call at 2 s.
>
> **OWED, not green:** the seven-step GUI drill (consent/Cancel/Continue,
> curl, quit leaves no child, 9.9.9 stub, SIGKILL restart, foreign listener)
> needs a GUI session and was not run unattended. The Settings rotation
> writes the keychain but does not update a RUNNING sidecar's
> `<data-root>/auth-token`; the kernel's mtime watcher is not fed until
> restart. III-2's app-level keychain drill remains OWED. The bundle stage ran
> at `2ae4462d` (`source_dirty: false`, `1.29.23`); the staged aarch64 binary
> answered a real curl with `x-api-version: 1.29.23` and body
> `{"status":"ok","version":"1.29.23"}`, and the 0700/0600 modes were
> measured. The plan's `verify --bundle <path>` spelling is wrong — the
> script's real form is `verify <bundle> [--staged <dir>]`; the bundle built
> (48.82 MiB) and verified: `ok — bundle seal, 2 Mach-O files, hardened
> runtime, arch parity [x86_64 arm64], provenance 1.29.23 @ 2ae4462dceb8`.
>
> **ponytail:** one Rust module, one test module, one component, one UI test
> file; no new dependency, no new crate, no second transport, no shell
> plugin, no unsafe; the 31 strings ride the existing five catalogs and the
> parity pin; the register row is the deliverable, not a framework.

> **R128 "Second Pass" (Phase III-3b, 2026-10-20): the round's own diff,
> read against the plan, and it did not survive the reading.** R127 shipped
> the kernel contract with twelve pins, four structural locks, sixteen
> red-proofs and a green bundle. This round is the adversarial lane over
> R127's own diff — the same job R121 did to R115–R120 — and it found
> three defects, all in the Rust contract, none of them in the plan's
> wording.
>
> **1. Silence was reported as "that is not a Brain kernel."** The probe
> returned `Foreign` for three different facts: a listener that answered
> wrong, a listener that answered NOTHING, and a peer that reset the
> connection. The plan defines `Foreign` by the shape of the ANSWER
> ("non-200 / no header / non-Brain body") — silence is not in that list.
> Two consequences, both real: a kernel that binds the port BEFORE it can
> answer a cold start says nothing for longer than the 300 ms probe, so it
> was refused as a foreign listener **and the child we had just spawned was
> killed**; and a child that DIED on boot said nothing either, so its death
> was reported to the user as "the listener on 127.0.0.1:8765 is not a
> Brain kernel; it was not stopped" — false on both counts, because it was
> our child and we did stop it. Silence is now `Unanswered`: it refuses at
> preflight (a held port is never spawned over — same shape, same
> fail-closed rule) and is WAITED FOR inside the bounded start, which turns
> a crash into the plan's own `StartTimeout { waited_ms, sidecar, data_root,
> last }` with the exit status in `last`.
>
> **2. The shell re-derived the seal, and got it wrong.** `load_provenance`
> refused a record whose `kernel_version` and `x_api_version` disagreed.
> But `sidecar.mjs` seals those from TWO sources — the kernel's Cargo
> version and the wire's openapi version — and `stage` already refuses to
> produce a record with the two out of step ("version lockstep broken").
> So the shell's rule could only ever fire on a record the sealer would
> never write, and what it said was false: "the bundled kernel's provenance
> record is missing or unreadable" for a file that was present, readable
> and well formed. Worse, it swallowed the better answer: a skew between
> the two versions is a VERSION MISMATCH, and the contract names both
> versions on screen. The shell now reads the sealed record and does not
> re-derive it (§4.9), and the skew is a named mismatch again.
>
> **3. Two of the round's own laws had no pin.** D8's "attach, never
> double-start" — the behaviour the whole gate exists for — was unpinned:
> `Preflight::Attach` had no test at all, so nothing stopped a future edit
> from turning an attach into a second spawn. And "a refusal creates no
> data root" was true for `consent_gate` (P5) but only by inspection for
> the start path: the ordering that reads the manifest before anything is
> written lived inside a function that needed an `AppHandle` no pin could
> build. `start()` is now split into the Tauri edge and
> `start_with_paths()`, and the start path is drivable with temp
> directories.
>
> **Four pins land, and each one was red-proved:**
> `a_kernel_that_answers_after_the_probe_timeout_is_waited_for` (the late
> answer is waited for; a wrong answer still ends the wait at once),
> `a_silent_listener_is_refused_before_any_start` (a held port is never
> spawned over), `a_matching_kernel_is_attached_and_never_respawned` (a
> matching kernel is adopted even with the bundled binary ABSENT; a
> differing one is refused), `a_refused_start_creates_no_data_root_and_
> spawns_nothing`. S1's malformed-shape list also lost the skewed pair,
> because that shape is not malformed — and it gained the arm that says so:
> a skewed record parses, the expectation is the `x_api_version`, and the
> decision is a `Mismatch` naming both.
>
> **All twenty drills re-run against the final refactor, every one red,
> every one reverted** (the plan's §8: a green pin after a refactor is not a
> passed drill). The failing texts: P1 `left: Ready right: Mismatch {
> expected: "1.29.23", observed: "9.9.9" }`; P2 `left: Probe(Probe {
> api_version: "1.0.0", body_version: "2.0.0" })`; P3 `left: Probe(Probe
> { api_version: "1.0.0", body_version: "1.0.0" }) right: Foreign`; P4 the
> pin **HUNG** and was killed after 25 s (no text — the hang is the red);
> P5 `a refusal created the data root`; P6 `left: 493 right: 448` (0755
> against 0700); P7 `a 0644 auth file must refuse, got Ok(...)`; P8 `left:
> Some("0.0.0.0") right: Some("127.0.0.1")`; P9 `left:
> ".../Contents/MacOS/Resources/brain-server" right:
> ".../Contents/MacOS/brain-server"`; P10 `shutdown waited 5.005302083s;
> the child was not killed`; P11 `expected <p data-testid="host-content">
> </p> to be null`; P12a `TestingLibraryElementError: Unable to find
> role="dialog"`; P12b `expected { consent: false } to deeply equal {
> consent: true }`; S1 `sidecar.rs must not read package.json`; S2 `the
> shell plugin must not be a dependency`; S3 `an HTTP client edge was added:
> reqwest`; S4 `FAIL provenance kernel_version: record 9.9.9 != Cargo.toml
> 1.29.23` (the bundle's own copy of the record — and the seal caught the
> tampered resource too: "a sealed resource is missing or invalid"); P13 `a
> late answer must be waited for, got Refused(ForeignListener { port: 8765
> })`; P14 `a held port must refuse, got Start`; P15 `a matching kernel must
> be attached, got Start`; P16 `a refusal created the data root`.
>
> **The fourth finding is about the bundle, and it is the one that would
> have shipped a stale seal.** The sealer checks the record's
> `kernel_commit` against the TREE's HEAD, so R127's own three commits made
> its staged record stale: `verify` at the tip reported a commit skew, and
> the shell binary inside the bundle was R127's, not R128's. The bundle was
> re-staged, rebuilt and re-verified at R128's code commit (`ok — bundle
> seal, 2 Mach-O files, hardened runtime, arch parity [x86_64 arm64],
> provenance 1.29.23 @ f52d4405e7e0`), and then staged and built AGAIN after
> this note, so the tip's own `verify` is green too. A record names the
> commit it was built from — that is why the seal is re-staged at the end
> and not merely once.
>
> **OWED, not green, and unchanged by this round:** the seven-step GUI
> drill (consent/Cancel/Continue, curl, quit leaves no child, 9.9.9 stub,
> SIGKILL restart, foreign listener) needs a GUI session and was not run
> unattended; the Settings rotation still writes the keychain without
> updating a RUNNING sidecar's `<data-root>/auth-token` (the kernel's mtime
> watcher is fed only at restart); III-2's app-level keychain drill remains
> owed. One thing this round DID close: the round's claim that the bundle
> was staged and verified is now true AT THE TIP, not at a commit the round
> has since moved past.
>
> **ponytail:** one new enum variant, one function split, four pins, one
> arm rewritten; no new dependency, no new crate, no second transport, no
> new knob, no new file, no UI change (the new refusal keeps the kinds the
> gate already renders); the i18n catalogs are untouched, because the
> contract's new fact is a distinction BETWEEN two refusals the gate
> already had.

> **R129 "Rotation" (Phase III-3c, 2026-10-20): the two OWED items are
> closed, and one of them closes another round's OWED with it.** R127 left
> two: plan §4.7's Settings sync (the keychain held the token; the RUNNING
> kernel read its bearer from `<data-root>/auth-token`, so a rotation left
> the kernel on the old token until a restart) and the seven-step GUI drill.
> III-2's app-level keychain acceptance was a third, borrowed from R127.
>
> **The rotation.** `store_kernel_token` now writes the keychain AND
> rotates the kernel's token file, and it says what happened: the outcome
> (rotated / no data root yet / failed) reaches the Settings panel, which
> no longer reports "saved" for a rotation that did not reach the kernel.
> The write is **atomic** — a 0600 temp file, fsynced, then **renamed**
> over the target — for two reasons the kernel's own code decides: its
> `auth::TokenStore` watches the PATH's mtime, so the rename is seen
> (and a rotation is live **without** a restart, which is stronger than
> §4.7 assumed), and because a rename is atomic, a reader can never
> observe a half-written token, which an in-place truncate would. The
> mode law holds before and after; a rotation before the consent gate
> creates nothing. Six pins: `a_rotation_writes_the_running_kernels_auth_
> file`, `a_rotation_before_consent_creates_nothing`, `a_rotation_refuses_
> a_world_readable_auth_file`, `a_rotation_replaces_the_token_file_
> atomically`, `the_settings_reports_that_the_running_kernel_took_the_
> token`, `a_rotation_that_failed_is_not_reported_as_saved`. All six
> red-proved: P17 `the rotation must write the kernel's token file, got
> Ok(NoDataRoot)`; P18 `a rotation created the data root`; P19 `a 0644 auth
> file must refuse the rotation, got Ok(Rotated { path: … })`; P20 `the
> token file was rewritten in place; a reader could see a partial token`
> (`left: 243289144 right: 243289144` — the same inode, which is the whole
> point); P21 `Unable to find an element with the text: Token saved to the
> login keychain, and the running kernel is using it now.`; P22 `Unable
> to find an element with the text: /the running kernel was not
> updated/.`
>
> **The GUI drill, and it is no longer OWED.** The user's Accessibility
> grant made the screen readable and clickable, so all seven steps ran on
> the REAL bundle with the real kernel — the evidence is the screen and
> the filesystem, not a screenshot (this machine's `screencapture` is
> denied; the accessibility tree is better evidence anyway, because it is
> the text the user actually saw). **(1)** With the data root and the
> marker removed, the consent screen appears: heading "Start the local
> kernel?", the local-only sentence, the 0700/0600 law, the data folder
> named, Continue and Cancel. **(2)** Cancel pressed → **no data root, no
> marker, no kernel**; the blocked screen says "The kernel was not
> started and no data folder was created." **(3)** Continue pressed → the
> sidecar starts; `curl -i` shows `x-api-version: 1.29.23` and the body
> `{"status":"ok","version":"1.29.23"}` (they agree); the data root is
> 0700, `brain.db` and `auth-token` 0600, the marker 0600; the unauth
> `/recall` is 401; and the live kernel's environment is EXACTLY the seven
> allowlisted keys and nothing else. **(4)** An orderly quit leaves **no
> child** and frees the port; the relaunch produces exactly one kernel on
> the port — no duplicate. **(5)** A 9.9.9 stub in the bundle → the
> refusal names **both versions** ("Expected version 1.29.23", "Reported
> version 9.9.9"), renders **no app content**, and our own spawned child
> is killed. **(6)** `SIGKILL` the sidecar → the relaunch has it back in
> **1 s**. **(7)** A foreign listener on 8765 → refused, **not stopped**,
> and no second kernel is spawned.
>
> **The rotation, measured live in the same session.** The Settings Save
> wrote the login keychain (`security find-generic-password` read it
> back), the RUNNING kernel's `<data-root>/auth-token` was rewritten, the
> kernel accepted the NEW token and rejected the old one with a 401 —
> **no restart** — and the panel said "Token saved to the login keychain,
> and the running kernel is using it now." That also retires **III-2's
> app-level keychain acceptance**, which had been owed since III-2: the
> app's own Settings dialog wrote the login keychain on the real machine.
>
> **One thing the drill cost, recorded because it is the operator's
> machine.** The operator's launchd kernel holds 8765, and the bundle's
> kernel wants the same port, so the drill stopped the service
> (`launchctl bootout gui/501/com.brain.server`) and restored it
> afterwards (`launchctl bootstrap`); the service is back and healthy
> (`x-api-version: 1.29.21`, the installed build). The drill also cleared
> the keychain entry it wrote and removed the data root it created, so
> the next launch is a clean first run. **A drill that disturbs the
> operator's machine must restore it, and the restore is part of the
> drill's evidence.**
>
> **Still OWED, named not hidden:** III-4 (updater),
> III-5 (Developer ID, notarization, the clean-machine tail), III-6 (the
> offline approval queue); and the `cargo metadata --no-deps` conversion
> of the S2/S3 Cargo arms, which are text scans today — a comment naming
> `reqwest` still turns them red, which is the ceiling recorded rather
> than hidden.
>
> **ponytail:** one function, one DTO, six pins, three i18n strings × five
> catalogs; no new dependency, no new crate, no new file, no new knob; the
> UI change is three branches in the Settings panel and a DTO the core
> already knew how to produce.

> **R130 "Reachable" (Phase III-4a, 2026-10-20): the shipped app could not
> talk to its own kernel, and every drill so far had been blind to it.**
> Three rounds of GUI drills drove the real bundle, and the app rendered
> correctly every time — the refusal path, the consent screen, the 0700/0600
> law, the rotation. What none of them could see is the thing the USER
> cannot see either: after the kernel starts, the app still cannot read a
> single answer from it. The wizard showed *"Could not reach the kernel"* —
> for a kernel that was running, healthy, and answering `curl` perfectly.
>
> **The three defects, found by reading the captured request off the wire.**
> (1) **CORS.** The webview's request carries `Origin: tauri://localhost` —
> captured, not assumed. The kernel's CORS layer exact-matches origins, its
> default allowlist is loopback dev-ports only, and the shell's spawn passed
> `env_clear()` + seven keys with **no `CORS_ORIGINS` among them** — so the
> kernel answered the webview without `access-control-allow-origin`, the
> browser engine blocked the response, and the renderer saw a network failure.
> The e2e sets `CORS_ORIGINS` for its own origin (`e2e/global-setup.ts:249`),
> so **CI compensates for exactly the step the real app omits** — which is
> why six rounds of drills and a green e2e never saw it. The drills used
> `curl`, which has no `Origin` and no bearer, and no CORS.
> (2) **The credential race.** The layout kicks the credential read off
> (`void bootstrapToken()`); the keychain read is async; the route's mount
> effect fired its ONE read before it resolved — so the captured request was
> a plain `GET` with **no `Authorization` header and no preflight**, against a
> kernel that requires one, and the effect runs once, so nothing retried it.
> (3) **The unnamed refusal.** The browser engine reports a blocked response,
> a rejected bearer and a dead port identically, so one sentence covered all
> three — the generic refusal this tree's own law forbids.
>
> **The fixes.** The spawn's **EIGHTH** key is `CORS_ORIGINS`, carrying this
> app's own origin and nothing else: the kernel stays loopback-bound and
> auth-required, and the origin decides only whether the *webview* may read a
> response, never who may call it. The bootstrap is memoized and shared, and
> the typed client's request middleware **awaits** it — the credential is a
> precondition of the first read, not a race with it; every route benefits
> and no route changed. And the Rust core now asks the kernel what the
> webview would experience (`kernel_wire_status`), so the renderer **names**
> the fault: unreachable, origin-not-allowed, no-credential, unauthorized.
> The origin fact is read from the **auth-exempt** `/health` and the
> credential fact from an authenticated route, because the auth middleware
> sits **outside** the CORS layer — a 401 carries no allow-origin, so
> reading the origin off a 401 would blame CORS for an authorization fault.
> Measured, not assumed.
>
> **The drill (the real bundle, 2026-10-20).** From a zero state: consent,
> `Continue`, the kernel starts — and the wizard loads **"Choose a form"**.
> The packs arrived on the FIRST read. The live kernel's environment is
> exactly **eight** vars (nothing inherited) with
> `CORS_ORIGINS=tauri://localhost`; it answers that origin with the
> allow-origin and a foreign origin with none. And the two refusals are named
> on the real screen where the old build said *"Could not reach the kernel"*:
> *"The running kernel does not allow this app's origin, so the browser blocked
> its answer. Start the kernel with CORS_ORIGINS naming the shell's origin."*
> and *"The running kernel rejected this app's credential. Save the current
> token in Settings."*
>
> **Two lessons worth keeping, one of them cost the round a stuck screen.**
> (a) A **rebuilt** app's signature no longer matches the keychain item's
> ACL, so macOS asks the OPERATOR to allow the access — and an unattended
> drill waits on that dialog forever. It is the operator's call (the user
> entered the password); the drill's data root is removed afterwards and the
> operator's launchd kernel is restored and verified. (b) A drill that
> exercises the kernel with `curl` proves the KERNEL works; it says nothing
> about whether the APP can talk to it. The wire is the browser's wire now:
> `Origin`, `Authorization`, and the engine's own policies.
>
> **Still OWED, named not hidden:** the keychain re-prompt is a **stable
> signing identity** (III-5's Developer ID) — until then every rebuild asks
> the operator again; III-4's updater (its
> execution prompt is written and measured — see the plans repo), III-5 and
> III-6; and the `cargo metadata --no-deps` conversion of the S2/S3 Cargo
> arms.
>
> **ponytail:** one env key added to an allowlist that was already seven, one
> memoized promise, one middleware await, one command, one DTO, one fault
> enum, one new module, four i18n strings × five catalogs; no new dependency,
> no new crate, no new plugin, no new capability, no `dangerous*` flag, no
> route changed, no unsafe. The whole round is the difference between the
> kernel answering and the APP being able to read the answer.

> **R123 "Tally": the suite is measured, and the register says who owes.**
> The test suite is the largest artifact in this repo and nothing had ever
> counted what it is *made of*. `scripts/test-census.py` classifies every
> test in `src/` + `tests/` by five deterministic rules, and
> `docs/test-census.md` is the committed derivation with a `--check` that
> refuses drift. **That table is the authority on the numbers — the figures
> below are this round's measurement as taken, and they move every time a
> pin lands** (they were already one behind by the time this note was
> written; the `the_test_census_table_re_derives_from_the_tree` pin is what
> catches it, and it is a pin to read, not to edit around). As measured:
> **2958 behavioral pins (87.0%), 333
> structural (9.8%), 53 controls (1.6%), 51 residual-documentation (1.5%),
> 6 unclassified (0.2%)** — and the unclassified bucket is reported, not
> hidden. One row of that count is the census's OWN pin, filed as a `control`
> because its doc-comment explains the CONTROLLED-vs-CONTROL distinction and
> therefore contains the word — the marker rule's stated one-test-wide
> ceiling, disclosed rather than reworded away.
>
> **The first classifier was wrong in four ways, and the round's own
> methodology is what found them — not a careful read.** Rule 3 only saw
> reads written IN the test body, so `tests/rbac_evaluation_pins.rs` and
> `src/docs_truth.rs` (source-scanning files whose tests route through a
> `read_repo`/`doc` helper) classified as *behavioral*; rule 4 only saw
> qualified calls, so **261 real pins that call their subject bare** —
> `verify_claim(c, q)`, `parse_flags(args)` — landed in `unclassified`; the
> doc-markers matched SUBSTRINGS, so `CONTROLLED`, `CEILINGS` and
> `STRIP_CONTROL_CHARS` read as labels; and `env!("CARGO_PKG_VERSION")` seeded
> the repo-anchor fixpoint, which swept every helper reaching
> `encrypt_v3_blob` into the structural class — including `backup.rs`'s
> `verify`, which reads an encrypted BACKUP, not the tree. Each was found by
> planting a test and watching the classifier get it wrong. The census now
> carries ten plants, including the three negative arms that matter most (a
> runtime-data read must NOT read as structural; a process spawn must NOT
> either — running a script and reading its exit status DRIVES a subject).
>
> **The register caught up with the tree.** Four guards that shipped in
> v1.29.13 without rows now have them, each with the drill it owes recorded
> (the two-thread env-drill owed on the salt lock; the compile-time fence
> owed on the operator key — a signature change, named not faked). Every
> structural row now carries an explicit **R123 decision**: one converted,
> two already retired, three LANDED-and-kept, seven keep-and-re-date with the
> project named. The law gained its missing clause: an exhaustive row owes
> nothing, because exhaustiveness IS the proof.
>
> **The conversion, and its teeth.** `crates_consumed_set_is_pinned` no
> longer greps manifest text — it asks `cargo metadata` whether a PATH
> dependency resolves, in both workspaces. Red-proof: a COMMENT naming
> `crates/brain-care-core` plus the member moved to the consumed list
> satisfied the old text condition and fails the new one. An anti-vacuity
> arm names two members cargo is known to resolve, so a probe that stopped
> seeing anything fails instead of quietly emptying the island list.
>
> **The register now has a pin.** Nothing read
> `docs/gate-law-register.md` — its first law ("a new guard lands with its
> row") was enforced by nothing, which is how four guards shipped bare.
> `the_gate_law_register_rows_parse_and_carry_their_last_column` parses both
> tables, refuses an empty `Red-proof`/`Owed`, and fails CLOSED on a
> malformed row. Drilled three ways, all reverted: blanked `Owed`, blanked
> `Red-proof`, and a row missing a column separator.
>
> **ponytail:** one script, one committed table, one converted pin, one
> register pin, no new dependency (`cargo metadata` was already there; a
> `serde_json` dev-edge would have re-locked the root for nothing), no new
> test file, no floor written before its population was measured, and no
> test deleted — the round's exit criterion is a measurement plus a
> decision per row, and "we deleted N tests" would have failed it even
> with every lane green.

> **R121 "Second Pass" + R122 "Conduit II": the closures attacked, the owed
> rows landed.** The twelfth pass (a dedicated adversarial lane against
> R115-R120's own diffs) found what a second pass exists to find, and the
> owed-conversions batch landed alongside it.
>
> **The pass's findings, all closed same-session:**
> 1. **The R116 taint inheritance was strippable** — `deep_merge` guarded
>    only `id`/`integrity`, so a `{origin:null}` patch overwrote the emitted
>    base and laundered a captured row in one call. The skip-list now guards
>    `origin`/`untrusted` too (the pin extends the revise drill with the
>    exact patch). 2. **R118's fixture-v2 was one-sided on the PLUGIN lane** —
>    `INVISIBLE_CLASSES` stayed v1 (the "fails CI by design" claim is false
>    there: the plugin's vitest runs nowhere in this repo — S9-07 biting).
>    The lane moved to v2 and the fork suite runs it (72/72). 3.
>    **`console_due` emitted the valet label raw**; it rides the read seam
>    now. 4. **`AGENT_TOKEN_FILE`'s runtime reader bypassed the mode law**
>    (boot enforced, mtime reload did not); it refuses now, and the boot
>    message names the mode before the reader can disguise it as empty. 5.
>    **THREAT_MODEL's "raw stored bytes" digest wording was false** —
>    corrected to the read-canonical truth (the PII ceiling itself stands).
>    6. Verdict asymmetry on `/add` + structured `/ingest` FILED (register
>    line, not silently aged). The other four closures verified HELD.
>
> **The owed rows landed:** the two structural pins' behavioral conversions
> (`the_ingest_lands_its_evidence_row_on_the_chain`,
> `the_edge_history_cap_bounds_and_discloses` — real app, real fixtures);
> the F4-19 digest pins (all three bases pinned against registry-resolved
> digests + a Dependabot `docker` ecosystem); the F4-21 staleness gate
> (`no_ignored_advisory_is_fixed_in_the_current_lock` — each ignore names
> its crate inline); the F4-18 islands census
> (`crates_consumed_set_is_pinned`, per-entry reasons); the F4-15 WCAG
> scope banner; **F4-05's remaining half** (domain create + import each
> write their chain row — the import REPLACES a corpus and wrote zero);
> **the F4-08 census** (the router's 52 rusqlite shapes pinned at EQUALITY —
> any delta is a named act; the migration stays staged in the register).
> The regulatory band: T4-07 reconciled (90 FR 4544/ECCN 4E091 rescinded
> pre-effect — "no rule in force" is the operative truth, the map row names
> the instrument) and L4-15 (the CRA runbook carries an
> APPLICABILITY-FIRST banner; the duty is conditional, the clocks accurate).
>
> **ponytail:** one deep_merge skip-list extension, one regex, two read-seam
> calls, one reader guard, and pins — the owed conversions reuse the
> existing harness; the census is equality-not-floor by design; no new
> knob, no abstraction, no dependency.

> **R120 "Conduit": the CI/supply-chain band lands its gates** The F4-13…23
> row cluster from the tenth pass — mechanical, runner-side, and open for two
> rounds because no round owned the workflows.
>
> 1. **A published release carries `SHA256SUMS`** over every staged file
>    (fail-closed: the manifest must cover the full dist set), generated
>    before the release step — `files: dist/*` ships it. *(F4-13.)*
> 2. **The green gate iterates every publication workflow** (`ci.yml
>    shell.yml codeql.yml fuzz.yml docs.yml`): an existing red run on the
>    tagged SHA refuses the publish; a paths-filtered skip is named and
>    accepted. *(F4-14.)*
> 3. **The shell type-check runs AFTER the supply-chain gates** — one red
>    `svelte-check` used to starve 13 steps including both audits.
>    *(F4-16.)*
> 4. **`check-doc-links.py` is anchored on its own file location** (from a
>    foreign cwd it scanned nothing and exited 0) **and is invoked by the
>    ci.yml docs-gates step**. *(F4-17.)*
> 5. **CodeQL gains the TypeScript lane** over `plugin/` + `shell/src` —
>    the code that parses model output was SAST-invisible. *(F4-22.)*
> 6. **The badges UMP arm reads the case-arm at the gate itself** (fixed
>    string), not the comment above it that a deleted gate left green.
>    *(F4-23; `--selfcheck` green on the new anchor.)*
>
> **Recorded owed, not churned** (the gate-law register's text-bound
> section): F4-19 digest pins (resolving real registry digests is an
> external act), F4-21 audit staleness (the ignore list needs pruning
> first — a lockfile decision), F4-15 WCAG evidence retarget, F4-18 crate
> islands pin. All four are rows with owners now, not silent gaps.
>
> **ponytail:** workflow YAML + one script anchor + one structural pin in
> the existing CI-shape file; no server code, no schema, no dependency.

> **R119 "Gatewright": the process round, two passes overdue.** D8-02
> ("enforcement is the uniform weak link — 3 of the top 10 are guards passing
> while violated") has been OPEN-UNROUTED since the eighth pass; its fix was
> never a patch but a standing artifact.
>
> 1. **`docs/gate-law-register.md` exists.** The law: a guard lands with its
>    row, its red-proof, and its class — behavioral, structural, or
>    exhaustive. A structural row is a regression lock, never proof, and
>    carries an `owed` column naming the behavioral conversion that retires
>    it. Seeded with the ten standing guards (each red-proof recorded) and
>    the four text-bound rows: the two R118 wiring pins, the R9-03
>    rate-limit literal, and the R9-04 file-granular Art 50 check.
>    *(Anti-rot pin `the_gate_law_register_names_the_known_guards`.)*
> 2. **The docs-truth fixes**: the COMPLIANCE.md PHI row now states the
>    read-time truth (raw PHI at rest behind volume encryption; no
>    write-time vault — the sentence the tenth pass called vacuous is
>    gone), and `dsar_deadline` is relabelled as the DSAR processing
>    deadline the operator window governs (Art 17 sets no clock).
> 3. **The register hygiene**: F4-05 corrected to PARTIALLY CLOSED (the
>    `release_quarantine` half silently closed at `memory.rs:2748`; import +
>    create still write zero audit rows), and R111 joined `SHIPPED_ROUNDS`
>    (two commits — the shell rendering steps — that the array had skipped).
>
> **ponytail:** a markdown table and one anti-rot pin — not a framework; the
> R9-03/R9-04 conversions are RECORDED as owed, not churned into the tools
> crate this round; no code behavior change beyond the two doc-truth
> relabels.

> **R118 "Lowtide": the LOW band closes before it ages into ceilings.** Eight
> known-LOW defects, one batch, each a bounded read, a parse, a header, or a
> gate — red-first pins in EXISTING files (no second test file, no knob, no
> abstraction; nothing grew one).
>
> 1. **The invisible set gained its three missing classes** — INVISIBLE PLUS
>    (U+2064), the deprecated format chars (U+206A–206F), and the musical
>    format controls (U+1D173–1D17A) — across the ONE canonical fixture and
>    both code lanes (rust + shell regex) in one change; the exhaustive
>    all-scalars pin forces the sync (a one-sided edit fails CI by design).
>    Fixture → v2. 2. **The Art-19 webhook's `subject` passes the
>    invisible/control strip** before riding to an external receiver.
>    3. **`/ingest/memory`'s audit row moved INSIDE its tx** (the sibling
>    arm's shape) — the post-commit second-connection form was a crash
>    window with no chain row. *(Offset pin
>    `the_ingest_audit_row_precedes_its_commit`.)* 4. **The tokenless
>    bridge-mount arm runs the SAME freshness + replay pair the receiver
>    arm does** — a captured signed envelope previously remounted forever.
>    *(Pins `bridge_mount_arm_refuses_a_stale_signed_timestamp` +
>    `bridge_mount_arm_refuses_a_replayed_webhook_id`, the latter with the
>    first-sight anti-vacuity arm.)* 5. **Per-domain databases join the
>    0600 family** — registry creation AND import, enforced AFTER the pool
>    build (the red-first pin caught the first placement tightening a file
>    that did not exist yet). *(Pin
>    `a_domain_db_created_by_the_registry_is_owner_only`.)* 6. **The
>    documented 0600 law is enforced on `AUTH_TOKEN_FILE`'s read path** —
>    the new secret-file seam now covers the last unguarded secret; boot
>    refuses the world-readable file when no env fallback exists, and the
>    rotation watcher's reload treats the refusal as empty → fail-safe.
>    *(Pin `a_world_readable_auth_token_file_is_refused_not_read` with the
>    compliant-file anti-vacuity arm.)* 7. **The edge-history lineage is
>    bounded** (`EDGE_HISTORY_MAX` = 500) and **names its ceiling**
>    (`truncated: true`). 8. **`brain_memory_proposal_decide` is REMOVED** —
>    the review queue is not the model's to prune: the server role-gates
>    both verbs to humans, and approve demanded a digest the queue list
>    never carried, so the tool advertised a promotion it could never
>    perform (plugin 0.6.14; the anti-comeback assertion pins the absence).
>
> **ponytail:** no new knob, no abstraction, no second test file; the two
> handler-side wiring pins are structural (the R9-03 class) and are owned by
> the gate-law register landing in the next round; zero new dependency
> edges; the wire delta is the additive `truncated` field (openapi not
> required — the schema enumerates the envelope loosely, the field is
> additive JSON).

> **R125 "Bounds" (Phase E, 2026-10-16): the measured GDL case budget.**
> `BRAIN_GDL_CASE_TOKEN_BUDGET` / `config::gdl_case_token_budget()` — the
> default is MEASURED, not chosen: 112 tokens/case p99 over the 12 judged
> `eval_run1` seeds (79 `gdl_gate` usage rows; the row now carries the
> exchange's usage, additive), × MAX_PHASE_ATTEMPTS (3) = **336** — the
> happy-path measurement with full headroom for the bounded retry ladder.
> `the_default_case_budget_is_the_measured_p99` re-runs the seeds and holds
> the const to the measurement. **Scale caveat:** 14 tokens/exchange is the
> loopback provider's own accounting, not a frontier model's — operators on
> real providers set the env var to their own measured ceiling. The budget
> is the CASE's cumulative authority (phases draw it as `fresh_exchange`
> views — the deliberate reversal of F11's per-phase isolation, recorded in
> the `case_budget` doc), and a confirm-exchange interrupt is a receipt-
> backed `Capped`, never the unvalidatable `Routed` it was mapped to before
> (found because the budget path made it reachable).

> **R126 "Tally II" (Phase F, 2026-10-17): the doc claims, measured — and the
> measurement came out the OTHER way.** F15/F16/F18 closed: the agent-loop's
> phase-exchange cost, the delegation-depth wording, and the register rows the
> context path never had. Three commits, `1c6a3fe9` / `d687889a` / `3d6b959d`.
>
> 1. **The per-phase exchange cost is 5 / 7 / 6 / 9, not three — and the first
>    derivation of that was wrong in the opposite direction.** The doc said
>    "three asks in total per phase". Asks are not exchanges: a Verify attempt
>    spends 1 (gate rejection), 2 (contradicting re-check — no confirmation is
>    ever asked) or 3 (consistent re-check + disagreeing confirmation). The
>    round's own prompt held that plan 17's "up to 9" was an overclaim, on the
>    reasoning that three *passing* attempts cannot happen — true, and beside
>    the point: **the maximum-cost attempt is a FAILING one, and the failing
>    attempt is exactly the kind that repeats.** Three A6 disagreements spend
>    3+3+3. That arm was already driven in-tree for its own law; measuring it
>    is what disproved the prompt. Following the prompt would have measured 7
>    and closed F15 on a sentence wrong the other way. The non-Verify ladder
>    (3) is measured too, because the doc states it — a derived number is a
>    number nothing checks, and the doc pin rejected a *correct* sentence until
>    that arm existed.
> 2. **The depth bound is a runtime check, and depth 2 is load-bearing.** The
>    doc claimed a "type invariant" and "not a check that could be forgotten";
>    `depth` is a private `u8` behind one `if`. The property holds and is
>    pinned (`b6_structural_depth_and_generation_overflow_refuse`). The
>    typestate that would make the old sentence true was **DECLINED** with a
>    measured blast radius (four production signatures + fixtures, in a file
>    F13 had just stabilized) — and depth 2 is documented as the adversarial
>    re-check child's OWN exchange, because the next reader proposes exactly
>    the refactor that deletes the falsifier.
> 3. **The register's context-path rows name THREE guards, not two.** The
>    prompt's row named `every_replayed_source_is_fenced`, which exists
>    nowhere in the tree; the live defect was two bare pins and zero rows.
>    Running the red-proof (revert the arm's `wrap_fenced`) turned **three**
>    pins red — the third, `plain_user_prefixes_are_not_envelopes_and_escaped_
>    content_is_checked`, was not in the plan. Running the drill instead of
>    asserting it is what found it, and naming only the two the plan knew
>    would have left a load-bearing guard unrecorded: this round's own defect,
>    reproduced inside the round that closes it.
>
> **Two pins caught their own author, and both fixes are recorded in the
> register.** The architecture pin's phrase check matched "three asks in total
> per phase" against RAW markdown, where the doc wraps it `**three\nasks in
> total` — green on the very lie it was written to catch; every phrase check
> now runs on whitespace-collapsed text. The depth pin's first drill restored
> only "type invariant" and left it green, because it refused the two long
> overclaim phrasings rather than the claim — the shortest form of a lie is
> the one that must be refused. The F15 doc pin's drill is the one worth
> keeping: dropping the ceiling 9→7 leaves every individual number still
> MEASURED, so only the ceiling arm fires (`It asserted [3, 5, 7, 6, 7]`).
>
> **A second pass found two of them measuring more than their claim**, and both
> fixes are in the register row: the measurement pin's arms each asserted a
> whole-case request count (11/13/11/14), which encoded the five-phase prefix
> and the Handoff exchange as much as Verify — adding a phase would have broken a
> pin about exchange COST for an unrelated reason. Replaced by an
> instrument-vs-instrument equality that needs no phase knowledge (every
> dispatched exchange leaves exactly one `control:exchange` row — they agree
> EXACTLY, the child's included). And the two delegation doc pins were
> substring-matching a 90 KB document, so a symbol named in an unrelated section
> would have satisfied them; they now read one section. **All four red-proofs were
> re-run after the refactor, because that is exactly when a pin goes vacuous
> quietly** — and the first attempt at the third drill reported green because the
> DRILL was buggy (it consumed its own line index before using it), which would
> have been filed as a passed red-proof. Cost: three doc pins at 0.00s, the
> measurement at 3.33s for five real case runs, zero production-code cost.
>
> **ponytail:** one measured const set the prose is held to, two doc pins, one
> anti-rot name list, three register rows, three red-proofs RUN and recorded;
//! no behaviour change, no schema, no wire, no new dependency, no new error
> type, no new test file, no floor written before its population was
> measured — and the round's exit criterion was that every claim reads TRUE at
> the commit that changes it, which is why the one commit whose measurement
> contradicted its own prompt is the interesting one.

## Retired rounds — where the older log went

**R117 "Console" (2026-10-12) and everything older** — R116→R68 and the
v1.28.x version notes — moved verbatim to
[`docs/AGENTS_HISTORY.md`](docs/AGENTS_HISTORY.md) on 2026-10-14.

This file had grown to 3,958 lines, 3,717 of them that log: the
per-round history its own footer says it does not keep here. Every round
below is **already** in `CHANGELOG.md`, structured by release rather than
by round — R115–R122 in §[1.29.13], R97–R99/R108–R109/R112 in §[1.29.12],
R95/R104–R107 in §[1.29.11], R93/R100–R103 in §[1.29.10] — so the log was
a second, less retrievable copy of the same reasoning.

**The cut point is R118 "Lowtide".** The five rounds above still name
work owed — R123's census measurement and owed rows, R122/R121's
closures, R120's CI band, R119's gate law, R118's LOW band — so they
stay. Everything from R117 back is closed, shipped, and carried by the
changelog. A round that still owes something re-enters here when it does.

## Architecture Law (the steering)

- **Two layers, one pattern.** Handlers are protocol adapters ONLY: parse →
  OptPrincipal → `authorize`/`authorize_role` → one `spawn_blocking` →
  service-core call → read-seam shaping → response. ALL SQL, bounds/caps,
  FK-children ordering, and invariants live in domain modules
  (`src/workflow/*`; `src/service/*` from v1.28.46) taking `&Connection` /
  `WorkflowTx` — never Pool, AppState, or axum types.
- **Audit-per-write.** Every mutation emits its hash-chained audit row INSIDE
  the caller's transaction (`record_tenant`, SAVEPOINT-nested): a transition
  and its evidence commit or roll back together.
- **Fail-closed everywhere.** Authz gates, poisoned locks, unreadable config,
  quarantine, legal holds, DPO switches: error paths deny loudly; silence is
  never certified (`let _ =` on writes is forbidden).
- **Read seam unconditional.** Every emitted text field passes
  `sanitize_read`; untrusted content is screened + stripped at WRITE time
  (`screen_content` pattern); LLM-facing payloads are fenced.
- **Bounds law.** Every list surface capped, every input bounded, every cap
  pinned by a test; SQL never decides a row's fate (Rust-side pure arbiter).
- **Wire-contract discipline.** Any route change ships openapi.yaml +
  route-coverage + route-authz guard-table entries in the same commit; the
  `x-api-version` stamp moves only when the wire contract moves.
- **Convergence.** New code is ALWAYS a service core; legacy extraction is
  trigger-based (piggyback / pre-storage-adapter deadline); handler-side SQL
  is ENFORCED ZERO by the `no_sql_in_handlers_enforced` guard (Cornerstone —
  any match in `src/handlers/**` fails CI; there is no allowlist). The law's
  public statement lives in `docs/architecture.md` (the two layer rules, the
  request-flow diagram, the seam table); this file is the operational
  summary.
- **The thin binary (Capstone).** main.rs is WIRING ONLY (bootstrap →
  compose → serve), pinned ≤ 300 lines with no cfg(test) region (the mass
  lives in `tests/`); route registrations live ONLY under
  `src/server/router/**`; `server::bootstrap` stays protocol-free (no axum
  types). Each clause is machine-checked by the spire gates in
  `src/spire_inventory.rs` (`route_registrations_live_only_under_router`,
  `bootstrap_stays_protocol_free`,
  `spire_inventory_freezes_the_thin_binary`); the one fenced exception is
  `src/bin/mcp.rs` — a separate binary's single-endpoint /mcp protocol
  edge, pinned at exactly one site. The line's measured record:
  `docs/AUDIT.md` (the Spire Line close-out).

## Service-layer convergence (Foundation Line, v1.28.46+)

- New code ALWAYS builds as a service core (`src/service/<domain>.rs` owning
  SQL + invariants + audit_write inside the caller's tx); handlers are
  protocol adapters only. Templates: `src/workflow/channel.rs` +
  `src/handlers/channel.rs`, and since Plumb `src/service/retention.rs` +
  `src/handlers/govern.rs` (the extraction exemplar).
- Legacy extraction is TRIGGER-BASED, never cosmetic: (1) piggyback — when a
  feature substantively touches `recall/gate/observe/domains/clients`, extract
  that surface's core in the same release (pins first, move second);
  (2) deadline — immediately before any v2.x storage-adapter work.
  (The historical freeze-and-burn machinery — the `sql_inventory_baseline`
  per-file table shipped in Plumb at 445 across 29 files, lowered line-wide
  by Quarry/Masonry/Terrace/Aqueduct/Confluence to 78 — is RETIRED: the
  Cornerstone extraction took gate.rs 78 → 0 and the enforcing flip deleted
  the table. `no_sql_in_handlers_enforced` in `src/service/mod.rs` now fails
  on ANY statement under `src/handlers/**`, recursively. Line plan:
  `IMPLEMENTATION_ROADMAP_v1.28.46_to_v1.28.52_FOUNDATION_LINE.md`;
  executor: `EXECUTION_PROMPT_v1.28.46_to_v1.28.52_FOUNDATION_LINE.md`.)

## Operational Context (read first)

**Paths**
| What | Where |
|---|---|
| Repo | `/Users/mark/Sites/brain-server` |
| **Live DB** | `~/.openclaw/workspace/brain.db` (override: `BRAIN_DB_PATH` env) |
| Binaries | `~/.local/bin/{brain-server, brain, mcp, bench, brain-connector-stub, brain-migrate-rehearse}` (+ `brain-connector-gh`/`brain-connector-crm` when their features are built) |
| Auth token | `~/.config/brain-server/auth-token` (0600; written by `install-service.sh`) |
| Launchd plist | `~/Library/LaunchAgents/com.brain.server.plist` |
| Logs | `~/Library/Logs/brain-server.{log,err.log}` |
| Config module | `src/config.rs` (all constants + env var resolution) |

**Commands**
```sh
# Build all 4 binaries
 cargo build --release --features bench --bin brain-server --bin brain --bin mcp --bin bench

# Tests + quality gates (always run with --features bench — the bench binary is feature-gated)
# Do NOT hand-type the count. `scripts/badges.sh` derives it and writes the README
# badge; `scripts/badges.sh --verify-count` re-derives and REFUSES on drift. A
# number pasted here is a number nobody diffed against a measurement — the exact
# failure this file's own header documents, repeated. (It happened: the old
# comment here carried a count pinned to a commit twelve releases back.)
cargo test --features bench                                  # count: scripts/badges.sh --verify-count
cargo clippy --all-targets --features bench -- -D warnings   # zero warnings enforced
cargo fmt --check

# CI DRY-RUN — REQUIRED before every push/release (v1.28.29 lesson):
# the local gate alone does not cover the default-feature build or the
# side workspaces CI runs on Ubuntu. Run these before pushing:
export RUSTFLAGS="-D warnings"
cargo clippy --all-targets -- -D warnings                    # job lint-test (default features)
cargo test --all-targets                                     # job lint-test (default features)
cargo test  --manifest-path crates/Cargo.toml --all-targets          # engine-crates
cargo clippy --manifest-path crates/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path tools/steward-harness/Cargo.toml         # steward-harness-gate
cargo clippy --all-targets --features otel -- -D warnings    # otel-gate
cargo test  --all-targets --features otel
# client-gate only when client/ touched (slow: wasm + desktop headers).

# FEATURE LANES — every `[features]` entry has a job in .github/workflows/ci.yml.
# Six did not, and two of those did not compile (`injection-classifier`,
# 9 errors; `neural-embed`, 2 errors), so the breakage accumulated unnoticed.
# A lane that does not exist looks exactly like a lane that passes.
# `tests/feature_lane_pins.rs` derives the feature list from Cargo.toml and
# fails on any feature no lane builds — do NOT delete that pin; it is the only
# thing making the six lanes below durable.
for f in compliance-pack multivec injection-classifier neural-embed loom rerank-tier; do
  cargo clippy --all-targets --features "$f" -- -D warnings
done
# A lane proves a feature COMPILES. It does not prove the feature WORKS — the
# ONNX/model artifacts are not fetched in every lane, and no lane exercises
# layer-2 scoring or a live rerank.

# Repo briefing — ONE-SHOT agent overview before any work (runs <1s):
scripts/repo-brief.sh
# (versions, HEAD, dirty paths, main.rs structure, env/route/CLI counts,
#  which guards exist vs ABSENT, stale-marker probe of living docs)

# v1.28.31 lesson — these two CI jobs are NOT covered by the pre-push hook
# (it fmts only the server tree) and burned the first Charter push. Run them
# locally as part of the gate, ALWAYS before push:
scripts/lipstyk-gate.sh
 cargo fmt --manifest-path client/Cargo.toml -- --check
# lipstyk-gate: the CI watchdog's trap-proof wrapper — the raw
# `lipstyk --diff "$(git rev-parse origin/main)"` form goes VACUOUS once you
# push (origin/main == HEAD → empty diff → exit 0 having scanned nothing) and
# it never sees UNTRACKED new modules (both teeth bit on the Terrace push;
# CI caught what the local run waved through). The wrapper pins the base
# (pre-push merge-base, else HEAD~1, else pass the old tip / release tag),
# marks new files intent-to-add, and REFUSES to pass vacuously. lipstyk:
# changed lines must add no diagnostics (Box<dyn Error> returns in
# tests included — helpers inside #[cfg(test)] mods are still scanned; use
# .expect() like sibling tests instead of Result-returning test bodies).

# CI and releases (2026-10-06 billing law): PRIVATE-repo Actions are
# disabled (the free 2,000 min/month died in six days; CI + CodeQL were the
# burn). PUBLIC-repo Actions are free, so the RELEASE TAG is the public CI
# trigger: a v* tag push runs the FULL ci.yml matrix on public (main is
# still never pushed there — release.sh's law), and release.yml's
# publication step fail-closes unless that matrix is green for the tagged
# SHA: red or absent ⇒ binaries build but nothing publishes. The pre-tag
# discipline is the LOCAL gate suite PLUS release.sh's own 1f schema-drift
# drill (2026-10-19): the shell lane that carries the typed-client drift
# gate never fires on a tag push and private Actions don't fire at all, so
# release.sh regenerates `shell/src/lib/api/schema.d.ts` locally, byte-compares
# it against openapi.yaml, and refuses to tag on a mismatch — the v1.29.22
# stale-client shape. release.sh watches the public runs
# after the push and exits non-zero on a not-green verdict. The
# release builds are 4 parallel per-target jobs (linux x86_64/aarch64,
# macOS arm64/x86_64); the verify-required-assets gate refuses to publish if
# any primary brain-server binary is missing.

# README badges — NEVER hand-type them; regenerate from the real build:
scripts/badges.sh                         # prints the version/test/UMP/SBOM badge block
scripts/badges.sh --verify-count          # the REAL count compare (one full cargo test, ~4 min)
scripts/badges.sh --selfcheck             # CHEAP gate: version↔README, UMP, checklist, SBOM
# (derives version from Cargo.toml, test count from cargo test --features bench,migrate)

# Install + restart launchd service (installs the CLI binaries, strips macOS
# provenance xattr, installs the Brain Shell .app to /Applications best-effort,
# and health-checks the SERVED version against the installed binary — a foreign
# kernel on 8765 cannot satisfy it; ~3 min window for first-boot migrations)
scripts/install-service.sh

# Health + stats against running server
brain doctor
brain status
```

**Runtime**
- Server: launchd-managed, `127.0.0.1:8765`, `KeepAlive=true`, `RunAtLoad=true`.
- Auth: bearer token via `AUTH_TOKEN_FILE` → `AUTH_TOKEN` (server) / `BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` → default file (CLI). Off by default if no token resolves.
- macOS Sonoma+: newly-copied executables get `com.apple.provenance` xattr → Gatekeeper SIGKILLs on first exec (exit 137). `install-service.sh` strips it; manual `cp` does not.

**OpenClaw integration**
- brain-server is the **memory backend** for openclaw; the plugin (`brain-server/plugin/`, TypeScript) calls `/recall` each turn via openclaw's `before_prompt_build` hook.
- openclaw config: `~/.openclaw/openclaw.json` (plugin block at key `brain-server`).
- The `brain` CLI is built from this repo's `Cargo.toml` (`[[bin]] name = "brain"`), **not** from openclaw.

**Customer-domain assets live in a private repo (2026-08-19)**
- The case-classification work is **not** part of brain-server: plan, customer
  taxonomy, routing matrix, playbooks, spine builder + outputs moved to a
  **private** repo (not named here — this file is published to the public
  portfolio, and naming the customer is a confidentiality problem
  independent of the assets themselves). Never commit customer-domain content
  here — `.gitignore` covers `spine/` + the two spine scripts defensively.
- This repo stays the vendor-agnostic product: graph-default-on recall, the
  feature-gated `src/classify` engine (when built), proposals/suggest/audit.
  The taxonomy is *data the engine consumes*, owned by the private repo.

**Steward engine IP lives in a private repo (2026-08-21)**
- The advanced architecture docs (Steward architecture, harness audit, state
  mapping, port specs, rubric pin, diagnostics loop, compliance mapping) are
  **MemorySteward LLC IP** and live in the **private** repo `brain-steward-ip`
  (local checkout `~/Sites/brain-steward-ip`). Never commit them here —
  `.gitignore` defends the names defensively (same pattern as the
  customer-domain repo).
  The public repo ships the substrate only; the engine crates (`brain-engine-sdk`
  5,887 top-level + `host/`/`pure/` lines, 8,352 total / 127 tests at v1.28.23,
  plus the five `*-core` engines) are PUBLIC, REAL code — measured 2026-09-15.
  (The old "scaffolds stay public while empty" claim was stale — do not repeat
  it; the SDK is built, kernel-consumed, and dependency-free by design.)
- **The Steward-line implementation plans (1.27.32 → 1.27.42) live in
  `brain-steward-ip/plans/`** with the master `ROADMAP_1.27.x.md`. This repo
  keeps **no** kernel-root plan files: every `IMPLEMENTATION_PLAN_*`,
  `IMPLEMENTATION_ROADMAP_*`, `ROADMAP.md`, `DESIGN_*`, `CLIENT_ROADMAP.md`,
  `REALITY_CHECK.md`, `MARKETING_PLAN.md`, `TODO.md` and `USE_CASES.md` that
  used to sit here was **moved to `brain-steward-ip/plans/archive/` on
  2026-10-04** — see `plans/archive/KERNEL_ROOT_PLANS.md` and
  `KERNEL_ROOT_NONPLAN.md` there for provenance and the list. None was ever
  tracked (`.gitignore` 55–70), so the public tree carried 222 root `.md` files
  while git held 16; the root is now 19, all of them real project docs.
  **Do not re-add one "for context"** — the private archive is the home, and
  `tests/external_claim_pins.rs` discovers the claim-sensitive ones by CONTENT
  rather than by path, so a copy here is not what any gate reads.

---

## Known issues (open)

- ~~**CI gap — tests run on x86_64 runners only.**~~ **CLOSED 2026-09-15
  as NOT-APPLICABLE:** no Jetson/fleet deployment exists (brain-server is
  not installed on any aarch64 host), so the advisory's precondition —
  "before fleet deploys" — is absent. The release matrix still
  cross-builds aarch64 and CI still executes tests on x86_64 only;
  REOPEN at the first aarch64 fleet deployment, and then as an ARM-hosted
  CI test lane, not a manual smoke.
- ~~**Historical token leak (openclaw-side).**~~ **CLOSED 2026-09-09:** the
  final `transcript_events` copy of the agent token was redacted + VACUUMed
  (backup: `~/Downloads/audit/openclaw-agent-pre-purge-*.sqlite`, 0600), the
  agent token was ROTATED (old value dead → 401; watcher reload audited), and
  the discovery of the night: the openclaw gateway's env carried the OPERATOR
  token — the plugin had been authenticating as full superuser. It now runs
  on the AGENT token (`service-env/ai.openclaw.gateway.env`), so the Twokeys
  agent boundary is ENFORCED live (purge → 403 proven). The operator token is
  unchanged. The `${BRAIN_SERVER_AUTH_TOKEN}` placeholder in
  `openclaw.json` resolves from the same env file. Hygiene:
  `~/.config/brain-server/auth-agent-token` (the installer's designated
  `AGENT_TOKEN_FILE` target, unreferenced by the running service) was
  re-synced to the live agent token so a future installer re-run or
  `AGENT_TOKEN_FILE` adoption cannot resurrect the dead value.

History: per-release detail lives in `CHANGELOG.md` §[version] and
`docs/AGENTS_HISTORY.md` (load on demand). Nothing older than the note above
is kept here.
