# Lesson 2: Who gets in

**Level:** L2 · **Time:** about 25 minutes · **Commands against a throwaway instance**

## What you will do

Set up access the way the system wants it: strong, boring, and split by
power level. Then learn the rotation and revocation verbs, including the
one refusal that looks like a bug and is a design decision.

## The simplest posture: one bearer token

One secret string in the token file. Every request carries it
(`Authorization: Bearer ...`). Fine for a single trusted operator on one
machine. The moment an assistant or a second human enters the picture, you
outgrow it, and the next section is why.

## The two-token posture: person and assistant

The token file can hold TWO lines. Line one is the operator token: full
power, the console, approvals, exports, purges. Line two is the agent
token: it can remember, revise, recall, and little else. It cannot approve
its own proposals, which closes the most obvious loop in the design, the
assistant quietly ratifying its own captures.

The assistant in your chat integration should be configured with the AGENT
token, never the operator one. This is not paranoia, it is the entire
boundary between "the machine proposed" and "a person decided", made
physical. If the assistant ever misbehaves, its token is revocable without
touching yours.

A historical note worth knowing: this split was not always enforced, and
the fix was an actual incident finding (an assistant found running with the
operator token). The tooling now expects the split, and drift from it
shows up in the audit trail fast.

## Rotating credentials, three verbs that matter

```bash
# 1. Rotate the bearer token. Fresh 32-byte secret, written safely,
#    and the RUNNING server picks it up within seconds. No restart.
brain token rotate

# 2. Generate JWT signing keys (RSA-2048, if you run JWT mode with an IdP).
brain key generate
brain key list
brain key prune --keep 3

# 3. Rotate the UMP operator signing key (provenance marks on exports).
brain key rotate
```

When to rotate: on any suspicion, on personnel changes, and on a schedule
you pick and actually keep. Rotation here is cheap by design. The token one
does not even interrupt service.

## The kill switch, and its one honest refusal

JWT identities and agent tokens are revocable mid-flight: revoke, and their
in-flight sessions die within the revocation window, future requests are
refused. There is a revoke verb on the API for exactly this.

Now the refusal. If you try to revoke the OPRATOR bearer, the server says
no, with a message naming the reason: a static token has no identity row to
revoke, so a "success" reply would be a lie during exactly the moment you
need truth, an incident. The remedy it names is the real one: rotate the
token and restart. This refusal replaced an earlier behavior where the
revoke verb SAID success while the operator credential kept working. If you
ever meet an old deployment notes doc claiming otherwise, trust the server.

## What the audit adds

Every authentication failure lands in a feed the Security panel shows, and
every credential event, rotation, revocation, is on the chain. When someone
asks "was the token changed before or after the weird Tuesday traffic", the
answer is one query, not archaeology.

## Least privilege, in one paragraph

Give read-only access to whoever only needs to read. The system has
capability levels for exactly this, down to read-scoped MCP access for tool
integrations. The pattern that ages well: start everyone at the least power
that unblocks them, raise deliberately, and let the audit trail show the
raises. The pattern that ages badly is a shared admin token "just for now",
which is how "just for now" becomes the incident report.

## Exercise

On a throwaway instance with a two-line token file:

1. Confirm the agent token cannot approve a proposal. File one (lesson 3 of
   Level 1), then attempt the approve call with the agent credential. Read
   the refusal and its reason.
2. Rotate the operator token (`brain token rotate`). Immediately make a
   request with the OLD token: refused. With the NEW one: fine. No restart
   happened.
3. Try to revoke the operator identity through the revoke API. Read the
   refusal message all the way through. It names the remedy.
4. Check the auth-failure feed in the Security panel and find your own
   failed request from step 2.

## What you learned

- Two tokens, two power levels. The assistant gets the agent one, always.
- Rotation is cheap and token rotation is hot, no restart.
- Agent and JWT identities are revocable; the static operator bearer is
  not, loudly, and rotation is the remedy.
- Auth failures are a feed, not a mystery.

## Next

[Lesson 3: Domains and tenants](03-domains-and-tenants.md)
