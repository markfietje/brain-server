# US State AI Map - Operator Runbook (v1.28.80)

Scope: brain-server is a single-node loopback-first memory component. It does not train frontier models, does not make consequential decisions by itself, and serves no UI to consumers. Most US duties fall on the deployer / operator for their use case. This file lists what the component gives you live, and what you must still do.

Status date: 2026-09-11. Verify dates against primary sources before a filing. No comprehensive federal AI law as of this date.

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
| Texas | TRAIGA HB149 | Jan 1 2026 | Any AI offered/used in TX. Bans: incite self-harm/crime, CSAM, nonconsensual intimate deepfake, government social scoring / nonconsensual biometrics. Disclosure for state agencies. AG enforcement, no private right. | Trace + audit as reasonable-oversight evidence. Quarantine for injection. Purge/tombstone for CSAM/deepfake takedown. | Attest no prohibited intent/use. Wire takedown SOP to /dsar purge. Keep audit retention. No impact assessment required by TRAIGA (cut from final). |
| California | SB53 TFAIA frontier + AB2013 training data | Jan 1 2026 | SB53: frontier developers over 1e26 FLOPs - safety framework publish, incident report, whistleblower. AB2013: any GenAI dev in CA - post training-data summary, repost on substantial mod, covers systems from Jan 1 2022. | Out of scope correctly for memory component (no training). No code change. Keep scope note for procurement. | If you are also a frontier/GenAI dev, publish framework + data summary separately. Memory exports do not satisfy AB2013. |
| California | SB942 AI Transparency as amended by AB853 | Covered-provider duties operative Aug 2 2026. Platform/hosting/capture-device phases 2027-2028. $5k penalties. | Large GenAI providers: free detection tool, latent disclosure, provenance. | Provenance fields + ai-notice endpoint are the bridge a provider can consume. Not a watermarking engine. | If you are a covered provider, build detection tool + marking separately. If you are a deployer, surface disclosure in your UI using /export origin. Server cannot disclose alone. |
| California | CCPA/CPRA + ADMT regs | Privacy live. ADMT full regime Jan 1 2027. Risk-assessment filings from Apr 1 2028. | Automated decision tech: right to know logic, opt-out, risk assessments. | /export portability, purge/tombstone deletion proof, trace for logic explanation, retention report. | Honor 45-day DSAR clocks, run risk assessments for high-risk uses, implement opt-out in your app, set retention windows. |
| Colorado | SB26-189 ADMT Act (repeals SB24-205) signed May 14 2026 | Jan 1 2027 (old Feb 1 / Jun 30 2026 dates dead, enforcement paused Apr 27 2026 for old law) | Developers + deployers of covered ADMT materially influencing consequential decisions (employment, housing, credit, insurance, education, health). Docs, notices, records, correction, human review. AG exclusive, no private right. NOTE (mid-2026): enforcement reported stayed pending xAI litigation — track status with counsel before assuming the date holds. | Impact evidence: trace + audit + admt-kit + retention report. NIST AI RMF map in COMPLIANCE.md for safe-harbor narrative. | Write impact assessment, consumer notices, correction/appeal path, human-review gate in your workflow. Do not treat old SB24-205 checklist as current. |
| Utah | AI Policy Act SB149 eff May 1 2024, amended 2025 SB226/HB452 | In force | Disclose GenAI use on request, proactive in high-risk (health/financial/legal, regulated occupations, mental-health chatbots). Business liable for AI statements. $2.5k / $5k repeat. AI Learning Lab path. | Origin metadata + ai-notice copy + audit of what was served. | Add upfront disclosure in high-risk flows, answer on-request disclosure from /export + trace, train staff that machine-did-it is no defense. |
| Illinois | HB3773 amends IHRA + AI Video Interview Act (2020) | Jan 1 2026 | Employer AI in hiring/promotion/discharge where it discriminates or uses zip as proxy. Notice required. IDHR enforcement. | Trace + scope filter + audit show what data informed a stored decision. Purge for bad entries. | Notify applicants/employees when AI used, test for disparate impact, do not use zip proxies, keep audit for IDHR inquiry. Server does not test impact alone. |
| New York City | Local Law 144 AEDT | In force since Jul 5 2023, DCWP enforces | Employers/agencies using AEDT for NYC hiring/promotion: annual independent bias audit, public summary, candidate notice. | Audit + trace + retention report feed the auditor. | Hire independent auditor yearly, publish summary, give 10-business-day candidate notice in your hiring flow. |
| Connecticut | CART Act (SB5, PA 26-15), signed Jun 2 2026 | General duties Oct 1 2026 (AI layoff flag on WARN notices); principal AEDT notice/disclosure duties Oct 1 2027 | AEDT broadly defined (substantial factor in employment decisions). AI use is no defense to discrimination claims; anti-bias testing counts as mitigation. No private right. | Subscription flag can be stored as provenance + audit; layoff notice workflow can use workflow lineage events. | Implement checkout disclosure + HR notice process by Oct 1 2026. Plan AEDT program for Oct 2027. |
| Florida | HB919 political ads + 836.13 altered sexual depictions | In force (conduct-triggered) | AI political-ad disclaimers, deepfake intimate-image bans. | Purge/tombstone takedown + certificate as removal proof. | Add disclaimer renderer in ad flow, takedown SOP wired to purge. |
| Washington | SB5838 Task Force | Mar 18 2024 | Study/report only, no private duty. | None required. NIST map reusable. | Track task-force output, no filing due. |

