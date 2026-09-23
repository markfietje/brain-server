# OWASP 2026 Compliance Matrix — brain-server (v1.27.12 "Agentic")

**Last reviewed:** 2026-09-09 against the two 2026 OWASP agentic frameworks
(rows below were first drawn up at v1.27.12; the dated addendum after Part 2
carries the deltas the v1.28.63–.76 hardening line shipped — the row statuses
stay, the addendum extends them).

| Framework | Edition | Published | Canonical source |
|---|---|---|---|
| **GenAI LLM Top 10 2026** | LLM01–LLM10 | 2026-08-04 | `GenAI-Security-Project/GenAI-LLM-Top10` `2026/final` |
| **Top 10 for Agentic Applications 2026** | ASI01–ASI10 | 2025-12-10 | OWASP Agentic Applications project |

This is the buyer/auditor artifact: every control carries a **status** — `Shipped
vX.Y` (with the exact feature), or `Ceiling v2.x` (a documented residual-risk
decision with an owner). The framework's own position (2026) is that **prompt
injection has no prevention** — there is no engineering fix (NIST 2025 / NCSC
2025 / Debenedetti et al. 2025 agree) — so this matrix's standard is **100%
control coverage**, not 100% risk elimination: every control has either a named
implementation or a documented, owned residual-risk decision. That is the
audit-ready form of "hardened."

Companion: `SECURITY.md` (ZT4AI posture, §), `COMPLIANCE.md` (§observability
playbook), `THREAT_MODEL.md`.

---

## Part 1 — OWASP GenAI LLM Top 10:2026 (LLM01–LLM10)

Ranking is incident-grounded (~10,000 real incidents; first edition, not expert
votes). LLM01's mitigation list is the load-bearing set for this stack
(least-privilege policy engine, invisible-char strip at every ingest+render
boundary, provenance-labeled channel, explicit human confirmation surfacing the
exact action, Rule of Two, memory writes as privileged operations, MCP/tool
supply-chain pinning). The 2026 MCP-defense literature converges on the same
shape: SHIELDMCP (ACL 2026 — per-run tool-description hashes, parameter
validation, response wrapping with instruction detection) matches the catalog
pins plus the single-block tool-result envelope; Arcjet's trusted-guidance vs
untrusted-evidence split matches the fence plus per-hit provenance; the
April-2026 MCP incident wave (Unit42 taxonomy, Microsoft XPIA advisory)
confirms sanitize plus classify as the current state of the art, which is
what the screen plus optional local classifier implements.

