# US State AI Map - Operator Runbook (re-verified against 1.29.2)

Scope: brain-server is a single-node loopback-first memory component. It does not train frontier models, does not make consequential decisions by itself, and serves no UI to consumers. Most US duties fall on the deployer / operator for their use case. This file lists what the component gives you live, and what you must still do.

Status date: 2026-09-14. Verify dates against primary sources before a filing. No comprehensive federal AI law as of this date.

> **Refresh cadence — BLOCKED, and deliberately NOT re-stamped.** The map's own
> instruction is a quarterly pass over NCSL + legislature pages (see "the other
> ~40 states" below). The pass due after 2026-09-14 **has not been run**: NCSL
> was unreachable (Cloudflare-blocked) from the build environment. The status
> date above therefore still reads 2026-09-14 **on purpose** — bumping it would
> claim a verification that never happened, which is the "a number shipped
> without anyone diffing it against a measurement" failure this repo exists to
> prevent. Next due: **2026-12-14**.
>
> What DID happen without the network: the CT CART general-duty date (Oct 1
> 2026) had already passed and was still filed under "Scheduled", so the filing
> is corrected above. That is arithmetic against a date this repo already
> asserted — not a fresh legislative check, and not a substitute for one.
>
> Also unverified from this environment, and therefore absent from the table
> rather than guessed: the two 2026 federal Executive Orders cited in the
> eighth-pass audit (EO 14409, EO 14434). No primary federal source is
> reachable from a build, and an unreached instrument must not be written into
> a deployer-facing register. See `AUDIT.md` (L8-05).

## Common live evidence (all states)

- Recall trace: `POST /recall?trace=true` + `GET /recall/{id}/trace` - what chunks, scores, abstention, scope, principal, domains.
- Audit: append-only keyed HMAC-SHA256 chain + `GET /audit/verify` + `/metrics` chain-ok. Read events opt-in via `BRAIN_AUDIT_READ_EVENTS=on`. For ADMT / employment review set `BRAIN_AUDIT_RETENTION_DAYS=180` or higher.
- DSAR: `POST /dsar {subject, action: export|purge|both}` - locate by owner + derived_from depth 8, export portable JSON, purge clears knowledge + vec_knowledge + FTS5 + relationships + evidence_links + proposals + workflow family in one transaction, tombstone idempotent, certificate with chain_head. Registry: `GET /tombstones`, cert: `GET /dsar/{id}/certificate`.
- Export provenance: `/export` emits source + origin (human/model/imported) + assertion_kind + confidence + provenance_summary. Use it to feed deployer disclosures.
- Retention: per-kind windows + `GET /retention/report`. Legal hold freezes ids against purge/decay with 409 legal_hold_active.
- Write gate: v1.14 proposal gate + quarantine. No autonomous promotion.
- Residency: `BRAIN_REGION` stamp on certificates. Data stays on host by default. No outbound HTTP except opt-in DSAR webhook HMAC-signed.
- Public notices: `GET /.well-known/ai-notice` (Art 50 pattern, reusable for US disclosure copy), `GET /.well-known/ai-literacy`, `GET /.well-known/cop-notice`.
- ADMT kit: `scripts/admt-kit.sh <chunk-id>` assembles get + audit rows for an assessor.

Ceilings (do not hide in a pilot): no app-level encryption at rest (operator full-disk LUKS/FileVault), single-process chain, read events off by default in loopback, backups are a third copy - you must run purge-aware rotation, trace endpoint serves recorded events only (no backfill pre-v1.15).

## Enacted / scheduled with direct private-sector duties

| Jurisdiction | Law | Effective | Trigger | Component live | Operator must do |
|---|---|---|---|---|---|
| Federal | TAKE IT DOWN Act Pub.L.119-12 (FTC enforces §3) | Criminal §2 effective from ENACTMENT 2025-05-19; FTC §3 notice-and-removal ENFORCEMENT live 2026-05-19 (the L7-02 correction — the pre-1.28.88 row inverted the two dates) | Covered platforms (public UGC forums): criminal ban on knowing publication of nonconsensual intimate depictions incl. AI digital forgeries; valid victim request → remove + reasonable efforts on identical copies ≤48h; FTC treats violations as FTC-rule violations (~$53k/violation). Verified 2026-09-14 vs govinfo PL 119-12 + FTC compliance page. | Purge/tombstone/certificate as removal proof (the same takedown primitive as the state bucket). | If you operate a covered platform: publish the plain-language notice-and-removal process NOW, wire the 48h removal + identical-copy sweep SOP to /dsar purge. This clock is STRICTER than every state window in the bucket below — follow it. |
|---|---|---|---|---|---|
| Texas | TRAIGA HB149 | Jan 1 2026 | Any AI offered/used in TX. Bans: incite self-harm/crime, CSAM, nonconsensual intimate deepfake, government social scoring / nonconsensual biometrics. Disclosure for state agencies. AG enforcement, no private right. | Trace + audit as reasonable-oversight evidence. Quarantine for injection. Purge/tombstone for CSAM/deepfake takedown. | Attest no prohibited intent/use. Wire takedown SOP to /dsar purge. Keep audit retention. No impact assessment required by TRAIGA (cut from final). |
| California | SB53 TFAIA frontier + AB2013 training data | Jan 1 2026 | SB53: frontier developers over 1e26 FLOPs - safety framework publish, incident report, whistleblower. AB2013: any GenAI dev in CA - post training-data summary, repost on substantial mod, covers systems from Jan 1 2022. | Out of scope correctly for memory component (no training). No code change. Keep scope note for procurement. | If you are also a frontier/GenAI dev, publish framework + data summary separately. Memory exports do not satisfy AB2013. |
| California | SB942 AI Transparency as amended by AB853 | Covered-provider duties operative Aug 2 2026. Platform/hosting/capture-device phases 2027-2028. $5k penalties. | Large GenAI providers: free detection tool, latent disclosure, provenance. | Provenance fields + ai-notice endpoint are the bridge a provider can consume. Not a watermarking engine. | If you are a covered provider, build detection tool + marking separately. If you are a deployer, surface disclosure in your UI using /export origin. Server cannot disclose alone. |
| California | CCPA/CPRA + ADMT regs | Privacy live. ADMT full regime Jan 1 2027. Risk-assessment filings from Apr 1 2028. | Automated decision tech: right to know logic, opt-out, risk assessments. | /export portability, purge/tombstone deletion proof, trace for logic explanation, retention report. | Honor 45-day DSAR clocks, run risk assessments for high-risk uses, implement opt-out in your app, set retention windows. |
| California | SB 1119 "Adam's Law" + the 2026-09-10 package (SB 867, AB 302 et al.), signed 2026-09-10 | Operative-date check owed: verify each bill's operative date against the leg info before filing (SB 243's chatbot baseline has been live since Jan 1 2026) | Companion-chatbot child safety: crisis-resource delivery on distress signals, self-harm/suicidal-ideation detection + PARENTAL NOTIFICATION for minor users, bans on manipulative/deceptive/sexualized companion conduct toward minors, pre-release safety protocols + testing, annual compliance reporting/audit. The L7-03 correction — the map's 2026-09-11 status date predated the signing by one day and the package was missing. Verified 2026-09-14 vs the Padilla office announcement + bill trackers. | ai-notice disclosure copy, origin metadata, audit trail; the parental-notification/crisis-protocol duties live in the DEPLOYER's chatbot surface, not the memory store. | If you operate companion chatbots in CA: ship distress detection + crisis resources + minor safeguards + parental notification per SB 1119, calendar the annual report; verify operative dates with counsel. |
| Colorado | SB26-189 ADMT Act (repeals SB24-205) signed May 14 2026 + HB26-1263 Chatbot Safety Act signed May 29 2026 | SB26-189: Jan 1 2027 (old Feb 1 / Jun 30 2026 dates dead). HB26-1263: operative duties Jan 1 2027 (act eff Aug 12 2026). | SB26-189: developers + deployers of covered ADMT materially influencing consequential decisions (employment, housing, credit, insurance, education, health). Docs, notices, records, correction, human review. AG exclusive, no private right. HB26-1263: operators of conversational AI (public-facing): age estimation (commercially reasonable methods), AI-not-human disclosure (persistent/repetitive/responsive), no engagement-reward tricks for minors, anti-sexual-content + anti-emotional-dependence measures for minors, self-harm protocol with crisis referral, no licensed-professional impersonation, annual AG report from Jul 1 2027. Verified 2026-09-12 vs leg.colorado.gov HB26-1263 (Signed Act Ch.208). NOTE: the Apr 27 2026 stay attached to repealed SB24-205 (xAI v. Weiser, order textually extended to replacement legislation) — counsel confirms whether Jan-2027 stands; do NOT treat it as vacated. | Impact evidence: trace + audit + admt-kit + retention report. NIST AI RMF map in COMPLIANCE.md for safe-harbor narrative. Conversational-AI disclosure copy can cite origin metadata + ai-notice endpoint. | Write impact assessment, consumer notices, correction/appeal path, human-review gate in your workflow. Do not treat old SB24-205 checklist as current. If you serve conversational AI: ship the AI-not-human disclosure + minor safeguards + self-harm protocol by Jan 1 2027; calendar the AG report. |
| Utah | AI Policy Act SB149 eff May 1 2024, amended 2025 SB226/HB452 | In force | Disclose GenAI use on request, proactive in high-risk (health/financial/legal, regulated occupations, mental-health chatbots). Business liable for AI statements. $2.5k / $5k repeat. AI Learning Lab path. | Origin metadata + ai-notice copy + audit of what was served. | Add upfront disclosure in high-risk flows, answer on-request disclosure from /export + trace, train staff that machine-did-it is no defense. |
| Illinois | HB3773 amends IHRA + AI Video Interview Act (2020); SB315 AI Safety Measures Act PA 104-0538 signed Jul 6 2026 | HB3773: Jan 1 2026. SB315: eff Jan 1 2027 (frontier framework + third-party-audit duties phase Jan 1 2028). | HB3773: Employer AI in hiring/promotion/discharge where it discriminates or uses zip as proxy. Notice required. IDHR enforcement. SB315: large frontier developers (>$500M revenue, >1e26-FLOP models, operating in IL): publish frontier AI framework + transparency reports, critical-incident reporting, whistleblower non-retaliation, ANNUAL independent third-party audits from Jan 2028, $1M/$3M AG penalties, no private right. Verified 2026-09-12 vs ILGA PA 104-0538. | Trace + scope filter + audit show what data informed a stored decision. Purge for bad entries. (Frontier-dev duties are out of the memory component's scope — correctly unclaimed.) | Notify applicants/employees when AI used, test for disparate impact, do not use zip proxies, keep audit for IDHR inquiry. Server does not test impact alone. If you are ALSO a large frontier dev: file IL disclosure, publish the framework, retain the auditor for Jan 2028. |
| New York City | Local Law 144 AEDT | In force since Jul 5 2023, DCWP enforces | Employers/agencies using AEDT for NYC hiring/promotion: annual independent bias audit, public summary, candidate notice. | Audit + trace + retention report feed the auditor. | Hire independent auditor yearly, publish summary, give 10-business-day candidate notice in your hiring flow. |
| Connecticut | CART Act (SB5, PA 26-15), signed May 27 2026 (announced Jun 2) | General duties Oct 1 2026 (AI layoff flag on WARN notices); principal AEDT notice/disclosure duties Oct 1 2027 | AEDT broadly defined (substantial factor in employment decisions). AI use is no defense to discrimination claims; anti-bias testing counts as mitigation. No private right. | Subscription flag can be stored as provenance + audit; layoff notice workflow can use workflow lineage events. | Implement checkout disclosure + HR notice process by Oct 1 2026. Plan AEDT program for Oct 2027. |
| Florida | HB919 political ads + 836.13 altered sexual depictions (2025 CS/SB1400 amend adds covered-platform 48h victim-request removal + posted mechanism) | In force (conduct-triggered) | AI political-ad disclaimers, deepfake intimate-image bans. PLATFORMS: ≤48h removal on victim request with a posted notice mechanism. | Purge/tombstone takedown + certificate as removal proof. | Add disclaimer renderer in ad flow, takedown SOP wired to purge. If you run a covered platform: post the removal mechanism and meet the 48h clock (the federal TAKE IT DOWN clock above is the same SLA — follow either, both land at 48h). |
| Washington | SB5838 Task Force (final report Jul 1 2026: 11 recommendations, 4 enacted incl. companion-chatbot duties eff Jan 2027, health prior-auth transparency, law-enforcement disclosure, CSAM) | Study complete; companion/health/LE/CSAM duties live or scheduled per their own statutes | SB5838 itself imposed no private duty — the map's old "study only" cell is now READ THE FINAL REPORT + check the four 2026 enactments for your trigger. | None required by SB5838. NIST map reusable. | Read the Jul 1 2026 final report; if you serve companion chatbots in WA, meet the Jan-2027 duties; otherwise no filing due. |
| Georgia + Oregon | GA SB 540 (companion-chatbot disclosures + minor protections, effective Jul 1 2027); OR SB 1546 (signed 2026-03-31: AI-not-human disclosure, self-harm detection + crisis-resource interruption, harm-prevention steps, PRIVATE RIGHT OF ACTION) | GA: Jul 1 2027. OR: signed/enacted 2026-03-31 — check operative date with counsel | The companion-chatbot family is now MULTI-STATE (the L7-03 correction — the map carried only CO/WA): CA SB 243 (live Jan 2026) + SB 1119 (above), CO HB26-1263, WA HB 2225-class duties, GA SB 540, OR SB 1546. Verified 2026-09-14 vs BillTrack50 + the Oregon Legislature OLIS page. | ai-notice disclosure copy + origin metadata + audit trail cover the disclosure legs; crisis-protocol duties are deployer-side. | Companion-chatbot operators: treat the family as one compliance surface — disclosure + crisis protocol + minor safeguards everywhere, OR's private-right-of-action makes Oregon the strictest enforcement venue; verify each operative date. |

Watchlist (no deployer duty yet): Virginia HB2094 vetoed 2025 (expect 2027 reintro), New Jersey A3854 hiring bias-audit proposed (NYC-style). Treat as plan-ahead, not backlog.

## Status snapshot (2026-09-14) — live now vs scheduled

Live and enforceable today: federal TAKE IT DOWN criminal §2 (from enactment 2025-05-19) + FTC 48h removal enforcement (§3, from 2026-05-19), TX TRAIGA, CA SB53/AB2013, CA SB942 (provider tier), CA SB 243 chatbot baseline + OR SB 1546, UT SB149, IL HB3773, NYC LL144, TN ELVIS Act, FL deepfake/election rules, **CT CART Act general duties (Oct 1 2026 — the date has passed; moved out of "Scheduled" 2026-10-05)**. Scheduled: IL SB315 eff Jan 1 2027 (audit duties Jan 2028) + CA ADMT business compliance + CO SB26-189 + CO HB26-1263 + GA SB 540 operative duties Jan 1 2027 (GA Jul 1 2027); CT AEDT duties Oct 1 2027; CA risk-assessment filings Apr 1 2028. Watch with counsel: CO stay scope (SB24-205 stay vs SB26-189), CA SB 1119 operative dates, any federal preemption ruling.

> **Scope of the CT correction — bookkeeping only.** The date arithmetic is
> provable from this repo: it asserted Oct 1 2026, and that date is in the past.
> Moving the entry from "Scheduled" to "Live" corrects this document's own filing
> of its own date. It is **not** a legal conclusion about what CT PA 26-15
> requires — the statute text remains UNVERIFIED (`cga.ct.gov` unreachable from
> the build environment), and the deployer-side obligation (checkout/HR notice
> copy) is one the server cannot observe, so it gets no `src/reg_watch.rs`
> deliverable pin. A pin asserting an artifact the server cannot see would be
> theatre; the honest machine-checked shape here would be a date-only WATCH,
> which is less than what already exists.

## The other ~40 states: narrow deepfake / election bucket

As of mid-2026 every state has introduced AI bills, 145 enacted in 2025, but outside the table above the enacted pattern is narrow: nonconsensual intimate imagery takedown, election candidate-impersonation disclaimer windows (often 60-90 days pre-election), voice-cloning (TN ELVIS Act Jul 1 2024), plus AZ/MI/MN/TX/WA election variants, NJ deepfake enacted, MA/MD study commissions.

Component posture for all of them: same takedown primitive (locate/purge/tombstone/certificate) + provenance to prove origin + audit to prove when. FEDERAL FLOOR: the TAKE IT DOWN Act's 48h removal + identical-copy sweep (row above) binds covered platforms everywhere in the US — a state window never loosens it. Operator wires two things per state where they operate: (1) disclaimer copy in the generating surface, (2) takedown clock SOP pointing at /dsar purge. No per-state code fork needed. Check NCSL database + legislature page quarterly; deepfake windows move fast.

Full 50-state inventory method: start from NCSL AI legislation database + Orrick AI Law Tracker + Atlas 13-record tracker, then filter to enacted + conduct trigger. Do not copy pending-bill text into controls; pending is signal, not duty.

## Enterprise profile snippet (copy/paste)

```bash
BRAIN_AUDIT_READ_EVENTS=on
BRAIN_AUDIT_RETENTION_DAYS=180
BRAIN_REGION=us-texas-1
BRAIN_WRITE_POSTURE=review
```

Plus openclaw.json enterprise posture: autoCapture false, allowedChatTypes direct/explicit only, strictDomain true, TopK 5/2500, workspaceOnly true.

## What this does not claim

ISO 42001 / SOC 2 attestation, BAA, bias-audit opinion, or legal advice are operator / external-auditor layers. This file + COMPLIANCE.md are the technical-file evidence those audits consume.

## Operator checklist — proof in code

Work top to bottom before operating in any listed state. Each row names the
proof: a route, a command, or a test. Anything unchecked is a gap, not a
deferral.

Component (verify once per deployment):

- [ ] Recall trace answers. `POST /recall?trace=true`, then
  `GET /recall/{id}/trace` replays chunks, scores, abstention, scope,
  principal, domains. Proves logic-explanation duties (CA ADMT, IL notice).
- [ ] Audit chain verifies. `GET /audit/verify` returns ok;
  `/metrics` chain-ok gauge reads 1. Proves oversight and record-keeping
  duties (TX, CO, CT, NYC auditor feed).
- [ ] Read events on with retention. `BRAIN_AUDIT_READ_EVENTS=on` and
  `BRAIN_AUDIT_RETENTION_DAYS=180` (or higher for employment review).
  Default is off on loopback: this is the most commonly missed row.
- [ ] DSAR round-trips. `POST /dsar {subject, action: both}` exports,
  purges, and returns a certificate; `GET /dsar/{id}/certificate`
  re-verifies; `GET /tombstones` lists the registry. Proves deletion and
  correction duties (CCPA, CO correction right).
- [ ] Export carries provenance. `/export` rows include source, origin,
  assertion_kind, confidence. Feeds deployer disclosures (UT, IL, CA).
- [ ] Retention report runs. `GET /retention/report` returns per-kind
  windows; legal holds report `held_ids` instead of purging (409
  `legal_hold_active` under hold).
- [ ] Write gate closed. `BRAIN_WRITE_POSTURE=review` (proposals, no
  autonomous promotion) and `INJECTION_POLICY` left at default
  `quarantine` (never `allow` where untrusted content arrives —
  `/health/db` tripwire `allow_policy_bypasses` must read 0).
- [ ] Region stamped. `BRAIN_REGION` set (e.g. `us-texas-1`); certificates
  carry it.

Operator process (verify per state you operate in):

- [ ] Takedown SOP points at purge. CSAM / deepfake / bad-entry removal
  runs `POST /dsar {action: purge}` with a named owner and a clock
  (TX, FL, TN, election windows).
- [ ] Disclosure copy live. AI-use notices in high-risk flows (UT),
  hiring notices (IL), **checkout/HR notices (CT — in force since Oct 1
  2026, so this is a CURRENT duty, not a scheduled one)**, candidate
  AEDT notices (NYC 10 business days, CT Oct 2027).
- [ ] Opt-out and human review paths exist in your app (CA ADMT, CO).
  The server provides the evidence; the buttons live in your surface.
- [ ] Impact assessment written and filed per calendar (CO Jan 2027,
  CA risk assessments Apr 2028). Trace + retention report are inputs,
  not the assessment itself.
- [ ] NYC bias audit hired yearly with published summary (LL144).
  No component substitutes for the independent auditor.
- [ ] Dates re-checked quarterly against primary sources (legislature
  pages, AG offices, CPPA). This file is dated 2026-09-14; statutes and
  stays move.

---

## Addendum — verified 2026-10-06 (ninth-pass regulatory arm)

**The status date above is deliberately NOT bumped.** The quarterly pass the
cadence requires did not run: NCSL is still Cloudflare-blocked from the build
environment (with it, orrick 403 / iapp 404 / olis timeout / cga.ct.gov dead
/ legiscan 403). Bumping the date would claim a verification most rows never
got. What follows is the dated record of the subset that WAS verified today
and how — the file's own discipline, extended rather than overridden.

**Federal — the two Executive Orders previously withheld are now verifiable
and the rows exist (L9-03).** The blockquote above said no primary federal
source was reachable, so the EOs stayed absent rather than guessed; the
ninth pass reached the **Federal Register** and both are `[V]`:

- **EO 14409** — published FR **2026-06-05**. Federal-agency /
  covered-platform duties, not component duties; deployer-level.
- **EO 14434** — published FR **2026-10-02** (four days before this
  addendum). Same posture.

Also FR-verified today: **FTC TIDA enforcement live since 2026-05** (FTC
blog) and an **FTC AI-impersonation NPRM published 2026-10-01**. None of
these change the component's posture (the header's "no comprehensive federal
AI law" stands — these are EOs and rulemaking, not statutes), but a
deployer-facing register should no longer say they are unverifiable.

**Export controls (L9-15): UNKNOWN → measured.** The 2026 FR sweep found
**no BIS model-weights rule** (chip/chokepoint rulemaking continues). This
component is not a weights distributor; the row moves from "unknown" to
"none found in the FR sweep as of 2026-10-06 — watch", which is a dated
observation, not a permanent fact.

**CT CART (L9-16): live on date arithmetic, statute still unread.** General
duties (PA 26-15) went live **2026-10-01** — five days before this
addendum — on the calendar this repo already asserted. cga.ct.gov remains
connection-dead, so **the statute text has still never been read from a
primary source**; treat the CT rows as date-verified, text-unverified.

**States/standards verified despite the wall:** CO SB26-189 (signed
2026-05-14, Ch.131, duties 2027-01-01 — primary), CPPA ADMT package
(existence; partial), EU AI Act Art 111(4) transitional date
(consolidated-text, 2nd verification), MCP spec currency (2026-07-28 —
sessions removed, `server/discover` added; a re-map is advisable),
CycloneDX 1.7.2 vs cargo-cyclonedx 0.5.9's 1.5 ceiling (pin confirmed
correct), SLSA v1.2, sigstore cosign v3.1.3, A2A v1.0.1, OAuth 2.1 still an
Active I-D (never cite as RFC). Access dates and the reachability ledger
live in the ninth-pass audit report.

**Next full quarterly due: 2026-12-14** (unchanged — this addendum is not
the quarterly pass).
