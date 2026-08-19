# OWASP 2026: our control matrix is the sales doc

*2026. When the security frameworks catch up to agentic systems, have the map ready.*

2026 brought two agentic-security frameworks that finally named the threats
people have been feeling:

- **OWASP GenAI LLM Top 10: 2026** (LLM01–10) — incident-grounded, includes
  prompt injection, model denial of service, sensitive-info disclosure,
  insecure output handling.
- **OWASP Top 10 for Agentic Applications 2026** (ASI01–10) — prompt injection
  on agent pipelines, broken access control, data integrity, delegation abuse,
  authorization confusion.

"Let's buy something that handles OWASP 2026" is becoming a procurement line
item. When that happens, the winner is whoever can map their system to the
matrix *honestly* — control by control — not whoever has the best marketing
page.

## We wrote the matrix before anyone asked for it

`docs/OWASP_AGENTIC_2026.md` maps every control, row by row, to either a
**shipped feature** or an **owned residual-risk ceiling**. Not a claim of "100%
hardened" — a statement of **100% control coverage**: every control has a named
answer, and the ones we can't fully eliminate (LLM01 prompt injection has no
prevention per OWASP 2026 itself) are segregated, gated, and least-privileged
into survivability.

Concrete rows, each verifiable live via the proof map:

- **LLM01 / ASI01 prompt injection** → the two-layer injection screen
  (deterministic blocklist + optional local classifier) + the `flagged`/
  `untrusted` segregation + the human approval gate. Reads never execute body;
  writes are gated outside the prompt.
- **ASI03 authorization** → deny-by-default JWT/JWS AuthZ, capability tokens,
  per-tenant audit scoping, Standard Webhooks signed-timestamp verification.
- **Sensitive-info disclosure** → PII output redaction + opt-in write-time
  placeholder mode + the `/health` content-leak fix (a real CVE class, fixed).
- **Supply chain** → CycloneDX SBOM on every tagged release (EU CRA / OWASP
  A03:2025).
- **Auditability** → the tamper-evident chain (previous post) + DSAR deletion
  certificates + the audit-ready-replay playbook.

The columns are grounded in the actual code — `src/screen.rs`, `src/gate.rs`,
`src/auth/`, `src/audit.rs` — and the rows carry the release that shipped them,
so the doc can't drift into fiction.

## The honest ceiling (this is the part that matters)

We state plainly the controls we **do not** claim: at-rest encryption, mTLS,
A2A federation, OIDC authorization-code, multi-team tenancy — these are owned
v2.x ceilings with named owners in the matrix. The OWASP 2026 standard is
**100% control coverage, not 100% risk elimination**; LLM01 has no prevention,
and a GCG-class adaptive attack can still beat a hardened encoder. What
survives that is segregation + gates + least privilege — which is why those are
the load-bearing controls, and why the matrix says so.

**The takeaway:** when a buyer (or an auditor, or your own CISO) asks "how do
you handle OWASP 2026?", don't improvise and don't overclaim. Ship a control
matrix where every row is a shipped feature or a named ceiling — and a proof map
that verifies the claims live. The document that's honest about its limits is
the one that wins the review.

*See [`OWASP_AGENTIC_2026.md`](../OWASP_AGENTIC_2026.md) and the
[proof map](../trust/proof-map.md).*
