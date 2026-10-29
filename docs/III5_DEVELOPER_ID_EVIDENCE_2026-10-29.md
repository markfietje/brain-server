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

## (S-2..§7 evidence lands below as the round executes)
