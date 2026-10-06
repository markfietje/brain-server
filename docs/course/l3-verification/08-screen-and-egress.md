# Lesson 8: The screen pipeline and the egress posture

**Level:** L3 deep dive · **Time:** about 25 minutes · **Two attacks, run both**

## Questions people ask

**How deep does the injection screen actually go?** Layered, and each
layer is pinned by tests you can name in a report: multilingual blocklist
families (five languages, same intents), a typoglycemia anagram tier
(first and last letter with a sorted middle matches a keyword), a BOUNDED
decoder (base64/hex runs over a length floor, a fixed number of decode
passes, a decode budget, so decoder bombs cannot hang a write), invisible
character normalization before matching (a bidi-split keyword cannot
dodge the matcher), and hostile-element stripping with attribute tiers
(`on*` handlers by name, fetch-capable URL schemes and CSS url() by
value). The second-layer classifier is optional, auto-loaded, and refuses
to guess when absent: `/health/db` says which posture.

**How do I test it rather than believe it?** The canary corpus approach:
send a battery (element welds, mixed case, entity-encoded payloads,
zero-width stuffed instructions, CSS fetch attributes) and assert
quarantine or strip for each. The repo's own pins run exactly this
battery in CI, and the read seam has a parity fixture the plugin and the
server share. As an assessor, ask for the canary list and run a sample.

**What stops the server from being used to probe my network?** The egress
law: every outbound HTTP client (webhook sinks, connectors, provider)
resolves, validates against the IANA special-purpose address table
(loopback, link-local, CGNAT, metadata ranges, and friends), and PINS the
first resolution for the process lifetime, so a DNS rebind mid-flight
changes nothing. Redirects are refused outright (signed webhook headers
must never ride a redirect). A private-network sink without the explicit
opt-in refuses at BOOT. The one deliberate exception: the mediated
host-call path allows loopback targets by allowlist, because that is its
job.

```bash
# The boot refusal, observed in one line: point an alert sink at a
# private address without the opt-in and read the refusal.
BRAIN_ALERT_WEBHOOK_URL=http://192.168.1.1/hook ./target/release/brain-server
# refuses to boot, names the env, names the remedy
```

## What to record

For the pack: two canaries (one element, one encoding) with their
verdicts, one read-seam strip assertion, and the egress boot refusal. Four
artifacts, and they carry more weight in a review than any prose section,
because they are behavior, not description.

## Next

[Lesson 9: The authorization matrix and the gates](09-authz-and-gates.md)
