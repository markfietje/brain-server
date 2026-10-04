# §7 — REGULATORY + STANDARDS APPLICABILITY MATRIX

**Audit date: 2026-10-04.** Anchor: the repo's own `docs/US_STATE_MAP.md` (status 2026-09-14 —
**20 days stale**, and the brief's 2026-09-12 frame is itself two weeks old).

> **Verification posture, disclosed first.** Primary-source access was materially constrained this
> session: `eur-lex.europa.eu` returned an HTTP 202 JS-challenge with no text, `op.europa.eu` 403,
> `congress.gov` and NCSL Cloudflare-blocked, and the search quota was exhausted mid-leg.
> **Consequence: I could not read the CRA Art 14 text, any US statute text, or the Digital Omnibus
> text directly.** Where the repo asserts a legal claim about an instrument I could not open, it is
> marked **UNVERIFIED** with the next check named. Nothing was guessed, and no "watch" was upgraded
> to a "duty" on inference.

**Scope framing carried through every row.** As shipped, brain-server is a single-node,
loopback-first SQLite retrieval store with a Dioxus console + Tauri shell, a TS plugin, channel
bridges, and **no training, no model hosting, no consequential decision-making, no autonomous
content generation** (`COMPLIANCE.md:700`; `docs/compliance.md:32-35`). That drives most rows to
**DEPLOYER** duty. **The repo's component/deployer distinction is unusually disciplined and, in my
judgement, correct** — it is what made this matrix tractable.

---

## 7.1 EU — AI Act (Reg (EU) 2024/1689, as amended by 2026/1744)

| Instrument (verified 2026-10-04 unless marked) | Obligation | Binds component? | Repo evidence | Gap | Who owes |
|---|---|---|---|---|---|
| **Art 113 as amended** (consolidated text, fetched): (c) Annex III → **2 Dec 2027**; Annex I → **2 Aug 2028**; (a) now excepts Art 5(1)(ba),(bb)+(1a),(1b) → **2 Dec 2026**; (d) Arts 102–110 from 27 Jul 2026 | Phased application | **No** — component is not Annex III | `COMPLIANCE.md:400-432`; `src/reg_watch.rs:75-97` | **None — dates verified correct** | Deployer, if Annex III |
| **Art 50(2)** | Marking of synthetic output | **No — the component generates nothing** | `COMPLIANCE.md:486-492`; `src/provenance.rs:91` | None in substance | Provider of a *generating* system |
| **Art 111(4)** — flagged **"new"** in the consolidated text | Legacy transitional: systems placed on market before 2 Aug 2026 must comply with Art 50(2) by **2 Dec 2026** | No | `src/reg_watch.rs:75-88` | **L8-03** — wrong instrument cited | Provider |
| **Art 50(5)** | Disclosure *"in a clear and distinguishable manner **at the latest at the time of the first interaction or exposure**"* + accessibility | No (component is not an Art 50 system) — **but the repo's bridge is mis-mapped** | `src/handlers/well_known.rs:161-172` `build_ai_notice()`; routed public at `router/auth.rs:687-688` | **L8-02** | Deployer, at its own UI seam |
| **Art 50(1)** | Tell users they're talking to AI | No | `well_known.rs:127-139` `build_ai_literacy()` | None | Deployer |
| **Art 4** (AI literacy) — in force **2 Feb 2025** | Provider/deployer AI literacy | Deployer | `docs/AI_LITERACY.md` | None | Deployer |
| **Reg (EU) 2026/1744** ("Digital Omnibus on AI"), of **8 Jul 2026**, in force **27 Jul 2026** — existence corroborated (EFTA factsheet `32026R1744`, Legal500), **full text UNVERIFIED** | Defers high-risk only | — | `COMPLIANCE.md:400-441` | None found | — |

## 7.2 EU — CRA (Reg (EU) 2024/2847)

