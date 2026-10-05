# OWASP 2026 Compliance Matrix — brain-server (v1.27.12 "Agentic")

**Last reviewed:** 2026-09-09 against the two 2026 OWASP agentic frameworks
(rows below were first drawn up at v1.27.12; the dated addendum after Part 2
carries the deltas the v1.28.63–.76 hardening line shipped — the row statuses
stay, the addendum extends them).

| Framework | Edition | Published | Canonical source |
|---|---|---|---|
| **GenAI LLM Top 10 2026** | LLM01–LLM10 | 2026-08-04 | `GenAI-Security-Project/GenAI-LLM-Top10` `2026/final` (DOI `10.5281/zenodo.22109015`) |
| **Top 10 for Agentic Applications 2026** | ASI01–ASI10 | 2025-12-09 | OWASP Agentic Applications project |

> **Provenance of the two dates above, stated because they are hand-typed.**
>
> - **2025-12-09** for the Agentic edition is a REPO-INTERNAL RECONCILIATION, not a
>   publisher-verified fact: this file previously said `2025-12-10` while
>   `COMPLIANCE.md` and `docs/MEMGHOST_MITIGATION.md` both said `2025-12-09`. The
>   majority and the audit agree on the 9th, so the odd file was corrected to match.
>   The publisher page is **not reachable from a build**, so this is the best
>   available reading and is labelled as such rather than asserted as verified.
> - **2026-08-04** for the LLM edition is left **unchanged** deliberately. Seven
>   sources in this repo carry it, backed by a live fetch recorded at
>   `docs/SECURITY_AUDIT_20260912_FOURTH_PASS.md:119` ("REAL and EXACT", with the
>   DOI above). An audit leg proposed 2026-08-03 with no source in the tree; a
>   DOI-backed claim is not swapped for an unsourced one. Resolving it needs a
>   fetch against the publisher, not a repository edit.

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

### Dated addendum — 2026-09-24 (the Decision Harness surfaces, v1.32.11 part 2 — routes + enforcement)

The harness's public-safe route face landed: execute a run, read its
stored trace, replay it under its own recorded conditions, and the
DPO-gated listing — plus the gate-side enforcement of the mode law (an
exploratory run can propose, never promote). The pipeline semantics stay
the line's private core; what is public here is route EXISTENCE and the
authz posture. The surface-level controls:

| Control | The surface-level law |
|---|---|
| **ASI03 Identity & Credential Abuse** | Every route is role-gated (Write/Read plus the `workflow` capability) and no route is public; the listing — the exfiltration surface — is DUAL-gated (Admin action AND the DPO role) and audited per call; absent and foreign runs answer the SAME probe-blind 404 (no existence oracle) |
| **ASI09 Human Oversight & Transparency** | Traces render provenance honestly: per-stage algorithm labels, digests, trust tiers, and timing are the recorded record, and the replay report names per-stage agreement with BOTH digests. As with the κ bar, `all_match` is DATA for the operator's read — the value never auto-gates anything |
| **ASI10 Rogue Agents / LLM06** | The mode law is enforced at the GATE, not the proposal write: an exploratory run's proposal carries its provenance ref, is listable and reviewable, and is permanently promotion-incapable (`exploratory_mode_not_promotable`) — deterministic and human proposals approve unchanged; the gate remains the only disposer |
| **ASI02 Tool Misuse** | The routes expose exactly the four documented verbs over ONE validated config path — the config documents ride the request body (never the environment), the loader's total validation applies at the surface, and the model binding is digest-verified before any execution (`model_digest_mismatch`) |
| **ASI08 Resource Exhaustion** | The route layer adds no unbounded input: the config loader's bounds govern execution, the listing clamps 1..=50, the raw query is length-capped and screened, and a replay is one bounded re-execution of an already-bounded config |

