# Authorization: the RBAC evaluation middleware

**Status:** shipped. **Scope:** route-level evaluation against the frozen role
vocabulary. **Not in scope, and stated up front:** per-record ACLs, SCIM, SAML,
group hierarchies, and **row-level tenant isolation** — `tenant_id` is
audit-scoping and DSAR partitioning, and no row-level isolation exists anywhere
in this server. Do not read the word "tenant" in this document as isolation.

## What runs, and where in the chain

```
security_headers → rate_limit → jwt_auth → opaque_auth → **rbac** → CatchPanic → Timeout → handler
```

`.layer()` applies **bottom-to-top**: a *later* source line runs *earlier* at
request time — so `TimeoutLayer` (the earlier line in `router/mod.rs`) runs
*inside* `CatchPanicLayer`, and a handler timeout surfaces as a caught panic
boundary response, not an opaque connection drop. The RBAC layer is therefore
registered after the CatchPanic line in `router/mod.rs` so that it runs *after*
authentication. Getting this wrong is silent — a layer above the auth layers
never sees a `Principal` and decides on every request without one — so the
ordering is pinned **by line number**, not by a comment
(`r47_the_rbac_layer_sits_between_auth_and_the_handler_layers`).

It is applied with **`route_layer`**, not `layer`. A bare `.layer()` also wraps
unmatched paths, which would turn this repo's probe-blind 404s into 403s across
the whole surface.

## The decision

`src/authz/policy.rs` is a pure function. No database, no clock, no transport
type, no `unsafe`. The whole policy surface is a `const` table that only a code
change can move — there is no expression language and no operator-editable rule.

```rust
pub enum Verdict { Allow, Defer(DeferReason), Deny(DenyReason) }
```

Three arms, not two. Collapsing "not this layer's decision" into either Allow
or Deny is a bug in both directions: a middleware that treats an earlier
decision as its own either overrides authentication or silently re-derives it.

| Deny reason | Meaning |
|---|---|
| `route_ungated` | A matched, non-public route with **no row in the gate table**. This is the round's reason for existing: coverage is now a property of the running server, not a claim about a test. |
| `method_not_permitted` | The row exists; this method is not one it covers. An empty method set is fail-closed, never "all methods". |
| `capability_deny_only` | The route's capability is in the frozen deny-only class. |

| Defer reason | Meaning |
|---|---|
| `public_path` | The authentication layer already exempted it. |
| `presentation_gated` | A declared exemption with no table row **by design** (see below). |
| `preflight` | A CORS preflight. |
| `no_principal` | **No `Principal` in extensions — and this is NOT a denial.** |

### Why `no_principal` defers rather than denies

The opaque **operator** token authenticates without inserting a `Principal` at
all (`server/router/auth.rs` calls `next.run(req)` directly), and two shipped
pins in `tests/authz_matrix.rs` require that token to keep reaching Admin
routes. Denying the absent extension would fail the round's own KILL condition
on the first run. Authentication has already ruled by the time this layer runs.

**Disclosed ceiling:** the RBAC layer does not gate the opaque operator path.
That is the v1.1 superuser back-compat law, it is load-bearing, and it is not
something this round changes.

## The declared exemptions

Seven registered non-public routes carry **no** `AUTHZ_GATES` row by design:
`/health/db` and `/auth/logout` (the verified bearer *is* the gate), and
`/audit/export`, `/compliance/evaluation-record`, `/compliance/inventory`,
`/ropa`, `/ropa/{id}` (registered only under the `compliance-pack` feature, so a
table row would be vacuous in a default build).

They live in `authz::gates::PRESENTATION_GATED`, and `tests/main_suite.rs`
reads **that** list. This moved from a test-local const during the round, because
the first cut of the middleware denied every non-public ungated route, refused
`/health/db`, and moved an `authz_matrix` row from 200 to 403.

## What the middleware does NOT enforce

- **The per-route role capability.** It cannot move here: the KCS publish gate
  is conditional on a request **body** field (`kind == kcs_publish`) inside
  `/proposals/{id}/approve`, a route that also requires `approve` and has a
  `remedy` branch. A middleware keyed on `(MatchedPath, Method)` cannot see a
  body. Declaring a per-route capability would deny *every* approval on that
  path.
- **The scope action.** The authz matrix already pins every table row to the
  `authorize()` literal its handler actually calls, so the row and the handler
  agree by construction. Re-deriving it would be a second opinion on a decision
  already made once.
- **The agent principal class.** Measured: `/reindex` is an `Admin` row, yet
  the agent receives a **200 soft-deny**, not a 403. The agent's per-route
  posture is the heterogeneous union of the matrix's `ROLE_GATED_FOR_AGENT`,
  `SOFT_DENY_LEGACY` and `LAYOUT_CONDITIONAL` lists — it is *not* derivable
  from the action column. Reproducing it in production would mean shipping a
  second copy of a test-side list. The agent is still refused: by the handlers,
  across every gated row, pinned by the matrix.

