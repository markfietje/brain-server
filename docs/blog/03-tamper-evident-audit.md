# Tamper-evident audit: why your memory store needs a hash chain

*2026. The control that turns "trust us" into "verify it."*

Most systems that call themselves auditable actually ship the weak version:
they append log lines. Appending is not auditing. If the store is compromised,
an attacker — or a bug, or a tired admin running the wrong `DELETE` — can edit
the log to look like nothing happened. Appending gives you a *record*. A hash
chain gives you **tamper-evidence**: proof that the record wasn't altered
after it was written.

## The mechanism

Every audit row is chained to the previous one:

```
row[0] = hash(payload[0])
row[n] = hash(row[n-1] || payload[n])
```

Change any row and every subsequent `prev_hash` disagrees. The chain is
self-authenticating: you don't need to trust a server process to vouch for the
log, you need one function (`GET /audit/verify`) that walks the whole chain and
recomputes every link. It answers, in O(n): **has this ledger been tampered
with, at any point, ever?** And it holds across database migrations — a subtle
bug where migrated rows had a NULL backref was caught and fixed, with a test
that would fail on the buggy version.

The chain records **decisions**, not just actions: write-gate approvals and
rejects, DSAR purges, quarantine verdicts, and (opt-in) even reads — so a
reviewer can replay *what the agent knew, when, and who approved it*. That is
the "audit-ready replay" the 2026 bar demands.

## Why it's the load-bearing compliance control

- **DSAR + deletion certificate:** when a subject requests deletion, the system
  locates → exports → purges → records a **chain-verifiable certificate**. A
  deletion you can prove happened is a deletion a regulator accepts; one you
  merely claim is a promise.
- **EU AI Act Art 50:** the transparency notice (`/.well-known/ai-notice`) is a
  documented, origin-annotated posture — and `origin` (`human`/`model`/`imported`)
  provenance on every row means the "where did this come from" question has a
  stored answer, not a guess.
- **SOC 2 / vendor assessment:** the proof map (`docs/trust/proof-map.md`) gives
  a reviewer the exact command to verify each claim live — `curl
  localhost:8765/audit/verify` → `{"ok":true}`. A store you can't verify is a
  store you shouldn't trust.

## The honest limits

The chain proves the log wasn't tampered with **after** a row was written; it
does not magically make the *first* write truthful. The human gate (previous
post) is what decides what deserves to be in the chain in the first place. And
the chain is single-process today — distributed audit across many instances is
a documented future ceiling, not a claim.

**The takeaway:** if you're going to be held to "show me what the agent knew and
who approved it," don't ship append-only. Ship a chain a reviewer can verify
with one command — and be able to prove a deletion happened, not just claim it.

*See [`docs/trust/proof-map.md`](../trust/proof-map.md) and the bi-temporal
explainer for how validity + chain together answer "what was true at time T?"*
