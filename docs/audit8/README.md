# Eighth-Pass Full-Spectrum Audit — index

**brain-server v1.29.2** (`e9c71919`, 2026-10-04) × **openclaw fork** (`1d2d29b22`, 0 behind /
90 ahead).

> **Why this is not `docs/SECURITY_AUDIT_20260912_FOURTH_PASS.md`.** The brief specified that path.
> **It already exists** with a real prior round in it (23,898 bytes, committed), and the brief's own
> framing — *"you are the FOURTH pass, today is 2026-09-12"* — is false: this is the **eighth** pass,
> seven releases later. Overwriting a colleague's report would have destroyed work and destroyed the
> evidence that the gap-ledger "zero" claim was already retracted upstream at v1.28.87. This pass
> therefore writes to `docs/audit8/` and the finding IDs are `8`-series (`F8-`, `D8-`, `R8-`, `P8-`,
> `K8-`, `S8-`, `L8-`, `T8-`). Prior reports were **not read** (fresh-eyes instruction).

## Read in order

| File | Contents |
|---|---|
| `00-executive-summary.md` | **§0 the brief's premises, measured** (six of them false) · §1 top-10 + three meta-findings |
| `01-mode-a-findings.md` | **F8-01…F8-10** — server Rust, each re-verified by the orchestrator at final HEAD |
| `02-satellites-supply-chain.md` | **S8-01…S8-12** — plugin, client, crates, edge tools, supply chain, CI/release |
| `03-mode-b-claims.md` | **R8-01…R8-03** — 24-row falsification table + the anti-vacuity sweep |
| `04-fork-audit.md` | **K8-01…K8-15** — fork hunks, hardening×wiring table, **rebase-survival**, auto-update verdict |
| `05-parity-matrix.md` | **P8-01, P8-02** — four-tree parity, rebuilt from scratch |
| `06-regulatory-matrix.md` | **L8-01…L8-11** — applicability matrix + standards currency + 11 named unverified items |
| `07-live-drill.md` | **§8 drill evidence** — scorecard, digest/replay, the DSAR defect, isolation proof |
| `08-design-grid-remediation-gates.md` | **D8-01…D8-10** design critique · coverage grid · R68–R72 releases · **gate results** · ceilings |

## Findings at a glance

| Series | Count | Highest severity |
|---|---|---|
| **F8** A-bugs (server) | 10 | **HIGH** ×4 (F8-01, F8-02, F8-03, F8-08) |
| **K8** fork | 15 | **HIGH** ×2 (K8-01, K8-02) |
| **S8** satellites/supply | 12 | **MEDIUM-HIGH** ×1 (S8-01) |
| **R8** falsified claims | 3 FALSE + 6 WEAKENED + 1 UNVERIFIABLE | **FALSE** ×3 |
| **D8** design | 10 | **Strategic** ×2 |
| **L8** regulatory | 11 | **HIGH** ×2 (L8-01, L8-02) |
| **P8** parity | 2 | **MEDIUM** (unowned property, no current drift) |
| **T8** docs-truth | 3 | folded into R8-01…03 |

## The three sentences

1. **The security model is sound; its machine checks under-deliver.** Three of the top ten are
   *guards that pass while their subject is violated* — proven by execution, not by reading.
2. **The mantras hold under test** — read seam, digest-binding, replay-safety, and revocation reach
   all survived direct attack in a live drill — **except erasure**, where a drill-proven gap lets a
   `completed` certificate stand over a surviving proposal body (**F8-08**).
3. **The docs are more honest than the numbers in them.** This pass found more documentation-truth
   defects than security defects, and **zero genuinely vacuous pins** — the anti-vacuity discipline
   in this repo works, and the `exec_spawn_carries_kill_on_drop` lesson was learned.

## Gates — all executed, all green

Full suite **3122 passed / 0 failed / 3 ignored** · clippy bench/default/otel exit 0 · fmt + client
fmt exit 0 · crates **308/0** · harness **44/0** · lipstyk **exit 0** (verified against a real code
base after correctly refusing to pass vacuously on docs-only commits) · badges selfcheck · env truth ·
doc links (404 resolve) · docs-truth · `cargo audit` **exit 0 on all four lockfiles**.

## The honest caveat

**§5.1's "test that fails on deletion" cells were derived by reading tests, not by deleting the
hardening and running the test** — the fork leg's terminal wedged before it could. That is the
largest single gap in this report. Eleven regulatory items are unverified with named next checks,
including the CRA Art 14 clocks — the repo's most-cited legal claim. Full list in
`08-…-gates.md` §9.5.

**No live surface was touched:** `~/.openclaw/workspace/brain.db` is SHA-256 identical before and
after (`39249cd9…`), the drill ran on port 18765 with a fresh DB, and no file in either repository
was modified.