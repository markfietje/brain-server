# From twelve products to one (a preview of Profiles)

*2026. Forward-looking: describes the planned v1.21.0 "Profiles" release, not a shipped capability.*

*Status: shipped in v1.21.0 — see [`docs/configuration.md`](../configuration.md) for the real knobs; this preview is kept for the record.*

A memory store ships with knobs. Ours has a lot of them — access scope, PII
mode, per-kind retention, audit level, allowed memory kinds, connectors,
legal-hold defaults. That's the honest cost of being configurable enough for
compliance: a healthcare deployment and a call-center deployment and a
developer-tool deployment genuinely need different postures.

But a wall of knobs is a product that says "figure it out." Twelve different
deployments shouldn't mean twelve different learning curves.

## The idea: a Profile is a posture, and posture is the product

**Profiles** (planned v1.21.0) turns the configurable surface into a small set
of **use-case postures** — the "90% solution" that turns "twelve products" into
"one product, twelve postures." A Profile is a JSON bundle of the *existing*
knobs, stored as one row per domain/tenant and applied at ingest and retrieval:

```
profile = {
  access_scope, pii_mode, per_kind_retention,
  audit_level, allowed_kinds, connectors, legal_hold_default
}
```

No new schema columns — a Profile just *picks* values the system already
understands. That's the design constraint that keeps it honest: we're not
adding capability, we're making the capability you already have **discoverable
and repeatable**.

## Why this is the right 90%

- **Onboarding wizard** — an operator answers five questions ("what industry,
  what data sensitivity, who uses it, what should be gated, how long to keep")
  and gets a Profile pre-filled from real defaults. The wall of knobs becomes a
  guided conversation.
- **Consistency** — the same industry deployment gets the same posture, because
  the Profile is a repeatable bundle, not tribal knowledge.
- **Audit-ready** — a Profile is a documented, reviewable artifact: "this
  deployment runs the healthcare Profile," which the audit trail can show.
- **De-risks tenancy** — a Profile per tenant (v2.0) is the natural unit of
  isolation.

## The honest framing

This is **forward-looking**. Profiles is planned v1.21.0; none of it is
shipped code. We flag it here because the *design* is what we want feedback on
now — before we build it. The configurable surface it packages already exists
(v1.14/v1.15); Profiles is the ergonomic layer on top.

**The takeaway:** the difference between "a powerful memory store" and "a
product" is whether the power is usable. If you have a deployment we should
build a Profile for — or think a knob is missing from the bundle — tell us
before v1.21.0, so the "90% solution" is built on real postures, not guesses.

*See the roadmap's v1.21.0 "Profiles" row. The knobs it packages are the ones
documented in `COMPLIANCE.md` (access scope, PII, retention, audit).*