| LLM01–10:2026 | brain-server control | Status |
|---|---|---|
| **LLM01 Prompt Injection** | Every ingest write path screened (`screen()` — deterministic blocklist always on + optional feature-gated local ONNX classifier, v1.20.3); `untrusted`/quarantined segregation; per-hit provenance tags (`source`/`node_kind`/`lawful_basis`/`region`) rendered inside the `UNTRUSTED_*` fence with `sanitizeForBlock` — recalled content cannot forge its own attribution or the fence markers (v1.27.12); approval gate for autoCapture (v1.20.1); invisible-char strip at ingest + client render boundary | **Shipped v1.11+ / v1.20.1 / v1.20.3 / v1.27.12** |
| **LLM02 Sensitive Information Disclosure** | PII scan + `[redacted:…]` output masking + `pii:read` gate; record-level `access_scope`/`owner`; DSAR locate→export→purge→certificate + tombstone registry; read-event audit | **Shipped v1.14 + v1.15** |
| **LLM03 Excessive Agency** | AuthZ action matrix at every non-public handler (`authorize`, v1.12.1, test-pinned route-by-route); capability tokens verbs×scope (v1.17.3); per-action human approval for memory writes (Rule of Two, v1.20.1) | **Shipped v1.12.1 / v1.17.3 / v1.20.1** |
| **LLM04 Supply Chain** | CycloneDX SBOM ships with every release + CI `cargo audit` gate (v1.17.5); pinned deps + `.cargo/audit.toml`; UMP §2.8 integrity blocks (v1.17.3); MCP servers are first-party + HMAC/`webhook_seen` verified | **Shipped v1.17.5 / v1.17.3** |
| **LLM05 Data & Model Poisoning** | Quarantine + consolidate contradiction/near-dup detection (v1.8); supersession expiry (`valid_to`); `origin` provenance column (v1.18.2); **no fine-tuning** (fixed local embeddings) | **Shipped v1.14–v1.18.2** |
| **LLM06 Unbounded Consumption** | Rate limiter (v0.9.4+); capacity envelopes + `bench --envelope` ship gate (v0.9.9); recall `limit` clamped ≤100; bounded webhook queue + idempotency | **Shipped**; per-principal quotas = **Ceiling v2.x** (tenancy) — owner v2.0 Cortex |
| **LLM07 Misinformation** | Calibrated abstention (`/recall` `decision: low_confidence` on `ClarifyQuery`, v1.5) + `POST /verify` span check; evidence spans + `answer_in_context` (v1.4); `/consolidate` proposal review | **Shipped v1.4 + v1.5** |
| **LLM08 Hidden Context Exposure** | No route returns a system prompt / hidden context; principal pillar on every response; audit redacts content (hash-only invariant, test-pinned) | **Shipped v1.2 + v1.15** |
| **LLM09 Vector & Embedding Weaknesses** | vec0 cleaned on purge/DSAR; superseded chunks excluded at retrieval (`valid_to IS NULL`); quarantined excluded from KNN; near-dup scan over the live vec0 index (not legacy JSON) | **Shipped v1.14 + v1.8** |
| **LLM10 Improper Output Handling** | Strict typed JSON + `test_openapi_covers_routes` contract test; `/verify` span check; client never executes response bodies (`xss_escape_hatch_is_unused` grep gate); recall banner marks untrusted content | **Shipped v0.9.5–v1.16.x** |

## Part 2 — OWASP Top 10 for Agentic Applications:2026 (ASI01–ASI10)

Incident names OWASP cites: EchoLeak (goal hijack), Amazon Q (tool misuse),
GitHub MCP exploit (supply chain), AutoGPT RCE (code exec), Gemini memory attack
(memory poisoning), Replit meltdown (rogue agents).

| ASI01–10:2026 | brain-server / OpenClaw control | Status |
|---|---|---|
| **ASI01 Agent Goal Hijack** | Screen + classifier + `untrusted` stamp; recall banner ("may contain untrusted content") | **Shipped + v1.20.1/3** |
| **ASI02 Tool Misuse** | MCP tools are thin typed proxies over a validated API; per-route action matrix; no tool-description parsing of untrusted input | **Shipped** |
| **ASI03 Identity & Privilege Abuse** | JWT/JWS + revocation + refresh-chain reuse detection; per-handler AuthZ; tenant-scoped audit; capability tokens not grantable for admin | **Shipped v1.2–v1.17.3**; full multi-team tenancy = **Ceiling v2.x** (owner v2.0 Cortex) |
| **ASI04 Agentic Supply Chain** | First-party MCP only; plugin pinned by openclaw config; SBOM; UMP integrity; fork MCP catalog sha256-pinned per tool and reconciled every run, with fingerprint-moved tools hard-blocked until re-acknowledged and pin acks Ed25519-signed (v1.28.80) | **Shipped** |
| **ASI05 Unexpected Code Execution** | brain-server is a token validator — no eval path on the served surface; client render never executes bodies. The ONE exec seam is the loop-mediated `exec` path, now WIRED behind the typed sandbox seam (v1.28.92): deny-default `sandbox-exec` profiles on macOS, target-gated Landlock on Linux, fail-closed on unavailable backend, `Drop`-kills-and-reaps on every path out — on top of the v1.28.75 mediation (operator allowlist empty-absent = deny ALL engine exec, argv-only, cwd-pinned, caps, argv0 + allowlist-entry canonicalization, danger screen incl. pipe-to-shell) | **Shipped (architectural) + v1.28.75 (mediation) + v1.28.92 (OS boundary, unwired pin retired with the Loop landing)** |
| **ASI06 Memory & Context Poisoning** | **The core of this line**: screen (G1) + approval gate (G2) + classifier (G5) + quarantine + retention decay + cryptographic integrity (audit chain, UMP blocks) + provenance (`origin`) + optional two-principal quorum (v1.28.80) | **Shipped + v1.20.1–3** |
| **ASI07 Insecure Inter-Agent Communication** | HMAC webhooks + `webhook_seen` idempotency; **Standard Webhooks handshake (v1.20.4)**; UMP capability tokens | **Shipped + v1.20.4**; A2A federation = **Ceiling v2.x** (owner v2.0 Cortex) |
| **ASI08 Cascading Failures** | Proposal TTL auto-reject + expiry audit (v1.20.1); bounded webhook queue + idempotency; per-row batch outcomes; failure isolation in DSAR/consolidate | **Shipped + v1.20.1** |
| **ASI09 Human-Agent Trust Exploitation** | Review panel surfaces **exact content + `source_prompt`** (never a summary); approval TTL; **digest-bound approval** — the approve call carries the SHA-256 of the read-canonical form and is rejected on any drift (v1.27.12), so a rubber-stamped decision can never bless modified content; optional second-approver quorum (v1.28.80); audit trail of every gate decision | **Shipped v1.20.1 / v1.27.12** |
| **ASI10 Rogue Agents** | A compromised agent can only write via screened + gated paths; revocation; read-event audit; DSAR purge = eject-and-forget | **Shipped + v1.20.1** |

