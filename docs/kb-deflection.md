# KB deflection — measuring demand reduction honestly

The KCS Evolve practice closes with a measurement question: did publishing
knowledge actually reduce demand? Two signals exist in brain-server, and they
are NOT equally strong.

## Primary metric: repeat-contact rate (CRM-sourced)

`repeat_contact_rate_units` on `/workflow/scoreboard`, aggregated from CRM
case envelopes (Bridges). This is the demand metric: if customers stop
re-opening tickets for symptoms that have published articles, it shows up
here. It is the number the weekly calibration report and the monthly human
sign-off carry as primary.

## Indicative metric: self-service deflection (on-page feedback)

Published pages built by `brain kb build` carry a "Did this solve it?" control.
An operator-hosted relay signs each vote (Standard Webhooks) and posts it to
`POST /webhooks/kb-feedback`; each verified delivery becomes one anonymous
`kb_feedback` finding row. The scoreboard derives:

- `self_service_deflection_units` — helpful ÷ total feedback × SCALE
- `kb_feedback_total` — total votes
- `kb_hot_topics` — published slugs whose feedback volume repeats
  (`KB_HOT_TOPIC_THRESHOLD`, default 3); a hot topic means "this symptom keeps
  coming back — article stale or missing", feeding the content-health loop.

**This number is indicative, not a savings claim.** It measures votes on pages,
not contacts avoided; selection bias (angry customers don't vote) and relay
placement both skew it. No industry lift percentages are claimed anywhere —
the repo's REALITY_CHECK rule applies to our own marketing as much as to
vendor decks.

Both signals land on the weekly calibration report and the monthly human
sign-off (the existing Leadership & Communication practice). The machine
computes counters; humans decide what they mean.

## Privacy posture

Votes are PII-free by construction: `{slug, helpful, day_bucket,
anonymous_id}` where `anonymous_id` is the RELAY's salted day-bucket hash
(salt lives in a 0600 file beside the relay). The raw IP never reaches
brain-server, nothing visitor-identifying is stored, and DSAR erasure has
nothing subject-specific to erase.
