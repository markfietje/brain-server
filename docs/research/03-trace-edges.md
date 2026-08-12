# TRACE Typed Edges + Faithful Explanation Paths

**File:** `src/trace.rs` (vocabulary + bounds) · `/graph/traverse?explain=true`

## The problem

A graph retriever that returns `1 -> 5 -> 9` is useless: it gives no *reason*.
An agent that answers "why?" needs typed, bounded hop chains —
`A --works_at--> B --ceo_of--> C` — and the traversal must be validity-aware
and bounded so a dense graph cannot blow the budget.

## The reference

**arXiv:2607.00339 (TRACE)** — hierarchical nodes + typed edges + validity-aware
traversal. The reasoning chain is a first-class artifact, not a side effect.

## The implementation

`src/trace.rs` provides a **prefix vocabulary** (`update:` / `supersedes:` /
`contradicts:` / `causes:`) and hard bounds `MAX_HOPS = 4`, `MAX_VISITED = 256`.
`/graph/traverse`:

- is **validity-aware** (`?at=`, bi-temporal filters on every hop);
- is **cross-domain** capable (`?cross_domain=true` fans out per domain);
- with `?explain=true` returns a `paths` array of **structured hop chains**
  `[{from:{id,name}, relation, to:{id,name}}, ...]` — the recursive CTE carries
  `relation_type` per hop — so a consumer can render the reasoning verbatim.
- `?kind=<rel_type>` filters edges (exact or `prefix:`), with LIKE-injection
  escaping on user input.

## Measured ceiling

- `causes:` is a **subgraph filter, not a causal claim**. The roadmap rule is
  explicit: *a graph path is association unless an intervention-ready causal
  model and domain-expert validation exist.* brain-server reports what the
  graph contains, never what is true in the world.
- Intermediate entity names are best-effort (seed + leaf named; intermediates
  surface as ids unless resolved via `/get/{id}`).
- The node-hierarchy reservation (`node_kind`, `parent_id`) exists but nothing
  populates session/topic yet.

*See the "faithful explanation" post in the blog — this is the "show the path,
don't assert the answer" principle.*
