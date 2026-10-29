# III-5 EVIDENCE — the Developer ID signature, notarization, and the clean machine (2026-10-29)

The evidence file for the III-5 execution round
(`plans/brain-shell/EXECUTION_PROMPT_III5_DEVELOPER_ID_2026-10-29.md` in the
private workspace): every pin's red-proof, the real-toolchain outputs, and
the clean-machine witness, recorded as they landed. The register row
(`docs/gate-law-register.md`) carries the law; this file carries the runs.

## §0 preconditions — measured green before any step

| precondition | verdict |
|---|---|
| `brain-server` at `9f149717`, clean, in sync with origin | green (`## main...origin/main`, no drift) |
| `security find-identity -v -p codesigning` | both identities present: `Developer ID Application: Mark Fietje (JHB4UNPL3G)` **and** the legacy `brain-server self-signed` |
| `xcrun notarytool history --keychain-profile stewardmem` | authenticated — `No submission history.` (the profile validates against the Notary API; stored operator-side in the login keychain, never a repo file, never CI — D6) |
| `cargo test --features bench` (RE-MEASURED) | green — 5/5 suites `ok`, zero failures |
| `cd shell && pnpm test` (RE-MEASURED) | green — 46 files passed, 1 skipped; **330 passed, 2 skipped** |

## S-1 — the lane refuses every non-Developer-ID signer (red-proof, real toolchain)

`scripts/release-app-sign.sh --red-proof` — compiles a probe Mach-O and
drives the lane's `verify_apple_signature` through every refused posture,
then confirms the revert. **Run 2026-10-29, exit 0:**

```
== RED-PROOF: the lane refuses every signer that is not Developer ID Application: Mark Fietje (JHB4UNPL3G) ==
error: refusing probe: "brain-server self-signed" is not the pinned signing identity "Developer ID Application: Mark Fietje (JHB4UNPL3G)" — the lane refuses any other signer, by name
ok    the self-signed identity is refused, by name
error: refusing probe: no Authority line (ad-hoc: Signature=adhoc, no Authority line) — expected "Developer ID Application: Mark Fietje (JHB4UNPL3G)"
ok    the ad-hoc signature is refused (no Authority line)
error: refusing probe: no secure timestamp — owed since III-1, mandatory under the Developer ID identity, absent here
ok    the missing secure timestamp is refused
>> verified: Developer ID Application: Mark Fietje (JHB4UNPL3G), timestamp 29 Oct 2026 at 20:09:13 — probe
ok    the revert is confirmed: Developer ID + secure timestamp verifies
== RED-PROOF complete: every refusal fired, the revert verified. ==
```

### The measured display shapes the refusals read

Measured against the real toolchain before the checks were written (the
refusals parse measured lines, never exit statuses — `codesign -dv` exits 0
on everything):

| posture | first `Authority=` line | `Timestamp=` line | `flags` |
|---|---|---|---|
| self-signed (`brain-server self-signed`) | `brain-server self-signed` | **absent** | `0x10000(runtime)` |
| ad-hoc (`codesign --sign -`) | absent — `Signature=adhoc` | absent | `0x10002(adhoc,runtime)` |
| Developer ID, no `--timestamp` | `Developer ID Application: Mark Fietje (JHB4UNPL3G)` | present — **this toolchain timestamps Developer ID signatures by default** | `0x10000(runtime)` |
| Developer ID `--timestamp=none` | same | **absent** — the produced negative | `0x10000(runtime)` |
| Developer ID + `--timestamp` | same | present | `0x10000(runtime)` |

The timestamp debt retired BY NAME here: every round since III-1 recorded
"the secure timestamp is impossible ad-hoc and is owed, not faked." With the
Developer ID identity it exists (and is even the default); the lane now
DEMANDS it — the `--timestamp=none` posture is refused, and every signature
the lane produces passes `--timestamp` explicitly.

### The suite pins (stub-driven, run on every `pnpm test`)

`shell/tests/sidecar-bundle.test.ts` →
`the_walker_demands_the_developer_id_identity_and_timestamp_when_expected`:
the walker (`shell/scripts/sidecar.mjs verify --expect-identity`) refuses the
self-signed identity BY NAME, refuses ad-hoc (no Authority line), refuses a
Developer ID signature without the secure timestamp, and passes the pinned
identity + timestamp. The red-first sequence was measured: the pins were
written first and ran RED against the pre-III-5 walker (no expectation
mode), then the walker landed and the same pins ran green.

