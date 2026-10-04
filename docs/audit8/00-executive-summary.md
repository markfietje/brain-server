# Eighth-Pass Full-Spectrum Audit — brain-server × openclaw fork (2026-10-04)

**Scope:** brain-server **v1.29.2**, HEAD `e9c71919` (2026-10-04), tree clean × openclaw
fork `~/Sites/openclaw` HEAD `1d2d29b22` (2026-09-25), plugin 0.6.10 vs canonical 0.6.11.
**Mode:** dual — Mode A forward sweep + Mode B claims falsification + design critique +
worldwide regulatory sweep.
**ID series:** `F8-*` (A-bugs) · `D8-*` (design) · `R8-*` (falsified claims) ·
`P8-*` (parity) · `K8-*` (fork) · `S8-*` (satellites/supply chain) · `L8-*` (regulatory) ·
`T8-*` (docs-truth).

---

## §0 — THE BRIEF'S PREMISES, MEASURED

The audit prompt was written against `v1.28.82 "Vigil"` (2026-09-12) and stated *"you are the
FOURTH pass."* **Every load-bearing premise in it is false at HEAD.** Recorded before any
finding, because a falsified premise invalidates the frame built on it:

| Brief's premise | Measured at execution time | Consequence |
|---|---|---|
| HEAD is `v1.28.82 "Vigil"`, today is 2026-09-12 | HEAD `e9c71919`, **v1.29.2**, today **2026-10-04**; schema **1.32.25** (not 1.32.23) | The whole release window asked about (`§[1.28.77]`..`§[1.28.82]`) is 7 releases stale. Two prior audit reports exist in-tree. |
| "You are the FOURTH pass" | **Eighth.** `SECURITY_AUDIT_20260912_FOURTH_PASS.md`, `…FIFTH_PASS.md`, `…SEVENTH_PASS.md` already committed | Prior reports were **not read** (brief said fresh eyes). Their conclusions are neither inherited nor contradicted here. |
| Deliverable = overwrite `docs/SECURITY_AUDIT_20260912_FOURTH_PASS.md` | That file exists with a real prior round in it (23,898 bytes) | **Not overwritten.** This pass writes to `docs/audit8/`. Overwriting would destroy a colleague's work. |
| Fork HEAD `8634b4fca89`, 0 behind / **76** ahead, plugin **0.6.5** | HEAD `1d2d29b22`, 0 behind / **90** ahead, plugin **0.6.10** | The fork's existential-risk table (§ rebase) is measured against a 14-commit-later fork. |
| CRA Art 14 clocks went live "yesterday"; verify against 2026-09-12 law | Clocks live **23 days**; **CT CART general duties fired 2026-10-01, three days ago, and the map still files them as "Scheduled"** | → `L8-01`, the highest-severity regulatory finding. |
| The Parity release's gap-ledger claim is to be attacked | **Already corrected upstream** at v1.28.87 → "balanced (4 known residuals with owners)" | The brief attacks a claim the repo had already retracted. Attacked the *retracted* form instead (§ parity). |

**HEAD moved during the audit.** It began at `39443a87` and became `e9c71919` mid-run
(a concurrent session landed three docs commits). Two legs therefore reported different
HEADs. Per the stop rule I **re-ran every decisive attack myself at final HEAD** rather than
reconciling by assertion; every verdict below is at `e9c71919`.

---

## §1 — EXECUTIVE SUMMARY

The system's four mantras **hold under test**. The read seam, digest-binding, revocation
reach, and the components/deployers legal distinction all survived direct attack (§7).
What this pass found is narrower and, in three places, more serious: **the enforcement
machinery is weaker than the claims that cite it.**

**Top 10 by risk × likelihood:**

| # | ID | Finding | Why it ranks here |
|---|---|---|---|
| 1 | **F8-01** | `no_sql_in_handlers_enforced` cannot see `PRAGMA`/`VACUUM`/`REPLACE` — **and two live violations are in the tree**. Guard **passes green** with them present (I ran it). | The repo's most-cited architectural law has no working enforcement. Two violations already shipped proves the leak is not theoretical. |
| 2 | **F8-08** | DSAR erasure certifies `completed` while an **approved proposal's full text survives** — its own comment names this exact hazard, then implements a fix that doesn't cover it. Drill-proven. | A **compliance certificate that certifies an erasure that did not fully happen**, on a system whose product IS erasure. |
| 3 | **K8-01** | Fork `wrapUntrustedToolText` skips its envelope on a **substring of attacker-controlled content**. One line in any file disables the hardening on four untrusted seams. | Trivial, remote-ish (via any tool the agent reads), and defeats a *shipped* fork control. |
| 4 | **F8-02** | Authz oracle **never reads `required_action`**; its doc claims two enforcement properties the constructor makes unreachable. The only pin is self-asserting. | The authorization layer *looks* like enforcement and is coverage-only. |
| 5 | **K8-02** | `link-reader-content.ts` bypasses `remoteImageHosts` completely — **upstream-owned, so outside the fork's hardening**. | The image-gate guarantee is true only for one of two render paths. |
| 6 | **S8-05** | `signal-gateway`: bind guard and auth guard are independent `if`s → `SIGNAL_GATEWAY_ALLOW_REMOTE=1` with no token serves **send/enumerate/SSE unauthenticated**. | Unauthenticated remote send on a live messaging identity; shortest exploit here. |
| 7 | **F8-03** | 30 s `TimeoutLayer` returns **408 while the abandoned `spawn_blocking` write still commits**. | Every write route: operator sees failure, retries, double-applies. Silent. |
| 8 | **L8-01** | CT CART duties **live since 2026-10-01**; `US_STATE_MAP.md` still says "Scheduled". | The repo set this clock itself and never re-armed it. A deployer reading it is told to wait. |
| 9 | **L8-02** | `/.well-known/ai-notice` is framed as "the Art 50 disclosure" but Art 50(5) requires disclosure **at first interaction**. | Overclaim to a procurement reader. Component scope is *correct*; the claim shape is not. |
| 10 | **F8-05** | `CRATE_TEST_FLOOR` is a **raw `#[test]` substring count** with ~146–311 units of slack and no comment-stripping. | The one spire guard that is gameable; the other four carry genuine self-pins (verified). |

**Three meta-findings worth more than any single bug:**

- **The anti-vacuity discipline is real and works.** I set out to find vacuous pins and found
  **zero** genuinely vacuous ones. `sql_statement_counter_still_fires`,
  `handler_body_ignores_comments_naming_the_symbol`, and the read-seam fixture pins all
  survived scrutiny. The `exec_spawn_carries_kill_on_drop` lesson was learned.
- **The docs are unusually honest, and I could mostly confirm it.** `docs/cra.md` disclaims
  conformity assessment; `provenance.rs` says "NOT C2PA"; `docs/api.md` refuses to claim SLSA;
  the Colorado row flags its *own* litigation uncertainty. This made the audit faster and the
  real findings sharper — I did not have to fight marketing.
- **Every genuinely serious finding is a *gap between a claim and its enforcement***, not a
  hole in the security model itself. The model is sound; the machine checks under-deliver.

---