The handlers' `authorize` / `authorize_role` remain the inner gate (defence in
depth). The middleware is an outer filter over a property that could not
otherwise be enforced.

## The `publish` capability

`publish` **is** in `CAN_ACTIONS`, so `Role::validate` accepts it and a role can
hold it. It is granted deliberately to four seeded presets — `admin`, `solo`,
`supervisor`, `controller`, the ones that already carry `approve` — and to no
others.

**Approval does not imply publication.** Accepting a draft into memory and
publishing it as an external-facing article are different acts with different
consequences, so they are different capabilities: a role that does the first is
not thereby granted the second, and removing `publish` from a role's `can`
stops publication for it immediately.

The deny-only class (`DENY_ONLY_CAPABILITIES`) is now **empty**, and it stays
declared: an empty list makes "nothing is deny-only right now" a stated fact
rather than an omission a future edit can fill in silently. The pin asserts
BOTH halves — the vocabulary names `publish` AND the class does not — because
either alone is half a truth.

**What this reversed.** The capability was previously unnameable: `Role::validate`
rejects any `can` item outside `CAN_ACTIONS`, the only production writer of the
`roles` table validates, and no preset carried it — so publication was
impossible for every role-bearing principal (including `admin`) while every
role-less principal passed. That was a defect, filed rather than absorbed, and
it is now closed.

**The false precedent, recorded because the original round nearly inherited
it:** the obvious argument for treating `publish` as deny-only was that
`workflow` is the same class. It is not. `CAN_ACTIONS` names `workflow`, and
`workflow-operator` grants exactly `can:["workflow"]`. Two in-tree comments
claimed otherwise and were wrong.

## The thirteen fixed presets (`src/role.rs::PRESETS_RAW`)

Seeded `INSERT OR IGNORE` at migration (operator edits survive a re-migration),
parse-validated by `all_presets_parse_and_validate`:

| Preset | Shape |
|---|---|
| `admin` | Full control: every scope, every action (incl. `admin`, `purge`), all tools |
| `solo` | SMB owner: the `admin` action set over all data, every panel (the simplest default) |
| `agent` | Front-line worker: reads and writes across the shared memory pool — `owner_filter:"all"`, no owner restriction (the gateway's standing identity sees legacy NULL-owner rows too), `read`/`write`/`reject`, UMP recall/get/feedback tools |
| `workflow-operator` | Governed workflow execution without administrative or publication authority: `can:["workflow"]` only |
| `supervisor` | Call-center lead: sees their agents' rows, approves/rejects their queue, can export (DSAR) but not purge |
| `qa-specialist` | Reads agent work + calibrates; cannot approve or purge |
| `clinician` | Min-necessary PHI: own private memory, read/write, no review |
| `dpo` | DSARs: read + `dsar_export` + calibrate, no routine write |
| `recruiter` | Per-candidate private memory + team pools, uses the review queue |
| `controller` | Daily operational control: broad actions incl. `purge`, retention enforcement |
| `exec` | Read-only dashboards, no write or destructive actions |
| `client-auditor` | A client's compliance login: READ-ONLY on exactly one client domain (the min-necessary wedge) |
| `bpo-ops` | Read-only capacity/connector/queue/breach board across all clients |

The capability vocabulary (`CAN_ACTIONS`) is: `read`, `write`, `approve`,
`reject`, `calibrate`, `release_quarantine`, `dsar_export`, `purge`, `admin`,
`workflow` — and NOT `publish`.

## No-role JWTs vs the role-based shared pool (R85)

A JWT **without** a `roles` claim is owner-bound: its record gate carries its
own subject as the owner predicate, so it reads its **own** private rows and
is denied another subject's private rows on every read surface (recall,
suggest, by-id, multi-get, UMP get, procedures, verify — and, since the
read-gate completion round, legacy `/search`, the graph trio
`/graph/entity`//`graph/relations`//`graph/traverse` (each walked edge's
knowledge row), and `/decision/{id}/evaluate`). An absent/empty
subject reads no private rows (fail closed). Domain/team scopes still follow
the signed JWT scopes; admin and loopback/opaque operator access are
unchanged.

The **`agent` preset is the explicit exception, not the default.** Its
`owner_filter:"all"` deliberately opens the shared pool across owners
(including legacy NULL-owner rows) so the gateway identity can work. Do not
mistake a no-role JWT for the agent role: no-role means owner-only, the
`agent` role means shared-pool by design. Narrowing the agent preset to
owner-only is a separate policy decision and release, not part of this fix.

The **same record gate binds the UMP any-id mutations** (R93): `/ump/revise`,
`/ump/forget`, and `/ump/feedback` may only target a row the caller's gate
admits — the identical `(owner, access_scope)` pair the read surfaces filter
on. A refused target answers exactly like a missing id (probe-blind 404): a
write surface that distinguishes "exists, not yours" from "not there" would
be an id-existence oracle for private data. **Hard forget is the `/purge`
act** and carries its destructive authority on top: Admin scope AND the
`purge` role capability. A UMP capability bearer can never reach hard erase —
no admin verb exists in the §5.2 vocabulary — while its write-verb soft
lane is unchanged, as is the loopback/opaque operator path.