| Instrument | Obligation | Binds? | Repo evidence | Gap |
|---|---|---|---|---|
| **Art 14 reporting**, applies from **11 Sep 2026** per Art 71(2) *(citation UNVERIFIED)* | 24 h early warning / 72 h notification / final report | **Genuinely arguable** | Full runbook; pinned by `reg_watch.rs:137-168,179-198` | **L8-04 — channel row unfillable** |
| **Art 14 clocks** (14 days post-fix vs one month post-notification) | Split clocks | — | `cra-reporting-runbook.md:64-70` | **UNVERIFIED** — EUR-Lex blocked |
| **Art 13** SBOM | Machine-readable SBOM | **No** (open-source, non-commercial, outside the CRA economic-operator concept) | `scripts/sbom.sh:32`; `docs/cra.md:45-55` **honestly disclaims conformity assessment** | None |

## 7.3 US Federal

| Instrument | Obligation | Binds? | Repo evidence | Gap |
|---|---|---|---|---|
| **No comprehensive federal AI statute** | — | — | `US_STATE_MAP.md:5` | Consistent with everything reachable ✅ |
| **EO 14409**, "Promoting Advanced AI Innovation and Security", **2 Jun 2026** (full text fetched 2026-10-04) | CISA/NSA/Treasury AI-cyber clearinghouse; classified "covered frontier model" benchmark; §3(c) **expressly forbids mandatory licensing**; AG to prioritise AI-enabled 18 U.S.C. 1028/1030/1343 enforcement | Component: no. **Critical-infra deployers: procurement-relevant** | **Absent from the map** | **L8-05** |
| **EO 14434**, "Inaugurating The Era Of Super Intelligence", **29 Sep 2026** (fetched 2026-10-04) | Federal executive to use "Super Intelligence" in non-statutory documents; APST to propose statutory language in 60 days; §4(c) creates **no enforceable right** | No | **Absent** | **L8-05** |
| **TAKE IT DOWN Act**, Pub. L. 119-12 *(UNVERIFIED — congress.gov blocked)* | Covered-platform 48 h removal + identical-copy sweep | Covered **platforms** (public UGC). **brain-server: no.** | `US_STATE_MAP.md:25`; `well_known.rs:177-188` RFC 9116 `security.txt` is a real intake channel | None |
| FTC / EEOC / CFPB / HIPAA / SOC-for-Cybersecurity / BIS export controls on model weights | — | — | `COMPLIANCE.md:639-653,668-682` | **L8-11 — UNVERIFIED** |

## 7.4 US States (excerpt — full table in the leg output)

| State | Instrument | Duty date | Status in repo | Gap |
|---|---|---|---|---|
| **Connecticut** | CART Act, PA 26-15 *(statute UNVERIFIED)* | general duties incl. AI-layoff flag on WARN notices | `US_STATE_MAP.md:45` lists "CT general duties Oct 1 2026" under **"Scheduled"** | **L8-01 — HIGHEST SEVERITY. That date passed 3 days ago.** |
| Texas | TRAIGA HB149 | eff. 1 Jan 2026 | `:27` ✅ | None |
| California | SB53/AB2013/SB942+AB853/CCPA-ADMT/SB243/SB1119 | various | `:28-31`; `COMPLIANCE.md:700` scope accurate ✅ | None |
| Colorado | SB26-189 (repeals SB24-205) + HB26-1263 | Jan 2027 | `:32` — **the only row flagging its own litigation-stay uncertainty** ("do NOT treat it as vacated") ✅ best-honesty row | None |
| Utah/IL/NYC/FL/WA/GA/OR | SB149, HB3773, LL144, HB919, SB5838, SB540, SB 1546 | various | `:33-39` ✅ | None |
| **~40 states** | Deepfake/election/voice-cloning bucket | rolling | `:47-53` instructs **quarterly** NCSL checks; status **20 days stale** | **L8-06** |

## 7.5 World

| Jurisdiction | Instrument | Repo posture | Gap |
|---|---|---|---|
| **China** | Interim GenAI Measures (2023); AI content labelling measures eff. **1 Sep 2025** *(UNVERIFIED)* | **Absent.** `src/provenance.rs:28` "NOT C2PA" is a correct *non*-claim | **L8-08** |
| **South Korea** | AI Basic Act, asserted in force since Jan 2026 *(UNVERIFIED)* | Absent | **L8-08** |
| Japan / Brazil PL 2338 / Canada AIDA / Australia / India | — | Absent | **L8-08** |
| **UK** | Pro-innovation regime, no equivalent in force | Absent | **L8-12** (minor) |
| **Singapore** | MGF for Agentic AI, published 2026-01-22, updated 2026-05-20 *(IMDA page JS-gated; UNVERIFIED)* — dates enforced by `assert_eq!` at `reg_watch.rs:564-565` | `docs/compliance.md:135` correctly labels it **VOLUNTARY** ✅ | Date basis unverified |
| **CoE CETS 225** | Framework Convention, asserted in force 2025-09-01 *(UNVERIFIED)* | `docs/compliance.md:136`; repo is not a party ✅ | Date unverified |
| OECD AI Principles | Voluntary | Absent | Minor |