The AI-law note, continued (code vs legal, no legal conclusions): the
trace READ surface — a bounded, audited, role-gated route returning the
stored trace document — is the ACCESS side of the same Art. 12-style
logging capability shipped earlier on this line (the high-risk
obligations regime generally applies from 2026-08-02 for in-scope
systems; deployer retention remains the deployer's duty, not the
software's). Shipping access control around log inspection is a
technical control; it makes NO legal conclusion about any deployment's
regulatory scope — that determination stays the operator's, with
counsel.

### Dated addendum — 2026-09-24 (the model registry: identity, lifecycle, and oversight)

The model registry is a technical control plane for model identity and
human disposition. It does not store weights or evaluation contents, and it
does not decide whether a particular deployment is legally in scope. The
rows below describe shipped code controls, not a legal conclusion.

| Control | The registry-level law |
|---|---|
| **ASI04 Agentic Supply Chain / LLM04** | A learned registration cannot omit its lowercase SHA-256 artifact digest; the request body is the only registration source, so no network-sourced identity is admitted. The canonical digest, identity, version, vocabulary, and lifecycle transition are the durable pins. The separate embedding-manifest digest law is not this registry. |
| **ASI03 Identity & Privilege Abuse** | Registration is an Admin-on-global operator action. The bulk listing is Admin plus the DPO role and audited per call. Promotion and retirement are proposal-plus-human-approval acts; no direct status-write route exists. |
| **ASI10 Rogue Agents / LLM06** | The human gate is the only lifecycle disposer. Deterministic execution resolves the bound `(config key, config digest)` through the registry and refuses unregistered, candidate-only, and retired bindings by name; exploratory execution accepts candidates but never unregistered or retired rows. |
| **ASI06 Memory & Context Poisoning** | Decision-layer identity, version, digest, and promotion are auditable records rather than free-form claims. A changed row refuses a previously reviewed lifecycle payload instead of silently applying stale intent. |
| **LLM02 Sensitive Information Disclosure** | The single-row view exposes identity, vocabulary, and digest references; listings expose only a digest-presence boolean. Weights and evaluation-set contents are not registry fields, and the listing is bounded and dual-gated. |

The human-approved lifecycle is an engineering analogue to an
oversight-and-recordkeeping pattern, not a claim that a registry satisfies
any particular legal provision. The EU AI Act's official text describes
Article 12 automatic event recording and Article 14 human oversight, and
states a general application date of 2 August 2026 (with specified
provisions applying earlier); those dates and obligations are a legal
source, not a classification of this software. Whether a deployment is
high-risk, who is the provider/deployer, and what retention or records
apply remain operator-and-counsel determinations.

### Dated addendum — 2026-09-24 (decision evaluation records, schema 1.32.14)

R30 adds a bounded decision-evaluation record, not an authoritative judgment
oracle. The only accepted source in this round is an operator-declared,
non-authoritative manifest whose case and manifest digests are checked against
persisted decision traces. QC/GDL gold packs are not decision labels, and
missing metric legs are not represented as zero. The controls below describe
shipped technical behavior only.

| Control | The evaluation-record law |
|---|---|
| **ASI03 Identity & Privilege Abuse** | Evaluation creation and listing are Admin-on-global plus the existing DPO role; detail uses the same conservative confidential posture, authorizes before lookup, audits found reads, and returns the same probe-blind 404 shape for absent records. No new `eval` capability or direct registry-status route is introduced. |
| **ASI04 Agentic Supply Chain / LLM04** | The target binds pipeline version, config hash, registry id/version/digest, and a learned artifact digest when applicable. Manifest and per-case labels are canonical SHA-256 commitments; raw model bytes, weights, and network-fetched artifacts are not evaluation inputs. |
| **ASI06 Memory & Context Poisoning** | Every case cites a persisted decision-trace id and bounded evidence ids, and the target is checked against that trace's model citation, pipeline, and config. Operator-declared data is explicitly non-authoritative; no gold-pack relabeling or training/labeling-pool write occurs. |
| **ASI08 Resource Exhaustion** | Case count, evidence-id count, string/digest/timestamp bounds, serialized manifest/report limits, and the 1..=50 listing page are checked before durable work. Database work is bounded and isolated behind `spawn_blocking`; no unbounded environment or corpus scan is exposed. |
| **ASI09 Human-Agent Trust Exploitation** | Acceptance bars are serialized as `reported_as_data_only`; the report names unavailable legs and their reasons, and no bar changes registry status. The checked audit row binds the canonical record digest, but it is not described as a detached cryptographic signature. |
| **ASI10 Rogue Agents / LLM06** | Evaluation records cannot write knowledge, advance a workflow, attach a registry reference, or promote/retire a model. The existing human gate remains the only lifecycle disposer; `evaluation_refs` stays fail-closed until a verified accepted producer exists. |
| **LLM02 Sensitive Information Disclosure** | The durable manifest/report contains bounded identifiers, closed labels, digests, and aggregate metrics only. Raw queries, raw evidence text, model weights, secrets, and unrestricted free text have no field in the record or emitted JSON; listings omit the full report. |

The technical audit receipt and evaluation record are evidence mechanisms, not
legal conclusions. The official EU AI Act text describes logging and human
oversight obligations in its own scope; NIST AI 600-1 and the OWASP Agentic
Security Initiative provide risk/security guidance. They do not determine
whether a deployment is high-risk, who is provider versus deployer, or which
retention, documentation, lawful-basis, consent, or jurisdiction-specific duties
apply. Those remain operator-and-counsel decisions.

### Dated addendum — 2026-09-25 (Shell M6-S0 contract integrity)

R31 hardens the shell boundary without adding a product surface or changing the
kernel wire contract. The generated TypeScript client is synchronized only by
an explicit generation step; ordinary tests and builds are non-mutating, and CI
byte-compares a temporary regeneration against the committed output. The
render-boundary sanitizer uses the same closed invisible-scalar membership as
the canonical fixture and server Rust implementation, with exhaustive tests,
visible-sample preservation, and idempotence. The shell workflow is triggered
by contract/fixture changes, grants read-only repository permission, pins
external actions, installs the declared Linux/Tauri/Rust prerequisites, and
runs the real dedicated-port Chromium and WebKit e2e suite without bypassing
WebKit CSP.

| Control | The shell technical-control law |
|---|---|
| **LLM04 Supply Chain** | OpenAPI-derived types, frozen pnpm installation, immutable CI action references, pinned `cargo-audit` 0.22.2, and a generated-file diff prevent a clean-runner contract or dependency drift from being silently accepted. |
| **LLM01/ASI06 Injection and context integrity** | Renderer-facing text crosses one exact canonical invisible-scalar boundary before display; the helper is pure and idempotent, preserves visible samples, and is not an HTML sanitizer. The existing `{@html}` lint ban remains in force. |
| **ASI08 Resource exhaustion / CI integrity** | Shell CI has a bounded timeout, one Playwright worker, explicit browser dependencies, and a fail-closed audit/install sequence; generated output is checked rather than regenerated by tests. |

For later M6 work, R30's evaluation record is a checked audit acceptance over
an explicitly operator-declared, non-authoritative manifest. It has no generic
evaluation signature field and no separate judgment registry. This entry
records technical controls and dated source context only; it makes no legal,
compliance, release, or public-publication determination. Provider/deployer
status, regulatory scope, retention, documentation, and jurisdiction-specific
duties remain operator-and-counsel decisions.

### Dated addendum — 2026-09-25 (R33 — registry lifecycle contract integrity)

R33 records the technical controls around the existing registry row and the
`registry_lifecycle` proposal contract. This addendum makes no legal or
compliance claim.

| Control | The R33 technical-control law |
|---|---|
| **ASI04 Agentic Supply Chain / LLM04** | The single-row detail response issues a server-computed `row_digest` as lowercase 64-hex SHA-256 over the canonical compact `RegistryRow` serialization. A lifecycle proposal reuses that server-issued digest and the exact current row; creation and human approval recheck both, so any drift fails closed. |
| **ASI06 Memory & Context Poisoning** | This is the proposal-only agency boundary: the `registry_lifecycle` proposal's content is the exact serialized `{action,id,version,row_digest,row}` shape, only `promote\|retire` are legal, and creation makes no status/knowledge change. It cannot become a knowledge node or dispose itself. |
| **ASI09 Human-Agent Trust Exploitation** | `promote` and `retire` are human-approval-only lifecycle actions. The existing human gate is the only human disposal path; no autonomous status transition is accepted. |
| **ASI10 Rogue Agents / LLM06** | The digest is an integrity binding, not a generic signature, and does not make `evaluated` reachable. Non-empty `evaluation_refs` is refused; the registry stores no weights or evaluation contents. |
| **LLM02 Sensitive Information Disclosure** | The lifecycle contract carries the exact registry row and digest references, never model weights or evaluation contents. |

### Dated addendum — 2026-09-25 (R34 — GDL provider boundary hardening)

R34 records technical controls at the existing GDL launch seam. It is not a
certification, legal conclusion, conformity claim, or risk-elimination claim.

| Control | The R34 technical-control law |
|---|---|
| **ASI03 Identity & Privilege Abuse** | The public GDL request carries only the ticket. Provider destination, model, secret file, and secret root are server-owned configuration. A JWT must pass actual-domain Write authorization and a GDL-local `workflow` capability check before profile resolution, secret access, DNS, or provider construction; role-less JWTs and unknown roles fail closed. `AgentLoopback` remains refused before run lookup. |
| **ASI07 Insecure Inter-Agent Communication** | Production provider construction requires HTTPS, rejects userinfo/fragments/queries/unsafe URL shapes, retains resolved-address validation and DNS pinning, and refuses redirects. The test-only loopback adapter remains isolated from production construction. |
| **ASI08 Cascading Failures** | Provider transport, status, stream, response-cap, and parser failures are bounded and mapped to stable operator-safe codes. Raw provider bodies, malformed payloads, bearer values, secret-bearing URLs, and filesystem paths do not cross the public error seam. |
| **LLM02 Sensitive Information Disclosure** | The configured secret is read only after authorization/configuration checks, confined beneath the configured root, rejected for symlink/empty/multiline/control/oversized content, and never persisted or logged. GDL audit details contain no provider body or secret. |
| **ASI09 Human-Agent Trust Exploitation** | Existing GDL deny-all execution, empty tool registry, bounded streaming, pending capture, and human-review disposition laws remain unchanged. Provider availability does not create an autonomous publication path. |

### Dated addendum — 2026-09-25 (R35 — GDL launch execution integrity)

R35 records technical controls around the existing GDL execution and settlement
seams. This addendum is dated engineering and risk context only. It is not a
certification, legal conclusion, conformity claim, high-risk classification,
risk-elimination claim, or release/publication decision.

| Control | The R35 technical-control law |
|---|---|
| **ASI03 Identity & Privilege Abuse** | The public `workflow-operator` role is the least-privilege supported JWT path to the GDL launch boundary. The `agent` preset remains without `workflow`; role-less and unknown-role JWTs fail closed before profile, secret, DNS, or provider work. |
| **ASI07 Insecure Inter-Agent Communication** | The provider request has a 25-second total request/body deadline. A slow-drip body cannot extend that deadline, and dropping the stream receiver drops the in-flight HTTP future rather than leaving detached work. The existing endpoint screen, DNS pinning, HTTPS requirement, and redirect refusal remain in force. |
| **ASI08 Cascading Failures** | Provider failure is a closed typed class. After admission, the exchange receipt, invocation completion, terminal GDL checkpoint, fixed audit detail, and claim release use the existing transaction/checkpoint seams. The terminal is non-retryable on the same run and exposes only stable operator-safe codes. |
| **LLM02 Sensitive Information Disclosure** | Provider bodies, malformed payloads, bearer values, secret paths, and secret-bearing URLs do not cross the typed error, response, or audit seam. The four-variable profile reports only `disabled`, `configured`, or `invalid`; partial/invalid configuration refuses boot or reports `NOT_READY`. |
| **ASI09 Human-Agent Trust Exploitation** | Provider failure does not create a publication path or a recovery action. Existing deny-all execution, empty tools, human review, and one-episode-per-run boundaries remain unchanged. |

The dated standards and legal materials used for R35 are engineering context.
They do not determine provider/deployer status, regulatory scope, conformity,
or any jurisdiction-specific obligation. Those remain operator-and-counsel
decisions.

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
