# Architecture

Brain Server is a single Rust binary that couples a **retrieval engine**, an
**embedding model**, a **knowledge graph**, and a **governance layer** behind a
versioned HTTP API. Everything runs in one process; the only external dependency is
an on-disk SQLite database.

## How memory moves — three nested loops

Troubleshooting here is not one process but **three loops turning at
different speeds**, converging on what ISO 10002, the KCS Solve loop, ITIL,
and COPC each describe separately:

```mermaid
flowchart LR
    subgraph L1["LOOP 1 · SOLVE (per case — minutes)"]
        direction LR
        A1["case opens"] --> A2["agentic crank:<br/>recall · reason · checkpoint"] --> A3["AskHuman when stuck"] --> A4["resolved + evidence"]
    end
    subgraph L2["LOOP 2 · EVOLVE (per pattern — days)"]
        B1["captured article<br/>proposed FROM the case"] --> B2["human approves by digest"] --> B3["published to KB"] --> B4["reuse counted ·<br/>freshness reviewed"]
    end
    subgraph L3["LOOP 3 · DEFLECT (per corpus — weeks)"]
        C1["published knowledge serves<br/>customers AND agents first"] --> C2["fewer repeat contacts"] --> C3["feedback + hot topics<br/>flag the gaps"] --> C1
    end
    A4 -- "capture" --> B1
    B4 --> C1
    C3 -.->|"gaps become new cases"| A1
```

Loop 1 never skips its human gate; Loop 2 exists only because Loop 1 left
evidence worth keeping; Loop 3 is why the knowledge base pays rent. The
rest of this page zooms into Loop 1.

## The governed agentic loop

The customer journey, the AI’s role, and the human’s role in one view.
The engine **cranks** through a bounded, checkpointed loop; when it runs
out of evidence it stops and asks one precise, digest-bound question —
it never guesses, and it never writes memory without the configured
gate in front of it.

> **Write posture, stated precisely:** with `BRAIN_WRITE_POSTURE=review`
> (recommended for teams; the installer provisions an agent token in this
> mode) every agent write to memory becomes a digest-bound proposal a human
> approves. Screened direct writes remain available under the default
> `open` posture when an operator explicitly chooses them. Either way:
> screened, provenance-stamped, audit-chained.

```mermaid
flowchart TD
    subgraph CUST["CUSTOMER JOURNEY"]
        C1["Customer has a problem"] --> C2["Opens ticket<br/>CRM · WhatsApp · portal"]
        C10["Resolved fast —<br/>or self-served instantly"] --> C11["Happier · no repeat contact"]
    end

    subgraph EDGE["GOVERNED EDGES — bridge processes holding zero brain tokens"]
        E1["CRM connector<br/>Zendesk · Salesforce · Genesys"]
        E2["Channel bridge<br/>WhatsApp · Slack · Teams"]
    end

    C2 --> E1
    C2 -.-> E2

    subgraph KERNEL["BRAIN-SERVER KERNEL (loopback · audited)"]
        direction TB
        I1["Case opens ONE governed run<br/>POST /workflow/runs"]
        subgraph LOOP["THE AGENTIC LOOP (bounded crank · checkpointed)"]
            direction TB
            L1["1 ASSEMBLE CONTEXT<br/>recall: vector + FTS + graph<br/>provenance-labeled · fenced"]
            L2["2 REASON AND ACT<br/>investigate · record findings<br/>evidence · confidence"]
            L3["3 CHECKPOINT<br/>durable state · resumable"]
            L4{"4 ENOUGH EVIDENCE<br/>TO DECIDE?"}
            L5["5 ASK THE HUMAN<br/>pending_question, digest-bound<br/>engine PAUSES — never guesses"]
            L6["6 RESUME AT CHECKPOINT<br/>answer verified against digest"]
            L1 --> L2 --> L3 --> L4
            L4 -- "no" --> L5
            L6 --> L1
            L4 -- "yes" --> L7
        end
        L7["7 PROPOSE — never write<br/>findings · draft answer · KCS article"]
        G1["HITL WRITE GATE<br/>human approves by digest<br/>quarantine- and legal-hold-aware"]
        K1["KNOWLEDGE PUBLISHED<br/>KCS article → static KB"]
        A1["EVERY STEP AUDITED<br/>hash-chained · tamper-evident · DSAR-erasable"]
        I1 --> LOOP
        LOOP --> L7 --> G1
        G1 -- "approved" --> K1
        G1 -.-> A1
        LOOP -.-> A1
    end

    subgraph HUMAN["HUMAN AGENT — owns judgment, not drudgery"]
        H1["Console · Slack · Teams<br/>review queue and case rooms"]
        H2["Answers the judgment call<br/>digest-bound approve / reject / edit"]
        H3["Talks inside the case room<br/>notes · skill invites"]
        H4["Shift handover<br/>I-PASS packet · one click"]
    end

    E1 --> I1
    E2 --> I1
    L5 -- "question surfaces where the agent already works" --> H1
    H1 --> H2
    H2 -- "POST /workflow/runs/id/answer" --> L6
    H3 --> LOOP
    H4 --> LOOP
    G1 --> H2

    K1 -- "serves the next customer" --> R1["RECALL WITH PROVENANCE<br/>approved knowledge only"]
    R1 --> C10
    K1 -.->|deflection measured on the scoreboard| C11
```

