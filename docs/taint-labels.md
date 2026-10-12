# The capture taint (origin labels)

brain-server's information-flow posture is deliberately minimal: ONE
boolean-grade label — **owner-authored vs captured-from-channel-traffic** —
not a lattice, not a policy engine. There is no auto-promotion to protect:
novelty, conflict and salience rank candidates, and only the human gate
promotes. The label exists so a human reviewer, a downstream model, and an
exclusion posture can all tell captured traffic from authored memory.

## Writing the label

`origin_context` accepts exactly `owner` (default) or `channel` on every
surface that takes it — `/ingest`, `/ingest/proposal`, `/procedure` — and
an unknown value is `400 invalid_origin_context`, never a silent
"not tainted". `channel` stores the row with origin `channel-capture`.

For the agent class the label is **derived, not asserted**: the seam
(`create_proposal` / the ingest core) stamps `channel` regardless of what
the caller claimed, because everything an agent writes IS capture.
Non-agent callers' assertions stand.

## Carrying the label

A label that lives at one surface is a decoration, so it rides every
content-returning wire:

- `/recall` and `/search` hits (`origin`, `untrusted: true`)
- `/get/{id}` and `/multi-get` rows
- `/suggest` hits (so the exclude posture acts on anticipation too)
- graph entity/relations edges (`origin` per edge)
- UMP records (`origin`, plus `untrusted: true` on the capture value) —
  id-safe, because the content-addressed id hashes `domain\0text` only
- the DSAR Art-15 bundle (per row, plus a `by_origin` census)
- the plugin's tools: recall (fenced), `brain_memory_get` (labeled +
  exclude-aware), graph relations (per-edge marker)

**Traverse is the stated ceiling**: a hop aggregates multiple edges, so a
per-path label would be ambiguous by construction. The edge labels are the
carriers there.

## Revisions inherit

`/ump/revise` builds its revision from the superseded row's emitted record —
which now carries the origin — and the merge treats `origin`/`untrusted` as
server-authoritative (the same skip-list as `id`/`integrity`). A patch
cannot strip the taint; only the capture value is honored on the way back
in, so a UMP peer can label its own traffic but can never assert owner-grade
provenance through the field. Export → import round-trips the label.

## Acting on the label

The openclaw plugin's `untrustedOrigins` posture has two values:

- `label` (default): captured hits ride into model context behind the
  untrusted-content fence with a `[memory | channel-capture]` prefix.
- `exclude`: captured hits are dropped from auto-inject AND from the tool
  results (`brain_memory_recall`, `brain_memory_get`) — captured memory
  never reaches that agent at all.

## The ceilings, stated

- Bare URLs in prose survive the read seam de-linked (linkified-inert) —
  the strip targets fetch primitives (markdown images, hostile elements),
  not prose.
- `/export` (JSON) ships stored content byte-verbatim by the portability
  decision: the label travels with it, never a sanitizer. The `ump-md`
  projection DOES ride the read seam — it is a rendered document.
- Graph traverse hops are unlabeled (above).
- The screen is a tripwire, not a boundary: the boundary is the human gate.