---

## 7.6 Standards currency — what I verified against the publishers

| Standard | Repo claim | Verified current (2026-10-04) | Verdict |
|---|---|---|---|
| **MCP specification** | `docs/mcp.md:67,109,239`: modern = **2026-07-28**, legacy 2025-11-25 | Context7 resolves `/websites/modelcontextprotocol_io_specification_2026-07-28` as current | ✅ **CORRECT AND CURRENT** |
| **MCP OAuth requirement** | `COMPLIANCE.md:335`: 2026-07-28 spec requires OAuth 2.0 PRM (RFC 9728) | Spec text: *"MCP servers MUST implement OAuth 2.0 Protected Resource Metadata"*; cites **OAuth 2.1 draft-13** | ✅ **CORRECT.** `x-brain-scope` is a legitimate `_meta` extension; gap honestly disclosed |
| **OWASP GenAI LLM Top 10** | published **2026-08-04** (`COMPLIANCE.md:10`) | Publisher page: **August 3, 2026** | Off by one day — edition current, date wrong |
| **OWASP Top 10 for Agentic Apps** | `COMPLIANCE.md:11` says **2025-12-10**; `COMPLIANCE.md:354` says **2025-12-09** | Publisher: **December 9, 2025** | **Internally inconsistent inside one file**; `:354` right, `:11` wrong |
| **LLM Top 10 entry names** | `COMPLIANCE.md:45-54` maps LLM01–LLM10 with a taxonomy that **differs from the 2025 edition** | 2026 edition introduces "updated rankings"; **entry names not readable** | **UNVERIFIED** — must be checked against the 2026 PDF |
| **NIST AI RMF** | `docs/compliance.md:132`: input window closed 2026-09-16 | nist.gov: *"The AI RMF 1.0 is being revised as part of the White House AI Action Plan."* New concept note: **AI RMF Profile on Trustworthy AI in Critical Infrastructure, 7 Apr 2026** | Directionally right; the **critical-infrastructure profile is unmentioned anywhere** → **L8-10** |
| **SLSA** | `docs/api.md:233`: "no SLSA provenance and no SLSA build level" | — | ✅ **Disclaimed honestly** — the negative claim is the right posture |
| **CWE Top 25** (2025, page updated 15 Dec 2025) | not referenced | #4 = CWE-862 Missing Authorization, #21 = CWE-306 Missing Authentication — **map directly onto this codebase** | → **L8-09**, free citation left on the table |
| **ISO 42001:2023 / 23894 / 27001 / 27005, ASVS, MITRE ATLAS, sigstore, OAuth 2.1 BCP, CycloneDX 1.5** | mostly absent (C2PA only as an explicit non-claim) | paywalled/Akamai-blocked | **L8-09** |

---

## 7.7 Prioritized regulatory gaps

**L8-01 — HIGH. CT CART Act general duties are live NOW; the map still says "Scheduled."**
`docs/US_STATE_MAP.md:45` lists *"CT general duties Oct 1 2026"* under **Scheduled**. Today is
2026-10-04 — **that date passed three days ago.** The operator checklist (`:109`) repeats
"checkout/HR notices (CT Oct 2026)" with no "now live" flag. **The single highest-impact item:
the repo asserted this duty against itself, set its own clock, and never re-armed it.** An
operator reading this document today is told to wait. *(Statute text UNVERIFIED — `cga.ct.gov`
unreachable. Next: PA 26-15 §-by-§ effective-date and enforcement clauses.)*