Watchlist (no deployer duty yet): Virginia HB2094 vetoed 2025 (expect 2027 reintro), New Jersey A3854 hiring bias-audit proposed (NYC-style). Treat as plan-ahead, not backlog.

## Status snapshot (2026-09-11) — live now vs scheduled

Live and enforceable today: TX TRAIGA, CA SB53/AB2013, CA SB942 (provider tier), UT SB149, IL HB3773, NYC LL144, TN ELVIS Act, FL deepfake/election rules. Scheduled: CT general duties Oct 1 2026; CA ADMT business compliance + CO SB26-189 Jan 1 2027; CT AEDT duties Oct 1 2027; CA risk-assessment filings Apr 1 2028. Watch with counsel: CO enforcement-stay litigation, any federal preemption ruling.

## The other ~40 states: narrow deepfake / election bucket

As of mid-2026 every state has introduced AI bills, 145 enacted in 2025, but outside the table above the enacted pattern is narrow: nonconsensual intimate imagery takedown, election candidate-impersonation disclaimer windows (often 60-90 days pre-election), voice-cloning (TN ELVIS Act Jul 1 2024), plus AZ/MI/MN/TX/WA election variants, NJ deepfake enacted, MA/MD study commissions.

Component posture for all of them: same takedown primitive (locate/purge/tombstone/certificate) + provenance to prove origin + audit to prove when. Operator wires two things per state where they operate: (1) disclaimer copy in the generating surface, (2) takedown clock SOP pointing at /dsar purge. No per-state code fork needed. Check NCSL database + legislature page quarterly; deepfake windows move fast.

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
  hiring notices (IL), checkout/HR notices (CT Oct 2026), candidate
  AEDT notices (NYC 10 business days, CT Oct 2027).
- [ ] Opt-out and human review paths exist in your app (CA ADMT, CO).
  The server provides the evidence; the buttons live in your surface.
- [ ] Impact assessment written and filed per calendar (CO Jan 2027,
  CA risk assessments Apr 2028). Trace + retention report are inputs,
  not the assessment itself.
- [ ] NYC bias audit hired yearly with published summary (LL144).
  No component substitutes for the independent auditor.
- [ ] Dates re-checked quarterly against primary sources (legislature
  pages, AG offices, CPPA). This file is dated 2026-09-11; statutes and
  stays move.
