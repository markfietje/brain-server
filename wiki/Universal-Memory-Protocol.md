# Universal Memory Protocol (UMP 1.0)

**Universal Memory Protocol** is an open standard for portable agent memory. The spec lives at [github.com/edihasaj/universal-memory-protocol](https://github.com/edihasaj/universal-memory-protocol). Brain Server implements it end to end, so memory written by one UMP agent can be read, verified, and reused by another, without a shared database or vendor lock-in.

This page explains what the Universal Memory Protocol is, what Brain Server supports, and how to use it.

## Why a memory protocol exists

AI agents accumulate memory in their own private formats. One agent stores notes as JSON, another as markdown files, a third inside a proprietary API. Move between agents or between tools and the memory stays behind.

The Universal Memory Protocol fixes that the way HTTP fixed web pages. It defines:

- **A record format.** Every memory is a record with a kind (semantic, episodic, procedural, working, identity), a body, timing, scope, and provenance.
- **A stable identity.** Each record gets a content-addressed id, `urn:ump:<hash>`, so the same memory has the same id everywhere.
- **Integrity.** Records can be signed by the owner's key, so a reader can prove the record is authentic and untampered.
- **Bindings.** The same records move over HTTP, as MCP tools, and as plain files (markdown or JSON).

Brain Server speaks all three bindings, so it can act as any agent's portable memory shelf.

## What Brain Server implements

Conformance is verified against the reference suite (`@universalmemoryprotocol/core`
1.0.0): **13/13 checks, UMP 1.0 / L3** on a fresh keyed instance, re-run by CI on
every push (the `ump-conformance` job asserts the badge line). The level
definitions map to brain-server as follows:

| Level | What it means | Brain Server status |
|---|---|---|
| L0 | Portable records over file bindings | Full |
| L1 | Server read/write operations | Full |
| L2 | Record integrity with content hashing | Full |
| L3 | Local integrity layer: signatures and capability tokens | Full |

When an operator key is configured, `GET /ump/capabilities` reports `conformance: "L3"`. Without a key the server reports `"L2"`, which is what a reader should expect: all the operations work, records are hashed, but signatures and tokens are not in force.

The handshake endpoint is public, so any client can ask before it starts:

```
curl http://127.0.0.1:8765/ump/capabilities
```

```json
{
  "server": { "name": "brain-server", "version": "1.17.5" },
  "ump": "1.0",
  "conformance": "L3",
  "kinds": ["semantic", "episodic", "procedural", "working", "identity"],
  "bindings": ["http", "mcp", "file"],
  "retrieval_signals": ["similarity", "recency", "salience", "scope_match", "provenance_depth"],
  "max_recall": 50,
  "writable": true,
  "audit": true
}
```

## Quick start

The fast path has three steps.

**1. Create the operator key.** This gives the server an identity and enables level 3.

```
brain ump keygen
```

This writes an Ed25519 seed to `~/.config/brain-server/ump/operator.key` (0600 permissions, the same posture as the JWT keys) and prints the public identity:

```
wrote UMP operator key /Users/you/.config/brain-server/ump/operator.key
did: z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN5x1fTDDgQ
```

Set `BRAIN_UMP_KEY_DIR` to put the key somewhere else. The server picks up any seed file in that directory. The `did:key` form is the `0xed 0x01` Ed25519 multicodec prefix + base58btc, and the leading `z6Mk…` prefix is fixed for Ed25519 keys (the remaining characters vary by key).

**2. Write a memory.**

```
curl -X POST http://127.0.0.1:8765/ump/remember \
  -H "Content-Type: application/json" \
  -d '{"ump":"1.0","kind":"semantic","body":{"text":"The release ships on Friday."}}'
```

```json
{ "id": "urn:ump:3dbd637652cbe621", "result": "created" }
```

**3. Recall it.**

```
curl -X POST http://127.0.0.1:8765/ump/recall \
  -H "Content-Type: application/json" \
  -d '{"ump":"1.0","query":"release date","limit":5}'
```

```json
{
  "results": [
    {
      "record": {
        "id": "urn:ump:3dbd637652cbe621",
        "kind": "semantic",
        "body": { "text": "The release ships on Friday." },
        "integrity": { "content_hash": "blake3:<base32>", "signature": "ed25519:<base64>", "signer": "did:key:z6Mk..." }
      },
      "score": 0.03,
      "signals": { "similarity": 0.03, "recency": 1.0, "salience": 1.0, "scope_match": 1.0, "provenance_depth": 0 }
    }
  ]
}
```

Recall runs the same deterministic retrieval pipeline as the normal `/recall` endpoint: local static embeddings, hybrid vector plus lexical search, graph rescue, and fusion. There is no LLM in the loop and no per-query cost.

## HTTP operations

The full surface is ten routes under `/ump/`.

| Route | Purpose |
|---|---|
| `GET /ump/capabilities` | Handshake and conformance level. Public. |
| `POST /ump/remember` | Store a partial record. Returns `{id, result: created\|merged\|rejected}`. |
| `GET /ump/memory/{id}` | Fetch one record by id. Integrity is verified before the record is returned. |
| `POST /ump/recall` | Ranked retrieval with per-result signals. |
| `POST /ump/revise` | Patch a record. Creates a new version and supersedes the old one. |
| `POST /ump/forget` | Erase a record, soft or hard, with a tombstone and an audit row. |
| `POST /ump/feedback` | Tell the server whether a recalled memory was followed, overridden, ignored, or contradicted. |
| `GET /ump/subscribe` | Server-sent event stream of changes. Events carry `{kind, id}` only, never record bodies. |
| `POST /ump/audit` | Read the hash-chained audit log. |
| `GET /ump/audit/verify` | Verify the audit chain is intact. |

A discovery document with the same payload as capabilities is served at `/.well-known/ump.json`.

### Consent

A record may declare a `scope.owner`. When it does, the owner must match the authenticated principal. When it does not, the record is owned by whoever wrote it. A mismatch is refused with a `forbidden_scope` error, so one user cannot silently write memory into another user's scope.

### Batch ingest

The export side always accepted batches. The import side accepts them too:

```
curl -X POST "http://127.0.0.1:8765/ingest?format=ump" \
  -H "Content-Type: application/json" \
  -d '{"ump":"1.0","records":[{"ump":"1.0","kind":"semantic","body":{"text":"One."}},{"ump":"1.0","kind":"procedural","body":{"text":"Two."}}]}'
```

Each record is processed independently and gets its own status, so one invalid record never aborts the rest. A single-record batch keeps the plain reply shape from earlier versions.

## MCP tools

The MCP server mirrors the HTTP surface, so an MCP-capable agent talks to Brain Server without writing HTTP.

- `ump.capabilities`
- `ump.remember`
- `ump.get`
- `ump.recall`
- `ump.revise`
- `ump.forget`
- `ump.feedback`
- `ump.audit`
- `ump.audit.verify`

These are thin proxies over the same handlers, so behavior is identical on both bindings.

## File binding

Memory is portable as plain files, which is how the Universal Memory Protocol moves between machines and tools without any server.

**Export everything as one markdown document:**

```
brain ump export --format md --out memory.ump.md
```

Each record becomes a front-matter block plus a body. The export also supports `--format ump` for the JSON envelope.

**Import it elsewhere:**

```
brain ump import memory.ump.md
```

The same formats work over HTTP for tools that do not use the CLI: `GET /export?format=ump-md` and `POST /ingest?format=ump-md`.

Round-trips are lossless for the fields the projection carries: id, kind, scope, time, lifecycle, and title.

## Identity and capability tokens

Level 3 adds a key and tokens.

- **Identity.** The operator key is an Ed25519 key. The public identity is a `did:key` value printed by `brain ump keygen`. Records written while a key is configured carry a signature under `integrity`, which lets any reader verify the record really came from this server and was not tampered with.
- **Capability tokens.** A token is a compact signed bundle with verbs (`read`, `write`, `derive`, `export`), a scope, and an expiry. Present it as a bearer token on the UMP routes:

```
Authorization: Bearer <token>
```

The server checks the signature and expiry at the middleware, then checks verbs and scope per operation. A read-only token cannot write. A token scoped to one project cannot touch another. Expired tokens get a 401. There is deliberately no admin verb, so a capability token can never reach the audit administration surface.

Tokens are self-issued: the operator signs tokens for peers. There is no third-party identity provider and no verification registry, which keeps the whole thing runnable offline.

## Security notes

- Record bodies are treated as data, never as instructions. The server verifies before it emits and filters by scope before ranking, which is the order the recall pipeline already uses.
- Clients that render memory should do the same: parse the structure, never execute or interpret a record body as a command channel.
- The key file is 0600 and the directory 0700, the same posture as the JWT signing keys. Rotation is delete and regenerate; old tokens stop verifying immediately.

## Conformance and honest limits

- Conformance is **suite-verified, not self-attested**: the reference
  conformance runner scores 13/13, UMP 1.0 / L3 against a fresh keyed
  instance, and CI re-runs it on every push (asserting the `UMP 1.0 / L3`
  badge line so the README badge cannot go stale). The suite assumes a fresh
  store — rerunning against a persistent DB reports `merged` on
  `L1.remember` (content dedup by design); the runner's correct target is a
  throwaway keyed instance with a fresh DB, same as the reference
  `ump-serve`.
- Level 3 covers the local integrity layer. Agent-to-agent federation, remote agent identity, and per-tenant key hierarchies are future work.
- The subscribe stream is a change signal. Live record streaming over the wire is federation work.
- The `did:key` emission is Ed25519 only, the same documented posture as the JWT EC/Ed gap.

## Related pages

- [API Reference](API-Reference) and the runtime `GET /openapi.yaml` for the full contract
- [Security](Security) for key storage and token rules
- [Governance & Compliance](Governance-and-Compliance) for the integrity and consent controls map
- [Roadmap & Release History](Roadmap-and-Release-History) for the v1.17.3 UMP Rollout release and the v1.17.4/v1.17.5 conformance + eval-fix releases