**L8-02 — HIGH (substantive mis-mapping). `/.well-known/ai-notice` cannot satisfy Art 50(5).**
`COMPLIANCE.md:502-510` calls the endpoint *"the Art 50 disclosure itself"*; `docs/compliance.md:134`
calls it an "Art 50 origin metadata note"; `src/handlers/well_known.rs:165` emits `"art_50": true`.
Verified against Art 50(5): disclosure must be given *"at the latest at the time of the first
interaction or exposure."* A machine-readable JSON document at a well-known path, fetched by an
auditor or a plugin, is not that. **The component is right to have no Art 50 duty** — it generates
nothing and is not an Art 50 system. **The error is the claim shape, not the scope**, and that is
what reaches a procurement reader. *Fix: relabel as deployer-side disclosure **input**; scope or
drop the `art_50: true` assertion.*

**L8-03 — MEDIUM. `reg_watch.rs` cites the wrong instrument for a load-bearing constant.**
`src/reg_watch.rs:75-88` bases the 2026-12-02 Art 50(2) transitional on *"recital 38 grants a
four-month transitional period."* The consolidated text shows the operative, verifiable hook is
**Article 111(4)**, flagged **"new"** in the amendment. **The date is right; the cited mechanism is
a recital rather than the enacting provision** — and `ai_act_art50_marking_deliverable`
(`:209-230`) pins `provenance.rs` against this constant, so the defect propagates into a **green CI gate.**

**L8-04 — MEDIUM-HIGH. The CRA runbook's reporting channel points at a blank that does not exist.**
`docs/cra-reporting-runbook.md:83` instructs the operator to submit *"under the manufacturer
identity registered in SUPPORT.md"*; the template at `:87-90` is a fill-in blank.
**`SUPPORT.md` is 32 lines and contains no manufacturer identity, no EU main establishment, and no
authorised representative.** Art 14 has been live 23 days. The repo is right to be sceptical that a
single-operator self-hosted project is a CRA "manufacturer" at all — but the runbook **presumes**
the answer and then cannot complete it. *Fix: state the applicability question explicitly, then
either populate the identity or mark the channel N/A for a non-EU single-operator deployment.*

**L8-05 — MEDIUM. The federal row omits two 2026 Executive Orders, one procurement-material.**
`US_STATE_MAP.md:5` correctly asserts no comprehensive federal AI law, but carries no federal row
at all — and since its 2026-09-14 date, **EO 14409 (2 Jun 2026)** and **EO 14434 (29 Sep 2026)**
exist. Neither binds the component; both belong in a deployer's federal row.

**L8-06 — MEDIUM. The map is 20 days stale against its own quarterly cadence.** Status `:5` is
2026-09-14; `:53,118-120` instruct quarterly NCSL/legislature checks, which **could not be run**
(NCSL Cloudflare-blocked). Any post-11 Sep 2026 enactment is unrefreshed. Severity bounded by the
file's own framing ("No per-state code fork needed").

**L8-07 — LOW. OWASP edition dates are wrong and contradict each other inside one file.**
`COMPLIANCE.md:11` (2025-12-10) vs `:354` (2025-12-09); publisher says **Dec 9**. LLM Top 10 dated
2026-08-04 vs publisher's **Aug 3**. *The irony worth naming: this repo has a machine-checked
calendar (`src/reg_watch.rs`) and a docs-truth discipline — and these two standards dates are
hand-typed and unpinned.*

**L8-08 — LOW for the component, MEDIUM for deployers. The entire non-EU/non-US set is absent.**
No China, South Korea, Japan, Brazil, Canada, Australia, India, or UK row. Given the product ships
Slack/Teams/WhatsApp/Signal bridges and claims international buyers, this is a procurement-surface
gap. **None of these instruments was verifiable this session** — treat as a scoped map to build,
not a set of confirmed duties.

**L8-09 — LOW. The standards map omits nearly everything the brief asks for.** No CWE Top 25,
MITRE ATLAS, ASVS, ISO/IEC 23894, ISO 27001/27005, sigstore, or OAuth 2.1 BCP. CWE-862 and CWE-306
map directly onto what this codebase already does well.

**L8-10 — LOW. The NIST AI RMF critical-infrastructure profile (7 Apr 2026) is unmentioned.** For
a repo with a documented contact-centre/regulated-sector posture, that is the relevant profile.

