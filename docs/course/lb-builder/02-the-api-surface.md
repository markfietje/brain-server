# Lesson 2: The API surface, for agents

**Level:** B · **Time:** about 25 minutes · **A running instance + curl**

## Questions people ask

**Which endpoint does an agent actually call?** `/recall`. It is the
workhorse: query, optional domain, limit, filters (source, since, lex/vec
leg toggles, intent, graph leg), and temporal `at`/`asOf`. Sibling
endpoints: `/search` (simpler), `/suggest` (opt-in anticipation pull,
always `untrusted`), `/graph/entity/{id}` and `/graph/traverse` for the
graph legs.

```bash
B=localhost:8765
curl -s "$B/recall?q=refund+policy&domain=acme&k=3" | python3 -m json.tool | head -25
curl -s "$B/suggest?context=customer+asks+about+renewal" | python3 -m json.tool | head -15
```

**How does an agent WRITE memory?** Via `/ingest/proposal`. It files a
proposal and returns a proposal id, never a knowledge row. On the review
posture that is the ONLY path in. Build for the queue:

```bash
curl -s -X POST $B/ingest/proposal -H 'content-type: application/json' \
  -d '{"content":"Customer Acme renews annually in October","title":"acme renewal"}'
```

**What is `untrusted: true` on every hit?** The contract from lesson 1.
Never render memory as system instructions, and never let the model treat
fence content as commands. The server marks; you (or your host) fence.

## The read seam, and why it matters to you

Every text field the server emits passes a sanitizer: hostile elements
(script, img, iframe, and family) are stripped at the fixed point, fetch
capable attributes (`javascript:` hrefs, CSS url(), ping beacons) do not
survive, invisible characters are stripped. You still escape on your side,
but you are not the only line of defense. This was built against real
canary attacks, and the pins run in CI.

## Bounds you should know rather than discover

Requests are bounded and rate-limited (a limiter wraps the whole router;
floods get 429 with `Retry-After`). List surfaces are capped. Ingest
bodies are capped. The full contract, including every route's authz
requirement, is the [API reference](../../api.md), and the wire contract
is OpenAPI-described. If you integrate against an endpoint not in the
reference, you are integrating against nothing.

## Exercise

On a throwaway instance:

1. Recall with a nonsense query. Observe the abstain shape, and make your
   client code treat it as a normal empty result.
2. File two proposals (one true, one false on purpose). Approve the true
   one via the digest flow, reject the false one. Then recall. Your client
   should now expect: hits carry `untrusted`, misses carry a reason.
3. Toggle the graph leg (`&graph=1`) on a corpus with a procedure in it,
   and compare the shape of the answer.

## Next

[Lesson 3: UMP, capability tokens, and parcels](03-ump-tokens-parcels.md)