## The posture knob

`BRAIN_RBAC_ROLELESS_POSTURE` = `pass` (default) | `deny`.

`pass` is the shipped back-compat: a principal with no roles passes role gates
**except** the two that dispose of a proposal (see below). `deny` is the opt-in
for a deployment that has minted roles at its IdP and wants a role-less token to
get nothing — every role gate AND every record read (its gate compiles to the
empty permit: no row matches at any owner or scope). An **unknown value refuses
boot** (the `BRAIN_WRITE_POSTURE` pattern). The resolved value is printed at
boot and echoed on `explain`.

## A role-less token cannot dispose of a proposal

`approve` and `reject` are **role acts**. A principal whose `roles` claim is
empty is refused on both under *either* posture — the knob above is not the
thing that switches this off, because the failure it closes needs no
misconfiguration: a write-scoped claim-less token could otherwise call
`/ingest/proposal` (a Write) and then `/proposals/{id}/approve` (also a Write,
whose role gate passed on the empty claim) and promote memory with zero humans,
quorum 1, and a principal-independent digest. The seeded `agent` role already
excludes `approve`, so this closes the claim-less gap only; every role-bearing
principal is unchanged, and the opaque/loopback operator (`None`) is not
role-gated at all.

Both guards live in one seam (`authorize_role`), next to the
approval-capability list, so they cannot drift apart.

`GET /ops/authz/explain?route=…` reports the **action/scope** gate's verdict
and echoes the posture. It does not compute the role layer: a route's role
capability is not derivable from its path (`/proposals/{id}/approve` also
demands `publish`, conditionally on the body), so the receipt stops at the
seam it can measure and says which seam that is.

## `tools_allowed` is enforced

The role field is no longer a description. Every UMP verb entry point —
`ump.remember`, `ump.get`, `ump.recall`, `ump.revise`, `ump.forget`,
`ump.feedback` — runs `authorize_tool` beside the gates it already had, so a
role that grants no `ump.forget` is refused there even when its `can`
allowlist grants `write` and its scope grants Write on the domain.

The semantics, in one place because no call site invents its own:

- **Several roles are the union** of their tool sets.
- **`"*"` is every tool.**
- **A role that does not declare the field is unrestricted.** Narrowing is
  opt-in; the field was never enforced, so an absent set must not silently
  disarm a hand-made role.
- **A role-less principal is untouched** — the action seam governs that class,
  and the two must not disagree about who they govern.

The practical effect on the seeded presets: the `agent` role reaches
`ump.recall`, `ump.get` and `ump.feedback` and is refused at remember, revise
and forget. That is what the preset said all along.

## What `read` no longer implies

Two surfaces outside ordinary retrieval, closed together because they leak the
same thing by different routes:

- **`GET /export`** is the portability path and it is §2.7's row decision, not
  its field decision. A row the caller does not own — in `knowledge` **or** in
  `proposals`, which are unapproved memory bodies — is replaced wholesale by
  `{"redacted": true}` (title, source and owner included: a title is often the
  sensitive part), and the graph is narrowed to the edges and entities the
  surviving rows reference. A `withheld` object counts what was taken out per
  collection, so the envelope never under-reports itself. The operator export
  is unchanged — nothing withheld, every row verbatim, because a redaction that
  reached loopback would silently break every backup.
- **`GET /proposals`** is owner-bound: the record gate's resolved owner set
  rides into the query, so a principal with no roles reads its own queue and a
  role whose `owner_filter` opens the shared pool still reviews everyone's.
- **`GET /quarantine`** and **`GET /decayed`** are review POSTURES, not reads —
  both walk content across every owner (content the screen judged unfit, and
  content past its expiry). They answer loopback/opaque operators and
  principals holding Admin on their own tenant, which is exactly what
  `review_flags_allowed` already means; every read- or write-scoped JWT is
  refused.

## Denial audit rows

One `audit_events` row per denial: `AuditKind::Auth`, `AuditStatus::Denied`,
the closed reason, the method, the matched route pattern, the `mask_sub`-hashed
subject (12 hex) and the tenant. A denial writes no business row, so the audit
row stands alone — there is no caller's transaction to share, and that is
disclosed rather than hidden. The row **never** records what another principal
could have done.

## `GET /ops/authz/explain?route=&method=`

Admin on global, through the existing `authorize` seam. Returns the caller's
**own** verdict and a closed reason.

- It refuses a `?roles=` set with `400 authz_explain_role_set_refused`. Given a
  role set, "which gates would these clear" is the most useful reconnaissance
  tool an attacker has, and this surface refuses to be it. The cost is slower
  support tickets; the asymmetry is the point.
- An ungated route is a probe-blind `404`.
- The Admin gate is consulted **before** any query validation. A *required*
  query parameter fails at the extractor, before the handler body, which would
  hand an unauthorized caller a `400` that proves the route exists.
