# OWASP 2026 Compliance Matrix — brain-server (v1.20.5 "Agentic")

**Last reviewed:** 2026-08-11 against the two 2026 OWASP agentic frameworks.

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
supply-chain pinning).

| LLM01–10:2026 | brain-server control | Status |
|---|---|---|
| **LLM01 Prompt Injection** | Every ingest write path screened (`screen()` — deterministic blocklist always on + optional feature-gated local ONNX classifier, v1.20.3); `untrusted`/quarantined segregation; provenance-labeled recall banner (plugin marks untrusted content); approval gate for autoCapture (v1.20.1); invisible-char strip at ingest + client render boundary | **Shipped v1.11+ / v1.20.1 / v1.20.3** |
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
| **ASI04 Agentic Supply Chain** | First-party MCP only; plugin pinned by openclaw config; SBOM; UMP integrity | **Shipped** |
| **ASI05 Unexpected Code Execution** | brain-server is a token validator — no eval path; client render never executes bodies | **Shipped (architectural)** |
| **ASI06 Memory & Context Poisoning** | **The core of this line**: screen (G1) + approval gate (G2) + classifier (G5) + quarantine + retention decay + cryptographic integrity (audit chain, UMP blocks) + provenance (`origin`) | **Shipped + v1.20.1–3** |
| **ASI07 Insecure Inter-Agent Communication** | HMAC webhooks + `webhook_seen` idempotency; **Standard Webhooks handshake (v1.20.4)**; UMP capability tokens | **Shipped + v1.20.4**; A2A federation = **Ceiling v2.x** (owner v2.0 Cortex) |
| **ASI08 Cascading Failures** | Proposal TTL auto-reject + expiry audit (v1.20.1); bounded webhook queue + idempotency; per-row batch outcomes; failure isolation in DSAR/consolidate | **Shipped + v1.20.1** |
| **ASI09 Human-Agent Trust Exploitation** | Review panel surfaces **exact content + `source_prompt`** (never a summary); approval TTL; audit trail of every gate decision | **Shipped v1.20.1** |
| **ASI10 Rogue Agents** | A compromised agent can only write via screened + gated paths; revocation; read-event audit; DSAR purge = eject-and-forget | **Shipped + v1.20.1** |

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