### Inside one crank cycle

```mermaid
flowchart LR
    S(["run open · SLA envelope stamped"]) --> W["WORK: one bounded step"]
    W --> R["recall context<br/>(provenance + fences)"]
    R --> T["think: finding? contradiction?<br/>evidence link? nothing?"]
    T --> REC["record to lineage<br/>(event · parent-linked)"]
    REC --> CK{"checkpoint due?"}
    CK -- "yes" --> CP["checkpoint event<br/>(state snapshot)"]
    CK -- "no" --> Q
    CP --> Q{"can decide?"}
    Q -- "yes" --> DONE["propose resolution<br/>→ HITL gate"]
    Q -- "no · blocked on judgment" --> ASK["AskHuman:<br/>pending_question + digest"]
    ASK --> PAUSE["engine STOPS here<br/>SLA clock keeps running"]
    PAUSE -- "human answers (digest verified)" --> W
    DONE --> CLOSE(["case closed ·<br/>knowledge captured"])
```

### Who does what — and why the human wins

| | The AI agent does | The human agent does | Benefit to the human |
|---|---|---|---|
| Investigation | Reads every past case, article, and graph relation; assembles evidence with confidence scores | Sees an assembled dossier, not twelve tabs | Minutes of digging become seconds of reading |
| Judgment calls | Detects it is stuck and asks one precise, digest-bound question | Answers once — in the console or from their phone via Signal/Slack | No guessing games: the machine knows what it does not know |
| Writing memory | Drafts the KCS article from the case's own recorded evidence | Approves or rejects by digest — nothing enters memory unreviewed | The knowledge base stays clean without being policed |
| Repetition | Cranks around the clock, resumes at checkpoints, never loses context | Handles exceptions and the customer relationship | Shift handovers take one click; context survives the shift change |
| Trust | Every action lands on a tamper-evident hash chain; content screened, fenced, provenance-stamped | Can prove to any auditor exactly what the AI did and who approved it | The AI is accountable by construction — safe to delegate to |

**The flywheel in one sentence:** every human-approved resolution becomes
retrievable knowledge, so the next customer either gets answered faster or
deflects to self-service entirely — and the scoreboard proves which happened.

