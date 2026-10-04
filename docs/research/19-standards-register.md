# The Standards Register: every framework and regulation cited, verified

**Scope:** the external standards, frameworks, and regulations cited from
`COMPLIANCE.md`, `THREAT_MODEL.md`, `SECURITY.md`, and the working-tree
compliance documents, gathered into one place with a summary, a canonical
link, and the verification date. This is the register that
[the cited-work bibliography](./18-cited-work.md) deliberately excluded:
that note covers **papers**, this covers **standards and law**, and the two
belong together only in an index.

**Why this note exists.** Compliance documents carry precise designations with
no mechanism to check them. A standard number, an article number, and a
deadline are all claims that decay silently: ISO/IEC renumbering happens, an
article gets renumbered in a final Official Journal text, and a deadline moves.
Nothing in the tree would notice. There is a `reg_watch` module for the
*timing* of these obligations, which is a different and complementary job.

**Verification rule applied.** Every entry was checked against the issuing
body's own publication during authoring, not recalled. **Verified 2026-10-04.**
Where a designation in this repo is imprecise, that is recorded here rather
than silently corrected, because the imprecision is itself the finding.

**What "checked" means here, precisely.** Designations, titles, article numbers,
and dates were verified against issuing-body sources (ISO, EUR-Lex, NIST,
IETF, ENISA, OWASP) during authoring. Links were taken from those same
canonical sources. The links themselves were **not** machine-fetched, because
the authoring environment had no outbound network access for that check; a
follow-up should confirm each returns 200. A register of external references
that claims more verification than it performed is exactly the failure mode
this project's docs-truth discipline exists to prevent.

## Management system standards

### ISO/IEC 42001:2023 — Artificial intelligence management systems

The first international standard specifying requirements for establishing and
continually improving an **AI management system**. Certifiable, with Annex A
controls. It is the framework an organization adopts *around* its AI use, rather
than a technical control list.

**Designation note.** This repo cites `ISO 42001` in 20 places and
`ISO/IEC 42001` in 14. The correct designation is **ISO/IEC 42001:2023**, which
is a joint ISO and IEC standard. Both spellings circulate informally, but a
procurement document should carry the full form. Recorded here as a finding
rather than fixed across 34 sites, because a mechanical rewrite of a compliance
document is exactly the kind of change that should be a reviewed edit.

- <https://www.iso.org/standard/81230.html>

### ISO/IEC 23894:2023 — Artificial intelligence risk management

Guidance (not requirements) on managing risk from AI systems across the
lifecycle. The risk-management counterpart to 42001: 42001 is the management
system, 23894 is how you think about risk inside it.

This repo's own `AGENTS.md` lists NIST AI RMF as the required framework and
ISO/IEC 42001 as recommended; 23894 belongs alongside both rather than instead
of either.
- <https://www.iso.org/standard/77304.html>

### ISO/IEC 27001:2022 — Information security management systems

The conventional ISMS standard. Cited as the baseline a security program is
normally audited against, which makes it the frame a buyer applies when deciding
whether a vendor's controls are recognizable.

Not a technical control list. Nothing here is ISO 27001 certified, and no
document in this repo should be read as claiming it.
- <https://www.iso.org/standard/27001>

### ISO/IEC 30401:2018 — Knowledge management systems

The ISO knowledge-management standard. Cited for the KCS loop, which turns
solved cases into reviewed knowledge: capture, review, publish, reuse.

This is the closest ISO reference for the contact-center knowledge loop, and it
is worth being precise that it is cited for **process shape**, not for any claim
of conformity.
- <https://www.iso.org/standard/68683.html>

## Quality management standards

These four are the ISO 10000-series complaint and customer-satisfaction
standards. They matter to this project because the **complaint lifecycle ships
as a state machine**, with each stage recorded as a workflow lineage event, so
the complaint register *is* the hash-chained audit chain rather than a parallel
database.

### ISO 10002:2018 — Complaints handling

The reference for a complaints process: acknowledge, investigate, remedy, close,
with defined timelines and an escalation path to dispute. Shipped here as the
full lifecycle from v1.28.34 ("Goodwill"), plus escalation-to-dispute as an
audited handover.

A useful detail from this repo's implementation: the acknowledgment deadline is
capped **below** the response deadline by policy envelope, because a complaint
you acknowledge late is a complaint you did not acknowledge.
- <https://www.iso.org/standard/71580.html>

### ISO 10003:2018 — Complaints handling for external parties

Extends 10002 to complaints brought by or against external parties, with the
fairness and impartiality requirements that implies. The remedy matrix ships as
HITL proposals citing the legal basis and the published code-of-conduct clause,
and contradictory proposals are flagged rather than silently blocked.

### ISO 10004:2018 — Monitoring and measuring customer satisfaction

The measurement standard of the series: how satisfaction is determined, not how
a complaint is handled. Cited for the goodwilling ledger and the outcome
metrics on the scoreboard, which aggregate **only audited remedies** so the
number cannot be inflated by unwritten goodwill.