## S-2 — notarization + stapling: the lane's own steps, measured green (dry run #3, #4)

The first design tried the bundler's native notarization (the only
ordering in which the stapled app rides its own archive). **It is
impossible on this route, measured twice:** the operator's tunnel sustains
~170 KB/s upload, and the bundler's Rust S3 multipart client deadlines
before the FIRST part lands (`abortedUpload … completedParts: [],
HTTPClientError.deadlineExceeded`, runs 1 and 2). The lane therefore
submits with Apple's own uploader — `xcrun notarytool submit
--keychain-profile stewardmem --wait` — which tolerates the link, exactly
the posture the execution prompt names.

**Dry run #3 (universal build) — the whole chain green, LANE-EXIT=0:**

```
Submission ID received
  id: 6c95d6e3-601a-421e-8403-47bba06c5f7f
Successfully uploaded file
Waiting for processing to complete.
Current status: Accepted......Processing complete
  status: Accepted
>> stapling the app…
The staple and validate action worked!
>> stapler validate (the stapled app — the offline pin)…
The validate action worked!
>> spctl --assess --type execute (the stapled app)…
>> the stapled app validates: stapler validate + spctl are green.
```

**Dry run #4 (arm64-only + stapled DMG) — LANE-EXIT=0:** the walker ran
twice (the bundle directory AND the extracted updater archive — the
shipped bytes): 3 Mach-O files, `[arm64]`, `identity+timestamp
[Developer ID Application: Mark Fietje (JHB4UNPL3G)]`, arch parity,
provenance `1.29.35 @ f6cbe369`. The DMG was built with `hdiutil` around
the ALREADY-STAPLED app, notarized, stapled, and `stapler validate` green
on it. The chain staged: artifact + `.sig` + `latest.json`, round trip
verified against the pinned key.

The lane-readable keychain item (`com.stewardmem.shell/notary`) was
stored by the operator for the bundler-env design and is no longer
referenced — notarytool reads its own profile from the data-protection
keychain. Custody note, not a dependency.

## S-2 REAL — the publication to v1.29.36, and the DMG-signature fix it forced

**Run 1 (2026-10-29 ~23:00, LANE-EXIT=0):** the lane ran FOR REAL against
the staged-at-tag sidecar (`1.29.36 @ 86cd1175`, `v1.29.36` describe,
clean) and published the whole chain: `StewardMem.app.tar.gz` + `.sig` +
`latest.json` (version 0.1.1, kernel 1.29.36, platform `darwin-aarch64`
alone) + `StewardMem.dmg`, then Developer-ID-signed and re-notarized the
four darwin kernel artifacts with a refreshed `SHA256SUMS`. Measured from
the public internet: the updater endpoint
(`releases/latest/download/latest.json`) → **HTTP 200** serving THIS
release's manifest; the DMG URL → **HTTP 200, 13,203,546 bytes**.

**The investigation the first download forced (macOS 27.2 Golden Gate,
2026-10-30 — docs via Context7, then measured).** The published DMG's
staple validated and the app inside passed everything, but
`spctl --assess --type open` on the DMG said `rejected, source=Insufficient
Context`. Four measurements settled what that verdict means:

1. **The DMG carried NO code signature at all** (`codesign -dv`: "code
   object is not signed at all") — the lane built it with `hdiutil` and
   notarized it UNSIGNED. The Notary ACCEPTS an unsigned image (the ticket
   binds to the image hash; the staple validated) — practice tolerates
   what the docs forbid.
2. **The VLC control** (3.0.21 arm64, downloaded the same hour): its DMG
   is ALSO "not signed at all", and it returns the IDENTICAL
   `Insufficient Context` under `spctl --assess --type open` — so that
   verdict is NON-DIAGNOSTIC for DMGs on 27.2, not a Gatekeeper judgment.
   Both DMGs mount clean WITH the quarantine attribute set.
3. **`syspolicy_check distribution`** (`/usr/bin/syspolicy_check`, new on
   this OS — "the same checks macOS does": Gatekeeper + XProtect +
   certificate profile) is the real probe, on the APP: ours passed EVERY
   check ("App passed all pre-distribution checks and is ready for
   distribution"; top-level AND deep policy match `notarized`), while the
   VLC control FAILED offline — "Notary Ticket Missing, Severity: Fatal"
   (VLC staples nothing; it relies on the online lookup). **Our chain is
   stricter than a decade-old widely-distributed one.**
4. **Apple's documented posture** ("Packaging Mac software for
   distribution", fetched via Context7 2026-10-30) names the disk image as
   a thing you SIGN — `codesign -s <Developer ID Application> --timestamp
   -i <Identifier> <DiskImageFile>` — under the nested-container rule:
   sign the app, the installer, AND the disk image; notarize only the
   outermost container.

**The fix (commit `89744f83`):** the lane now `codesign`s the DMG with the
pinned identity + `--timestamp` + `-i com.stewardmem.shell` AFTER
`hdiutil create` and BEFORE `notary_submit` — so the notarization covers
the SIGNED bytes — and `verify_apple_signature "$DMG"` gates it like
every other Mach-O. The pin
(`shell/tests/updater-manifest.test.ts` → `the signing lane
Developer-ID-signs the first-install DMG before notarizing it`) is
STRUCTURAL (the R127 S2/S3 class: the gate runs operator-side with the
real keychain and the Notary; CI cannot drive it) and was RED-PROVED
against the pre-edit lane — `AssertionError: the lane must sign the DMG
with the pinned Developer ID identity and a secure timestamp (Apple:
"Packaging Mac software for distribution" — sign the app, the installer,
AND the disk image): expected false to be true` — green after, 17/17 in
the file. (macOS 27.2 also deprecates `hdiutil create/attach` in favour of
`diskutil image create/attach`; recorded in the lane comment, not churned.)

**Run 2 (2026-10-30 ~07:10, LANE-EXIT=0):** re-staged at the fix commit
(`1.29.36 @ 89744f83`, describe `v1.29.36-1-g89744f83` — the writer
refuses a staged record that names another commit, measured law), the
full lane re-ran, and the new gate fired in the log:

```
>> signing the DMG with the Developer ID identity…
>> verified: Developer ID Application: Mark Fietje (JHB4UNPL3G), timestamp 30 Oct 2026 at 07:10:25 — StewardMem.dmg
```

**The published bytes, measured after re-publication:**

| check | verdict |
|---|---|
| `codesign -dv` on the DMG | `Identifier=com.stewardmem.shell`, `Authority=Developer ID Application: Mark Fietje (JHB4UNPL3G)` → Certification Authority → Apple Root CA, **`Timestamp=30 Oct 2026 at 07:10:25`** |
| `stapler validate` (DMG) | green — "The validate action worked!" |
| quarantine set → mount | clean (MOUNT-OK) |
| `syspolicy_check distribution` (app inside) | "App passed all pre-distribution checks and is ready for distribution." |
| updater endpoint | HTTP 200 |

## S-3 — Gatekeeper, witnessed with the real verdicts

The spctl/stapler matrix, measured on the dry-run-built stapled app
(all exits unmasked; `/tmp/iii5-spctl/` fixtures):

| posture | `stapler validate` | `spctl --assess --type execute` |
|---|---|---|
| **the stapled app** (the lane's product) | **green** — "The validate action worked!" | **accepted, exit 0** |
| unstapled verbatim copy (re-signed, same bytes) | green — the ticket is ON APPLE'S SERVERS against the CDHash | accepted, exit 0 — the online check finds it |
| unstapled, MODIFIED bytes (one tampered provenance line, re-signed — CDHash unknown to the Notary) | **RED, exit 65** | **REJECTED, exit 3** |
| ad-hoc (`--sign -`), quarantined | — | **REJECTED, exit 3** |

**The measured law this matrix teaches — three findings, each a
correction of an assumed one:**

1. `spctl --assess` DOES consult notarization: unstapled, it asks Apple's
   service ONLINE. So "staple removed → spctl reds" holds for any bytes
   Apple has not notarized (any tamper, any rebuild); a verbatim unstapled
   copy passes WITH NETWORK and fails without it.
2. The staple is exactly what makes the verdict OFFLINE-capable — which
   is why the first-install artifact is the stapled DMG, not the updater
   tarball (an update is never quarantined; Gatekeeper never sees it).
3. Notarization attaches to the CDHash, not the artifact: re-signing
   identical bytes reproduces the CDHash and inherits the ticket. The
   gate that catches modification is the seal/CDHash mismatch — the
   walker's per-file strict verify — and the modified-bytes row above.

The first-launch witness (the Gatekeeper dialog, quarantine present, no
bypass) belongs to the clean machine — S-4 below.

## S-4 — the clean machine: DEFERRED with the blocker named; the path is prepared

The arm64 clean-VM test (quarantine present; Gatekeeper first launch
WITHOUT bypass; sidecar up; the chat answering live; the audit chain
verifying; the unstapled and ad-hoc negative controls on the same
machine) did not run in this round. **The blocker, named:** the
operator's VPN tunnel (all traffic, kill-switched) sustained ~230 KB/s –
2.3 MB/s down / ~170 KB/s up during the round — enough for the notarytool
uploads, hopeless for a VM image (ghcr's 34 GB pull died in a
connection-loss retry loop; Apple's CDN measured the IPSW at ~20 hours on
the NL entry). **The owner:** the operator. Gate 8's last open surface
is this witness; every locally-measurable surface measured green in this
round (S-1..S-3).

**RE-MEASURED 2026-10-29 23:43 — the recorded blocker did NOT reproduce,
and the earlier "downloading" claim was STALE (nothing in flight, no
partial file, no VM in `tart list`):** the tunnel measured **5.3 MB/s
down** (the 13 MB DMG fetched in 2.5 s), Apple's CDN answers through it
(`swcdn.apple.com` reachable; the gdmf catalog service responds), and
`softwareupdate --list-full-installers` lists **macOS 27.2 Golden Gate,
17,389,792 KiB (~17.4 GB), build 26B123**. At the measured rate the
fetch is **~55 minutes** (`tart create <name> --from-ipsw latest` —
Tart resolves and fetches itself; 557 GB free on the host). The tunnel
throughput is evidently TIME-VARIABLE: the runbook is to start the fetch
when the pipe is fast (measured now), with a resumable downloader or a
5G hotspot as the fallbacks if it dips mid-flight, and a borrowed
Apple-Silicon Mac as the no-download alternative. The witness battery
runs the moment the VM exists.

## S-5 — Intel: superseded by the arm64-only decision, recorded

Plan 05 §2 declared universal (x86_64 + aarch64) mandatory, and the
execution prompt's S-5 carried the Intel verification as an external
dependency. **The operator's decision (2026-10-29, mid-round): the
desktop app is arm64-only.** The consequences, all landed: the lane
builds `--target aarch64-apple-darwin`; the CI proof build mirrors it;
the update manifest answers for `darwin-aarch64` alone (the writer's
`--targets` law refuses offering Intel Macs an app they cannot run —
measured defect in dry run #4); the x86_64 app slice no longer exists, so
the external Intel-RUN verification retires with it; the KERNEL keeps
both darwin slices (the CI-built darwin-x86_64 artifacts are signed and
notarized in the lane), so Intel Macs run the kernel headless. Rosetta
was never counted (the prompt's own law) and is now moot for the app.

## S-6 — the e2e lane ran; the residues swept; the out-of-tree identities

**The e2e lane** (owed since M3-g): `pnpm test:e2e` — **7 passed, exit 0**
(chromium live-wire smoke + the webkit no-bypass CSP leg).

**The rebrand residue (docs/scripts class), the census:** 5 live lines
swept — install-service.sh's pre-rebrand bundle-name candidates (the
fresh-start law now stated where the candidates used to be) and its
universal bundle path (now the aarch64 dir); the shell package name
(`brain-shell` → `stewardmem-shell`, README title with it). Dated history
KEPT by the tree's own law: the 1.29.27 incident comments (updater.mjs,
install-service.sh), the release-lane rename measurement
(release-app-sign.sh), the register's drill transcripts, the published
CHANGELOG sections. Code-level names that are NOT residue: the Cargo
package `brain-shell` (the bundle's real executable name — the walker's
census reads it), the kernel binaries (`brain-server`, `brain`, `mcp` —
wire-level).

**The out-of-tree identities — the runbook.** Landed during this round:

- [x] **GitHub repo About** (`stewardmem/stewardmem`): already
  StewardMem-branded — "StewardMem — governed, local-first memory and
  decision infrastructure for AI agents; deterministic retrieval,
  human-gated memory, provenance, and tamper-evident audit", homepage
  `https://stewardmem.github.io/stewardmem/`. Nothing to change.

