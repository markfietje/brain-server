# Lesson 9: The authorization matrix and the gates

**Level:** L3 deep dive · **Time:** about 25 minutes · **One matrix, one mismatch hunt**

## Questions people ask

**How is authorization decided?** A data table, not scattered ifs: every
route names its required capability and role set in a guard table, the
auth middleware resolves the principal (operator bearer, agent token, or
verified JWT with revocation checks), and the oracle evaluates. The table
is machine-checked from two directions: a floor on its row count, and a
behavioral matrix that drives the composed router with real principals.
Deleting a row fails the floor; deleting enforcement fails the drive test.

**What are the principal classes?** The operator bearer (static, full
power, unrevocable by design, rotation is the remedy), the agent token
(scoped write access, revocable mid-flight), JWT identities (jti + subject
revocation, capability claims), and capability tokens (scoped, signed,
per-integration). The console and channel edges map onto actors resolved
through the same machinery.

**How do I verify the matrix matches reality?** The mismatch hunt, and it
is genuinely fun with two terminals: pick five rows from the authz table
(`/authz` surface in the API reference names it), and for each, attempt
the route with a principal that SHOULD fail (read-only token on a write,
agent on an approve, nobody on an admin verb). Record the refusal codes.
Then find ONE thing a privileged principal CAN do that surprises you, and
check whether the table predicted it. The table always wins; when it does
not, you have found a real bug and the maintainers want it.

**What are the workflow gates?** Beyond route authz, the governed surfaces
carry semantic gates: approve demands the approve capability ON TOP of
write, channel sends demand window plus consent plus an approved
proposal, WFM skill changes land as proposals, and the replay gate refuses
delivery traces that diverge from their recorded evidence. Each gate
refuses with a named code, so your report can cite behavior, not intent.

## Exercise

1. With a read-only capability token: attempt `/ingest/proposal`, an
   approve, and one admin verb. Three refusal codes, three matrix rows.
2. With the operator bearer: succeed at all three. (The negative control,
   always.)
3. Read the authz documentation page and find the row for `/events`
   streaming. Explain why stream denial happens BEFORE the stream opens
   (HTTP 403, not a mid-stream apology), and why that ordering is the
   claim.

## Next

[Lesson 10: Determinism, evals, and self-measurement](10-determinism-and-evals.md)
