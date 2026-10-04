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

## The `publish` capability — a named deny-only class

`publish` is **not** in `CAN_ACTIONS`. `Role::validate` rejects any `can` item
outside that list, the only production writer of the `roles` table validates,
and the migration seeds thirteen fixed presets — none carrying `publish`.

**Therefore no role row can hold `publish`, and KCS article publication is
impossible for every role-bearing principal, including `admin`.** Only role-less
JWT principals and the unconfigured superuser can publish.

This round does **not** fix it: the fix is minting `publish` into
`CAN_ACTIONS`, which the frozen-vocabulary rule forbids. It is declared in
`DENY_ONLY_CAPABILITIES` and the premise is pinned
(`r47_publish_is_a_deny_only_handler_seam_capability`) so the class cannot
outlive its justification quietly.

**The false precedent, recorded because the round nearly inherited it:** the
obvious argument for pinning this as intended is that `workflow` is the same
class. It is not. `CAN_ACTIONS` names `workflow`, and `workflow-operator`
grants it. Two in-tree comments claimed otherwise and were wrong.

## The thirteen fixed presets (`src/role.rs::PRESETS_RAW`)

Seeded `INSERT OR IGNORE` at migration (operator edits survive a re-migration),
parse-validated by `all_presets_parse_and_validate`:

| Preset | Shape |
|---|---|
| `admin` | Full control: every scope, every action (incl. `admin`, `purge`), all tools |
| `solo` | SMB owner: the `admin` action set over all data, every panel (the simplest default) |
| `agent` | Front-line worker: own private memory only, `read`/`write`/`reject`, UMP recall/get/feedback tools |
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

## The posture knob

`BRAIN_RBAC_ROLELESS_POSTURE` = `pass` (default) | `deny`.

`pass` is the shipped back-compat: a principal with no roles passes role gates.
`deny` is the opt-in for a deployment that has minted roles at its IdP and wants
a role-less token to get nothing. An **unknown value refuses boot** (the
`BRAIN_WRITE_POSTURE` pattern). The resolved value is printed at boot and
echoed on `explain`.

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