### Dated addendum — 2026-09-23 (the DecisionModel seam, v1.32.10)

The decision-harness seam landed as types + tests only (the `DecisionModel`
trait in the always-on SDK `decision` module; the kernel's
`workflow::harness` consumes it with a deterministic reference model and
the decide adapter). No routes, no state change, no learned models — the
v1.32.8 gate is untouched. The type-level controls:

| Control | The type-level law |
|---|---|
| **ASI03 Identity & Privilege Abuse** | A model evaluates inside the caller's already-authorized context: `DecisionContext` reaches the model by shared reference only, so a model cannot widen its own role scope or escalate — pinned by a type-level test |
| **ASI04 Agentic Supply Chain / LLM04** | Model identity is id + version + kind, and a LEARNED model cannot be constructed without its weights digest (`ModelKind::Learned` carries the digest structurally — un-digestable learned models are unrepresentable); deterministic models carry none |
| **ASI05 Unexpected Code Execution** | The seam is pure evaluation: no I/O, no process spawn, no dynamic loading, no clock; `unsafe_code = "forbid"` crate-wide in the SDK, and evaluation returns results or honest refusals — never panics |
| **ASI10 Rogue Agents / LLM06 Excessive Agency** | The monotonic-narrow authority law — **a DecisionModel proposes; only the gate disposes** — pinned at the type level: `&self` receivers and plain-data seam types (`Send + Sync + 'static`, no durable-state handles), so a model's output alone cannot mutate durable state |
| **ASI01/ASI06 (pre-wiring)** | Evidence enters the seam as PROVENANCE REFS only (ids + closed trust tiers); raw text is unrepresentable, and a ref without provenance (an empty id) qualifies as nothing |

### Dated addendum — 2026-09-23 (the Decision Harness engine, v1.32.11 part 1 — engine only)

