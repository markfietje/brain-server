# The PRF Gate + Evidence-Faithful Snippet (grounding the answer)

**File:** `src/search/mod.rs` (`prf_should_expand`, `highlight_ranges`) · `Evidence`

## The problem

Two failure modes plague hybrid recall: **query expansion that never fires** (a
gate that compares against an unreachable threshold is dead code) and
**unfaithful snippets** (a result that highlights text it doesn't contain, or a
snippet the server fabricates).

## The reference

- **Pseudo-Relevance Feedback (PRF)** — the classical Rocchio/expansion idea:
  use the top pass-1 results to expand the query. The lesson from v0.9.x: the
  gate must be **reachable**, not decorative.
- **Faithful evidence** — the "with_snippet" invariant: a snippet is a verbatim
  substring of the source, and highlights are byte-offset ranges *within* it.

## The implementation

1. **Reachable PRF gate** (`v0.9.1`): `prf_should_expand` fires expansion only
   when the top pass-1 result appears in **both** dense and lexical lists within
   a bounded rank — cross-retriever agreement, so expansion never fires on
   noise. The prior gate compared an RRF-fused score against an unreachable
   `0.3` (top RRF ≈ `2/60 ≈ 0.033`) and never ran. Anti-injection guardrail
   skips quarantined rows.
2. **Evidence with highlights** (`v0.9.5` M2): every result carries an
   `Evidence { text, line_start, line_end, heading_path, source_uri,
   revision_id, highlights }`. `text` is a verbatim substring of `content`
   (never synthesized); `highlights` are byte-offset `[start,end)` ranges
   **within the revealed snippet** so they can never point past what's shown.
   The server never injects HTML. `source_uri` + `revision_id` (v0.9.4 source
   linkage) form a stable, dereferenceable link to the exact source revision.
   `enrich_evidence` is one batched LEFT JOIN, not N queries.

## Measured ceiling

- PRF is a **deterministic, agreement-gated** expansion — no learned expansion
  model. The anti-injection guardrail keeps quarantined content out of the
  expansion terms.
- Highlights are on the snippet window (redaction by design); a client wanting
  highlights over the *full* chunk calls `/get/{id}`.
- Legacy pre-v0.9.4 rows carry `None` source linkage (graceful), so their
  `source_uri`/`revision_id` are absent — the "unlinked chunk" ceiling.

*The `Evidence` shape is what the `/ops` and `/register` console surfaces render —
provenance as the retrieval primitive.*