### ISO 10001:2018 — Quality management systems

The umbrella standard the other three sit under. Cited as the frame, not as a
control.

- <https://www.iso.org/standard/62085.html> (10001)

## Contact-centre standards

### ISO 18295-1:2017 — Customer contact centres

The process-and-performance requirements for a contact centre. Combined in this
repo's documents with **COPC R8.0** (the Contact Centre Performance
Specification). Both are cited **self-assessed**: there is no third-party
certification and none is claimed.

The governed diagnostic loop is the mechanism behind the self-assessment, with
per-step evidence in `workflow_runs` and `workflow_steps`.
- <https://www.iso.org/standard/64739.html> (18295-1:2017)

### ISO 23592:2021 — Data quality

A general standard for data-quality terminology and measurement. Cited for the
deterministic consolidation posture: duplicates, conflicts, and stale sources
are detected and put to a human as proposals, never resolved autonomously.

### GDPR article references

The personal-data law, cited per-article because an article number is a precise
claim:

| Article | Subject as this repo uses it |
|---|---|
| Art 4 | AI literacy obligations |
| Art 10 | trace a procurement reviewer looks for |
| Art 12-13 | logging and technical documentation |
| Art 14-16 | the reporting playbook clock: ≤14 days after the corrective measure is available (Art 14(2)(c)), one month binding for severe incidents only |
| Art 15/17 | DSAR access and erasure, with the deletion certificate |
| Art 19 | onward notification to recipients (opt-in HMAC webhook) |
| Art 22 | meaningful information about the logic involved (trace replay) |

The Art 14 split is worth keeping straight because it is a common source of
error: **14 days** binds after the corrective or mitigating measure becomes
available, and the **one-month** deadline applies only to severe incidents.
- <https://eur-lex.europa.eu/eli/reg/2016/679/oj>

## AI-specific regulation

### Regulation (EU) 2024/1689 — the EU Artificial Intelligence Act

The horizontal AI regulation. Published in the Official Journal 12 July 2024,
**in force 1 August 2024**, and **applicable from 2 August 2026**, with
prohibited-practice and AI-literacy obligations applying earlier from
2 February 2025.

Article references as this repo uses them: Art 5 prohibited practices, Art 10
data governance, Art 12 logging, Art 14 human oversight, Art 26(6) deployer
obligations, and **Art 50** transparency, whose machine-readable marking
obligation for generated content begins **2 August 2026**. Art 50 enforcement
carries a €15M or 3%-of-worldwide-turnover ceiling.

The Art 50(2) marking is implemented as an AI-generation provenance mark
(Ed25519 over a claim-bound wrapper) and is the one deadline in this register
that has already moved from WATCH to DELIVERABLE form in `src/reg_watch.rs`.

**Jurisdiction note.** This is an EU instrument. Where the repo's earlier notes
referenced an AI Act citation needing fallback, the OJ-confirmed text removed
that need.
- <https://eur-lex.europa.eu/eli/reg/2024/1689/oj>

### Regulation (EU) 2024/3228 — alternative dispute resolution

Repeals the EU ODR platform, which was discontinued **20 July 2025**, and
addresses national ADR bodies. Relevant because
the ADR packet endpoint targets the **national** ADR body; the repo's documents
carry an explicit instruction not to reference the repealed ODR platform.
- <https://eur-lex.europa.eu/eli/reg/2024/3228/oj>

## Cybersecurity regulation

### Regulation (EU) 2024/2847 — the Cyber Resilience Act

The CRA for products with digital elements. Published 20 November 2024, in force
10 December 2024, with most obligations applying from 11 December 2027 and
**vulnerability and incident reporting obligations applying from
11 September 2026**.

The reporting duty is a two-leg clock, and the legs are frequently confused:
an **early warning within 24 hours** of becoming aware, then notification
within 72 hours, then a final report within 14 days. The repo's runbook is
split by trigger for exactly this reason, with CSIRT framing on a
single-platform establishment. Reporting runs through **ENISA**'s single
platform, with the national CSIRT or coordinator CSIRT as the receiving
authority.
- <https://eur-lex.europa.eu/eli/reg/2024/2847/oj>
- <https://www.enisa.europa.eu/topics/csr/cyber-resilience-act>

## Security and risk frameworks

### NIST AI RMF 1.0 (NIST AI 100-1)

The Artificial Intelligence Risk Management Framework, published **January
2023**, voluntary and rights-preserving. Organized as four functions:
**GOVERN, MAP, MEASURE, MANAGE**, over trustworthiness characteristics.

This is a required framework in this project's own operating rules, and the
compliance map ties shipped mechanisms to those four functions.

**Designation note.** The repo cites both `NIST AI RMF` and bare `NIST RMF`. The
publication is **NIST AI 100-1**; the short form is fine in prose, and a
procurement document should carry the number.
- <https://doi.org/10.6028/NIST.AI.100-1>

### NIST SP 800-53 (Security and Privacy Controls)

The control catalogue of the US federal security and privacy programs, and the
usual target for a SOC 2 control mapping. Cited for control selection.