> Rendering note: diagrams are fenced <code>```mermaid</code> blocks rendered
> client-side by the vendored `theme/js/mermaid.min.js` +
> `theme/js/mermaid-init.js` (no CDN, no CI preprocessor). To export a static
> PNG/SVG instead:
> `npx -y @mermaid-js/mermaid-cli@11 -i diagram.mmd -o diagram.svg -b white`.

---


---

## What’s inside the process

Same process, same SQLite — the loops above are the *control story*,
not a separate service:

```
                    ┌───────────────────────────────────────────────┐
                    │              brain-server (one process)        │
  HTTP clients ───▶ │                                               │
  (agent plugin,   │   ┌──────────┐   ┌───────────┐   ┌──────────┐  │
   brain CLI, MCP, │   │  Handlers│──▶│  Recall   │──▶│ SQLite   │  │
   Dioxus client)  │   │  (Axum)  │   │  Engine   │   │ (WAL)    │  │
                    │   └────┬─────┘   └─────┬─────┘   │  vec0    │  │
                    │        │ auth/AuthZ    │         │  FTS5    │  │
                    │        ▼               ▼         │  KG      │  │
                    │   ┌──────────┐   ┌───────────┐   └──────────┘  │
                    │   │ Audit log│   │ Static    │                 │
                    │   │ (hash    │   │ embeddings │                 │
                    │   │  chain)  │   │ (model2vec)│                 │
                    │   └──────────┘   └───────────┘                 │
                    └───────────────────────────────────────────────┘