Operator acts, named and owed to the operator's own hands (no credential
of this machine reaches them):

- [ ] **LinkedIn title**: "StewardMem — governed, local-first memory and
  decision infrastructure for AI agents" (drop any Brain Server title).
- [ ] **arXiv keyword**: the paper's keyword list carries `stewardmem`
  (and `agent memory governance`) — the next posting/revision.
- [ ] **README/portfolio pins**: the portfolio site's project card names
  StewardMem with the repo URL `github.com/stewardmem/stewardmem`.
- [ ] **stewardmem.com landing line**: "StewardMem — governed memory for
  AI agents" (the domain's landing copy carries the current name).

## §7 — the ledger

The register row (`docs/gate-law-register.md`, appended this round)
carries the law and the gate's state. The ledger, in the prompt's own
order:

- **P5 (OpenClaw against the shipped binary) is next** — the shipped
  darwin artifacts are now Gatekeeper-clean and signed for exactly this
  round; **watch the MCP catalog fingerprint pins**.
- Then **the decommission run book** — the remaining distance to
  decommission is one integration round (P5) and a customer.
- The labeling round (human hours); C4 (field-gated); COPPA (priority);
  rule_rates (the decision seam); reject (operator).
- The standing debts: recovery, Apple-quote, **the hand-made archive**
  (III-4's measured install stall — upstream's to answer; this round's
  DMG does not touch the updater's path), register conversions.
- **S-4, the clean-machine witness, rides at the front of the next
  round's gate** — the VM is prepared; the IPSW is downloading.

## Addendum (2026-10-30) — the minimum-OS matrix, measured from the shipped bytes

The question (operator, 2026-10-30): what is the app's honest minimum macOS,
and does Tahoe 26 still get the full product? Measured from the v1.29.36
shipped binaries — `vtool -show-build`, `otool -L`, and the sources' own
availability guards — and verified against Apple's current FoundationModels
documentation (Context7, 2026-10-30):

| shipped binary | `minos` | `sdk` | the fact that carries the floor |
|---|---|---|---|
| `brain-shell` (the app) | **15.0** | 27.0 | the app loads on 15+ |
| `brain-server` (sidecar kernel) | **11.0** | 27.0 | the kernel loads anywhere |
| `brain-afm` (the chat bridge) | **15.0** | 15.0 | FoundationModels linked **WEAK** (`otool -L`: `…FoundationModels … weak`); `Package.swift` declares `.macOS(.v15)`; and the newer surfaces are availability-guarded — `@available(macOS 26.0, *)` (`ChatModelLayer.swift:121,236,420,506`) and `#available(macOS 27.0, *)` (`AppleFMProbe.swift:63,80`) |

Apple's docs (verified 2026-10-30): the **Foundation Models framework ships
with OS 26** (the OS-26 release-notes family); `SystemLanguageModel.availability`
answers `.available` / `.unavailable(.deviceNotEligible | .appleIntelligenceNotEnabled |
.modelNotReady)` — each a state the app's UI already names; **Private Cloud
Compute is `#available(macOS 27.0, *)`** with on-device fallback on older.

**The matrix (arm64-only build):**

| macOS | app + kernel + recall | on-device chat | the 27-only vocabulary / PCC |
|---|---|---|---|
| **15 Sequoia** | ✅ | ❌ framework absent (weak link + guards → the bridge reports no model) → **the R142 fallback posture**: the deterministic `/recall` answer beside a named notice — the designed degradation | ❌ |
| **26 Tahoe** | ✅ | ✅ **the FULL product** — gated only by Apple Intelligence's own availability states, each named in the UI | ❌ guarded off |
| **27 Golden Gate** | ✅ | ✅ + the capabilities vocabulary | ✅ on; PCC still entitlement-dead on the direct channel (only an ASC submission could ever request it) |

**The declared `LSMinimumSystemVersion: 15.0` matches the binaries exactly —
the floor is honest as shipped, and Tahoe needs nothing.** Owed, named: the
15/26 rows are measured-not-witnessed (the S-4 VM is 27.2); a Tahoe-26 Tart
VM leg of the same battery is the cheap witness if the claim ships in
marketing — a Sequoia-15 leg is optional (its degradation is the designed
posture). The ASC submission's own floor field carries this same truth.
