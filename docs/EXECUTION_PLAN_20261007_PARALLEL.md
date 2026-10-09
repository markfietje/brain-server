# Parallel programme execution plan — 2026-10-07 (historical record)

**Status:** sealed history. This is the plan `AGENTS.md` cites for the R82/R83
parallel programme, reconstructed here because the reference had no file.
Nothing below is a live instruction; current-round state lives in the
local-only security-plans set, not in this tree.

## Lanes

- **L0 (`main`, this tree):** the lane that ships. Rounds land here.
- **L1 (worktree `brain-L1`):** developed R82 "Reach"; L0 shipped it.
- **L2 (worktree `brain-L2`):** developed R83 "Grammar"; L0 shipped it.
- **Fork lane (`openclaw` fork, parked):** under the operator's zero-conflict
  directive no fork-owned file may be edited in ways that could
  merge-conflict with upstream. Its round never shipped under a bare ID.

## Lane-qualified round ledger

The server lane keeps bare round IDs (history is immutable). Any parallel
lane suffixes its live IDs (`R<n>-<lane>`), so a parked lane can never
collide with a shipped server round. In particular the fork lane's planned
round is `R84-fork`, never bare `R84` — the bare ID belongs to the shipped
server round below.

| Live ID | Lane | Disposition |
|---|---|---|
| R82 | server · L1→L0 | shipped (1.29.4, Reach) |
| R83 | server · L2→L0 | shipped (1.29.4, Grammar) |
| R84 | server · L0 | shipped (1.29.4, Domains) |
| R84-fork | fork (parked) | parked under the zero-conflict directive; never shipped under the bare ID |

## Rule for future parallel work

A new parallel lane mints `R<n>-<lane>` identifiers from the start and
records them in its round ledger; the in-tree pin is the lane table above.
Bare `R<n>` IDs already in `CHANGELOG.md`, `AGENTS.md`, and the shipped-round
pin are history and are never rewritten to make room.