```

### The layering law

Handlers are protocol adapters ONLY: parse → principal → authorize → one
`spawn_blocking` → domain call → read-seam shaping → response. ALL SQL, caps,
FK ordering, and invariants live in domain modules (`src/workflow/*`) that
take `&Connection` / `WorkflowTx` — never pool or HTTP types. Every mutation
emits its hash-chained audit row INSIDE the caller's transaction: a transition
and its evidence commit or roll back together.
Error paths deny loudly (fail-closed); silence is never certified. New code is
always a service core; see `docs/engine-sdk.md` for the stable engine ABI the
workflow cores compile against.

---

## Retrieval engine

Recall is **hybrid**: a vector leg and a lexical leg run concurrently on independent
pooled read connections and are fused.

- **Vector leg** — `sqlite-vec` (`vec0`) KNN over embeddings. Embeddings are
  computed in-process by the static `model2vec` model; vectors are int8/binary
  quantized (4–32× smaller) for edge memory bounds.
- **Lexical leg** — SQLite FTS5 (BM25).
- **Fusion** — Reciprocal Rank Fusion (`k = 60`), a deterministic, weight-free
  merge.
- **Expansion** — deterministic PRF (pseudo-relevance feedback) expands the query
  when the top pass-1 result appears in **both** dense and lexical lists within a
  bounded rank. It fires only on cross-retriever agreement, never on a fused score
  threshold alone.
- **Graph leg (optional)** — Personalized PageRank over the knowledge graph, opt-in
  via `?graph=true`, as a third RRF leg.

Every result carries **provenance**: per-retriever ranks, the fused score, any
expansion terms, and (optionally) a rerank score.

### Abstention

When retrieval quality is too low to support a claim, `/recall` returns
`{decision: "low_confidence", hits: []}` instead of top-1 garbage. This is driven
by a calibrated multi-signal recommendation (rank overlap, gap, lexical density) —
never a magic score cutoff.

---

## Ingest pipeline

1. **Markdown / structured / memory** ingest arrives at a handler.
2. Text is **chunked** with a CommonMark-aware splitter (heading-boundary splits,
   code-fence-safe, one chunk per `knowledge` row).
3. Chunks are **embedded** by the static model and written to `vec0`.
4. Text is tokenized into FTS5.
5. `[[relation::entity]]` links (and explicit entities/relations) build the
   **knowledge graph**.
6. **Temporal stamps** (`observed_at` / `valid_from` / `valid_to` / `authority`)
   and **source provenance** (`source` + immutable `revision`) are recorded.

Ingest is governed by a **write-back gate** (v1.14): a candidate can be scored
(novelty via KNN, conflict via consolidation, salience via heuristics) and held in
a proposal queue **without creating a `knowledge` row**. It becomes memory only via
human approval.

---

## Knowledge graph

Entities and relationships live in `entities` / `relationships` tables with a
four-timestamp bi-temporal model (`valid_at` / `invalid_at` + `created_at` /
`superseded_at`, v1.27.22). `/graph/traverse` walks the graph (bounded to depth
4, ≤256 visited) and, with `?explain=true`, returns **faithful hop chains**
(`A --works_at--> B --ceo_of--> C`) rather than a flat id string. Traversal
visits only *current* edges — a rewritten edge whose `superseded_at` is set is
skipped (a backdated correction no longer yields two live edges for one triple).

Graph edges are superseded two ways, both retire-never-delete:

- **Operator-approved `supersedes` links** (via `/consolidate`) atomically
  expire the prior fact (its `invalid_at` closes): historical recall
  (`?at=<past>`) still returns it, current recall does not.
- **Automatic on changed re-ingest** (v1.27.22): re-ingesting a relation with a
  different window sets the old edge's `superseded_at` (transaction-time end)
  and inserts the corrected version as the new current belief. The full version
  lineage is readable via `GET /graph/relationships/{id}/history`.

---

## Governance layer

- **Append-only audit log** — a keyed HMAC-SHA256 hash chain. Each row records
  a keyed MAC of the previous row over the full record, with a per-DB epoch and
  a pinned chain head (`/audit/verify`); pre-v1.27.31 legacy epochs verify as
  legacy (v1.27.31). Read events (recall/search/get) are opt-in.
- **Workflow governance** — governed runs on lineage events (branch-never-delete
  rewind), role-gated with audited transitions; the outcome scoreboard,
  monthly calibration signing, and since v1.28.34 the ISO 10002/10003
  complaint lifecycle: lineage-event state machine, HITL remedy matrix citing
  legal basis + published conduct clause, deterministic role-tier approval
  caps (over cap escalates exactly one level), national-body ADR packet per
  Reg. 2024/3228, goodwill ledger aggregating only audited remedies.
- **Prompt-injection quarantine** — suspicious input is stored but excluded from
  retrieval until reviewed.
- **DSAR / GDPR** — locate → export → purge → chain-verifiable deletion
  certificate (`POST /dsar`), plus a queryable `/tombstones` registry.
- **Calibrated abstention**, **span verification** (`/verify`), and **reviewable
  proposals** keep the memory honest without an LLM.
- **Read-seam sanitization** — every emitted text field passes redaction →
  markdown-reference strip (EchoLeak) → invisible-Unicode strip before leaving
  the server, so a stored chunk cannot smuggle context out through a rendered
  URL or bidi/zero-width trickery (v1.20.3 / v1.20.27).
- **Fail-closed bind + SSRF-hardened egress** — startup refuses a non-loopback
  bind without auth (v1.20.29); outbound webhook/alert calls follow no redirects
  (v1.20.26).

---

## Data storage

- **SQLite** in WAL mode, with `busy_timeout` so concurrent writers queue rather
  than fail.
- **`vec0`** for quantized embeddings; **FTS5** for lexical search; relational
  tables for the knowledge graph, sources/revisions, and governance.
- **Backup/restore** — AES-256-GCM encrypted, checksummed, excludes secrets.

---

## Multi-domain

Memories can live in scoped **domain databases** (health, business, code, …), each
with its own graph. Retrieval **auto-routes** by per-domain centroids and falls back
across domains on a miss, so one domain's memory never leaks into another's answers.
This is a v1.x foundation (see [Roadmap](./roadmap.md)).

---

## See also

- [Deployment](./deployment.md) — running, configuring, and backing up.
- [Security](./security.md) — the threat model and controls.
- The [API reference](./api.md) and the full [API_CONTRACT.md](./API_CONTRACT.md).