**L8-11 — UNKNOWN. No export-control analysis on model weights.** The repo ships an optional ONNX
classifier and locally-embedded embedding weights, and has a documented PQC seam. **I could not
reach BIS/ECFR.** This is a genuine unknown, **not** a clean bill of health.

---

## 7.8 What this leg could NOT verify — and why

| Item | Blocked by | Next check |
|---|---|---|
| **CRA Art 14 full text** (24 h/72 h/14-day/one-month clocks) | EUR-Lex → HTTP 202 JS-challenge; op.europa.eu → 403 | CELEX **32024R2847**; or a national implementing act from a reachable mirror |
| **Reg (EU) 2026/1744** recital 38 + Art 111(4) | EUR-Lex blocked; secondary corroboration only | CELEX **32026R1744** |
| **CT PA 26-15** — what took effect 2026-10-01 | `cga.ct.gov` unreachable | CT CGA bill page; §-by-§ effective-date clause; CT AG guidance |
| **All other US statutes** (CO/TX/CA/UT/IL/NYC/FL/WA/GA/OR) | congress.gov + NCSL Cloudflare | State legislature pages directly; CPPA and CO AG releases |
| **US federal sectoral** — FTC disgorgement, EEOC, CFPB, HIPAA, SOC-for-Cybersecurity, procurement, **BIS export controls** | no reachable primary source; quota exhausted | ftc.gov, eeoc.gov, ecfr.gov (15 CFR §774 Supp. No. 4), acq.osd.mil |
| **Non-US/world** — China, Korea, Japan, Brazil, Canada, Australia, India, OECD | quota exhausted | CAC, MSIT/MOLIT, national regulators |
| Singapore MGF dates (enforced by `assert_eq!` in CI) | IMDA page JS-gated | IMDA primary PDF |
| CoE CETS 225 in-force date | coe.int Cloudflare | Council of Europe Treaty Office ratification table |
| ISO/ASVS/ATLAS edition currency | paywalled/Akamai | Publisher catalogues; OWASP ASVS project page |
| **OWASP LLM Top 10 2026 entry-by-entry names** | behind a redirect/JS gate | The 2026 PDF; verify `COMPLIANCE.md:45-54` matches |

**Three caveats on this leg's own work, which matter more than the findings:**
1. **The CRA clocks — the single most-cited legal claim in this repo — were not verified at all.**
   Treat `cra-reporting-runbook.md`'s 14-day/one-month split as plausible-but-unconfirmed.
2. **No US statute text was verified.** Every US row is the repo's claim, re-dated only where the
   repo's own arithmetic proves it stale (CT), and flagged elsewhere.
3. **Quota exhaustion removed secondary-source triangulation** for blocked jurisdictions. Where I
   fell back to secondary corroboration, I labelled it as such rather than dressing it as primary.

### Where the repo OVERCLAIMS vs UNDERCLAIMS — and what it gets RIGHT

**OVERCLAIMS** (asserts a duty that does not bind this component): the `art_50: true` assertion and
"Art 50 disclosure itself" framing (`L8-02`); the `AI_ACT_ART50_MARKING` pin framing a **provider**
deadline as a **component** deliverable (`L8-03`); the CRA runbook presuming manufacturer status it
never establishes (`L8-04`).

**UNDERCLAIMS:** CT live 3 days ago and still filed as future (`L8-01`); two federal EOs absent
(`L8-05`); map 20 days stale against its own cadence (`L8-06`); NIST CI profile unmentioned (`L8-10`).

**GETS RIGHT — stated plainly, because it made this audit faster and the findings sharper:**
the component/deployer distinction is drawn correctly and consistently throughout; `docs/cra.md`
disclaims conformity assessment rather than implying it; the Colorado row flags its *own*
litigation-stay uncertainty and says "do NOT treat it as vacated"; `docs/api.md` refuses to claim
SLSA; `src/provenance.rs` says **NOT C2PA** rather than implying conformance;
`COMPLIANCE.md:143-146` correctly quarantines LLM07 from the AI Act's generator-provider duty; and
`src/provenance.rs:19` signs the **claim-bound wrapper** `{artifact, claim}` — the right call, and pinned.

---