The harness's deterministic pipeline engine landed as code + tests only
(the config document with canonical hashing, the pure stage runner with
per-stage provenance records, the additive run-trace table + session-log
kinds' writer). No routes, no learned models, no inference — the
v1.32.8 gate is untouched. The engine-level controls:

| Control | The engine-level law |
|---|---|
| **ASI02 Tool Misuse** | The stages are internal pure functions of a typed input and a validated config document — the harness is not an agent tool and not an MCP tool; nothing can invoke a single stage from outside, and (no routes this round) nothing external reaches the engine at all yet |
| **ASI04 Agentic Supply Chain / LLM04** | The pipeline config is digest-pinned by construction: the config hash is the sha256 of the canonical re-serialization of the LOADED document, every stage record carries it, and the model binding is a config-digest pair the model itself verifies at evaluation. No network-sourced configs exist on this path |
| **ASI05 Unexpected Code Execution** | Every stage is a pure function — no eval, no dynamic loading, no unsafe, no I/O in the stage path; the loader is total (a hostile config refuses by name, never partially loads, never panics) |
| **ASI07 Cascading Agents** | No self-invocation: stages are pure functions called once each by a linear runner over a config-declared, bound-checked list — structurally recursion-free; there is no agent-to-agent channel, and escalation is the human path |
| **ASI08 Resource Exhaustion** | Bounds live at the config validator: the stage list is capped at the architecture's fixed eight with duplicate/unknown/out-of-order refusals, and retrieval limits are clamped at 100 (the existing search-side over-fetch law) |
| **ASI10 Rogue Agents / LLM06** | Monotonic-narrow end to end: a stage refusal folds into a typed escalation record — the engine writes no durable state, and the only persistence is the trace artifact itself (digests and refs). Outputs propose; only the human gate disposes |

The AI-law note (code vs legal, deliberately not overstated): automatic
per-run traces with immutable references — input/context digests, config
hash, model digests, retrieval parameters, a compile-time environment
fingerprint — are the TECHNICAL LOGGING CAPABILITY behind EU AI Act
Art. 12(1)-style automatic event recording (high-risk obligations
generally apply from 2026-08-02; the deployer-side retention duty, e.g.
the six-month minimum, is the deployer's, not the software's) and keep
GDPR Art. 22-style transparency consistent (a decision trace an operator
can replay and inspect). This round ships the capability and its tests;
it makes NO legal conclusion — whether any given deployment is in scope
of those regimes is the operator's determination with counsel.

## Part 3 — AIUC-1 crosswalk (procurement bridge)

A crosswalk maps ASI01–ASI10 to the AI-Under-Contract (AIUC-1) requirements so
procurement can bridge the OWASP agentic list to a contractual requirement set
instead of maintaining two separate controls. The crosswalk is directional:
each ASI control satisfies the AIUC-1 requirement it names; the reverse mapping
is not claimed. Deployers drafting a contract can cite the ASI rows above as the
control-evidence for the corresponding AIUC-1 clause.

## Part 4 — Residual risk (the "100%" answer, named with owners)

These are the honest ceilings every control list converges on. Each is a
documented residual-risk decision with an owner, not an omission.

| Item | Why it stays open | Owner |
|---|---|---|
| **LLM01 has no prevention** | OWASP 2026's own position: no engineering fix exists. The screen + classifier degrade against adaptive attackers; the load-bearing defenses are architectural (segregation, gates, least-privilege) | Ops (retrain classifier; re-run adaptive evals per threat-model change) |
| **Adaptive white-box classifier evasion (GCG-class)** | ~100% adaptive ASR for ModernBERT-class encoders in 2026 research — beats any hardened encoder. The `untrusted` segregation + approval gate are the surviving controls | Platform (v1.21+ re-evaluation) |
| **Per-principal consumption quotas (LLM06)** | Tenancy work | v2.0 "Cortex" |
| **At-rest encryption (LLM02)** | LUKS/FileVault documented posture; SQLCipher = v2.x | v2.0 "Cortex" |
| **mTLS for webhook receivers (ASI07)** | Operator option today; A2A-bound later | v2.0 "Cortex" |
| **Full multi-team tenancy + SSO (ASI03)** | Consumes the v1.2 AuthN/AuthZ foundation | v2.0 "Cortex" |
| **A2A federation / remote agent identity (ASI07)** | The first-party Standard Webhooks handshake (v1.20.4) is the 2026-compliant boundary until then | v2.0 "Cortex" |

**Bottom line.** "100% hardened" = **100% control coverage**, not 100% risk
elimination. The residual-risk section is the truthful statement an auditor can
sign.
