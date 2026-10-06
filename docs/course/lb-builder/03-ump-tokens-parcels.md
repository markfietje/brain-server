# Lesson 3: UMP, capability tokens, and parcels

**Level:** B · **Time:** about 25 minutes · **Commands on two throwaway instances**

## Questions people ask

**What is UMP?** The Universal Memory Protocol: a small, documented HTTP
surface so memory clients and servers interoperate. This server reports
`"UMP 1.0 / L3"` conformance (that L3 is a UMP level, not a course level),
verified by a CI conformance gate, not asserted in a slide deck. The
[UMP page](../../universal-memory-protocol.md) is the contract.

**How do two brain-servers exchange memory?** Parcels: signed exports of
APPROVED rows.

```bash
# Side A: export (quarantined rows never leave; the crossing is ledgered)
brain parcel export --domain acme --out acme.ump

# Side B: import. Note --expected-signer is REQUIRED, the server refuses
# without it (400 signer_required). An import that cannot name whose
# signature it expects cannot detect the wrong signer.
brain parcel import --file acme.ump --domain acme \
  --expected-signer did:key:z6Mk...
brain parcel ledger
```

Two laws to build around: imported rows land as PENDING PROPOSALS (even
verified imports get the human look), and provenance marks (Ed25519
signatures over export artifacts) mean "this came from server X" is a
cryptographic statement.

**What are capability tokens?** Scoped UMP credentials: mint one, hand it
to a tool or integration, and its power is baked in (read-only exists).
`brain ump keygen` starts you. Combine with the two-token posture: the
operator token is yours, the agent gets the least capability that works.

## The MCP surface

For tool-style integrations the server also speaks MCP, stateless, with a
scope switch: `read` (recall only) or `full` (write verbs too). Default
`full` for compatibility, but you should pin `read` for anything that does
not write. The tool catalog is compile-time pinned, and a scope violation
refuses at dispatch time, before any network seam.

## Exercise

1. Run two throwaway instances (two DB paths, two ports).
2. Export a parcel of 3 approved rows from A, import into B, confirm all
   three arrived as pending proposals. Approve one. Notice B's approval is
   a NEW human decision by B's operator: provenance traveled, authority
   did not.
3. Tamper with the parcel file (edit a byte inside the content). Import
   with the correct expected-signer. Read the refusal. That refusal is the
   whole point of signing.
4. Mint a read-only capability token and attempt `/ingest/proposal` with
   it. Record the refusal code for your integration's error handling.

## Next

[Lesson 4: The injection screen, from the builder's side](04-the-screen-for-builders.md)