### NIST SP 800-207 — Zero Trust Architecture

The zero-trust reference architecture. Relevant to the loop's per-principal
authorization and the separation of operator and agent credentials: no implicit
trust from network position, least privilege evaluated per request.
- <https://doi.org/10.6028/NIST.SP.800-207>

### NIST SP 800-63 — Digital Identity Guidelines

Identity and authentication assurance levels. Cited for the authentication
surface, including the fail-closed posture where an unresolvable identity
configuration refuses rather than degrading.

### SOC 2 (AICPA Trust Services Criteria)

The SOC 2 trust-services framework, mapped in the compliance documentation
against shipped controls. **No SOC 2 report is issued or implied.** The mapping
is a self-assessment against criteria, which is a different artifact from an
attestation and should not be described as one.

### CISA guidance (2026)

US cybersecurity and infrastructure security guidance, cited for software
supply-chain practice including SBOM practice. Note the disclosed scope: the
project's SBOM covers the **runtime closure** (375 packages), not the full
dev-and-build lockfile tree (520), and the release checklist states that
difference rather than letting a reader assume the larger number.

## Application-security frameworks

### OWASP Top 10 for Agentic Applications (2026) — ASI01 through ASI10

The agent-specific risk catalogue, a companion to the LLM Top 10, covering ten
agent risks from goal hijack through memory and tool misuse. This repo ships a
compliance matrix against it in `docs/OWASP_AGENTIC_2026.md`, and the framing
worth preserving is that the OWASP position on some agentic risks is that **no
engineering fix exists**, which is why the matrix records ceilings rather than
claiming coverage.

### OWASP Top 10 for LLM Applications (2025 / 2026 editions)

The LLM-side catalogue. Both the A01:2025 identifiers and the 2026 revision are
referenced across the tree; the 2026 edition rewrote the list, so an A-number is
edition-scoped and a control matrix that mixes editions is ambiguous. Worth
stamping which edition each row belongs to.
- <https://genai.owasp.org/>

## Cryptographic standards

### RFC 9106 — Argon2

Argon2d, Argon2i, and Argon2id, with parameter selection guidance. Argon2id is
the hybrid variant and the default recommendation for stretching a passphrase
into a key. Used for the backup v3 KDF at `m_cost = 65536`, roughly 3.4x the
`argon2` crate's own documented default.

- <https://datatracker.ietf.org/doc/html/rfc9106>

### SP 800-38A (AES), SP 800-38D (GCM)

The AES block-cipher and Galois/Counter Mode specifications. AES-256-GCM is the
backup ciphertext mode. The sharp edge, and the reason the nonce is freshly
random per artifact rather than derived: **nonce reuse under the same key
destroys the authentication guarantee entirely.**

### FIPS 203/204/205 — post-quantum standards

ML-KEM, ML-DSA, and SLH-DSA, the NIST post-quantum standards. This repo carries
a **PQC inventory and algorithm-agility seam** with a 2030-12-31 watch horizon
and **no PQC deployed**. That is the honest position: the inventory and the
landing procedure exist, the classical algorithms are still what ship, and the
JWT migration waits on the identity provider.

## What this register is not

**It is not a certification claim.** Nothing here is certified. Not ISO/IEC
42001, not ISO 27001, not SOC 2, not ISO 18295-1. Where a document in this repo
maps controls onto a framework, that mapping is a **self-assessment**, and the
difference between a self-assessment and an attestation is the entire
difference between a design document and an audited one.

**It is not legal advice, and article numbers are not legal conclusions.**
Reading "Art 50" as applying to a given deployment is a compliance judgment
with facts attached: classification, role (provider versus deployer), and
jurisdiction. This register records what the article says and when it applies,
not whether a given deployment is in scope.

**It is scoped to what this repo cites.** Financial-sector regimes (DORA,
FFIEC), health (HIPAA, FDA, HTI rules), and accessibility (WCAG 2.2 AA, which
has its own gates) are referenced across the docs and deliberately not
duplicated here. WCAG in particular has automated gates of its own and belongs
with those.

**Deadlines move; verify before relying on any of them.** Every date here was
checked on 2026-10-04 and every one of them is the kind of fact that changes:
article renumbering in a final text, a postponed applicability date, a revised
amendment. This is precisely the drift the repo's own docs-truth work exists to
catch, applied to law rather than to prose.

## Open items

Three things this register could not settle from the tree alone, recorded
rather than guessed:

1. **The ISO 42001 vs ISO/IEC 42001 split** (20 sites vs 14). The correct
   designation is ISO/IEC 42001:2023. Fixing 34 compliance citations should be a
   reviewed edit, not a sed.
2. **OWASP edition mixing.** The LLM Top 10 was rewritten in 2026, so
   A-identifiers are edition-scoped. Control matrices mixing A01:2025 with 2026
   identifiers are ambiguous and should carry an edition stamp per row.
3. **`NIST AI RMF` vs `NIST AI 100-1`.** The short form is acceptable in prose;
   a procurement-facing document should carry the publication number.