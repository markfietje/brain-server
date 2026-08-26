# Keystone worked example — one case through the whole Order-of-Care loop

The series-exit gate for the v1.28.x line: one real case walked end-to-end
through every tier shipped in 1.28.22–1.28.36, with the commands an operator
actually runs. Every artifact below is deterministic — re-run it and the
outputs match (modulo timestamps).

## The loop

1. **CRM intake** (Bridges): a Zendesk ticket syncs in via
   `brain-connector-crm`; the body lands on the proposal path under review
   posture, one governed run opens per `case_ref`.
2. **Solve**: the run cranks its steps; evidence and checkpoints land on the
   lineage; recall events feed KCS's search-and-solve signal.
3. **Confirm-gate close**: `POST /workflow/runs/{id}/complaint/lifecycle`
   (or the outreach close gate) — the case closes only on customer
   confirmation or the documented three-attempt exception.
4. **Article** (Capture): the solve files a `kcs_*` capture proposal; approval
   promotes it to a draft knowledge row.
5. **Publish + translate** (Keystone G-B): `kcs_publish` publishes it;
   a human translates (`POST /kcs/translate`), approval writes the approved
   per-locale row pinned to `based_revision`; the build emits `de/<slug>.html`
   with hreflang alternates and a visible fallback note where untranslated.
6. **Status page** (Keystone G-A): mint the ref
   (`POST /workflow/runs/{id}/status-ref {"action":"mint"}`), ship the token
   by the closing note or CRM ticket field, then rebuild:
   ```sh
   brain kb build --domain <d> --out site/ --base-url https://kb.example.com \
       --with-case-status --locales en,de,fr,es,nl
   ```
   The customer sees `status/<ref>.json`: one of seven fixed words, a
   promise bucket from the SLA class ("expected within 72 hours"), one
   fixed-template sentence, a build stamp. No PII, no deadlines, no names;
   `/status/` is excluded from robots.txt and never appears in the sitemap.
7. **Feedback event**: the customer solves from the KB page; the feedback
   event counts as solved-proof deflection.
8. **Effort proxy computed with a re-ask** (Keystone G-C): the customer had
   also opened a duplicate ticket; the sync maps the merge into one
   `case/reask` event (`source: "crm_merge"`), or the operator marks it
   (`brain workflow note <run> <text> --reask`), or the derived heuristic
   proposes `case_merge_suggested` (exact hashed-subject match within
   `BRAIN_REASK_WINDOW_DAYS`) and the human's approval emits it. The proxy
   weighs it ×2; `reask_rate` reads re-asks over closed cases.

## Honest ceilings

- Static = build-cadence fresh: the status page stamps its build time; no
  live route exists and none is planned inside brain-server (loopback is law).
- brain never sends anything: refs, translation requests, follow-ups all ride
  humans or CRMs.
- Translation is a human act; the tool governs filing, staleness, and
  negotiation only.
- Duplicate detection is exact-hash only; no fuzzy matching exists.
- The effort proxy is defined and emitted but not yet wired into scorer
  gold-set families (the next scorer version consumes it).
