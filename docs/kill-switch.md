# Principal kill-switch

Revocation is identity-wide and immediate. One call names a principal;
from that moment its cards fail verification, its dispatches are refused,
and its in-flight runs are cancelled. Re-provisioning the same name does
not resurrect it.

## HTTP

- `POST /ops/agents/revoke` (Admin on global) takes `principal` (max 256
  chars) and `reason` (max 500). Returns the principal, the revocation,
  and how many runs were drained.
- `GET /ops/agents/revocations` (Read on global) pages the registry,
  newest first, capped at 500 rows.

## What actually happens

The revoke upserts one registry row (latest wins), writes a hash-chained
audit row, and cancels the principal's active runs through the normal
compare-and-swap path, each cancellation carrying a `delegation/revoked`
lineage event. All three land in the caller's transaction or none do.
Card verification checks the registry before signature work, so revoked
cards fail fast and probe-blind; delegation dispatch and result handling
re-check at decision time. If the drain hits its page cap, a
`drain_incomplete` audit row names the remainder instead of pretending
the drain finished.

Bearer tokens for a revoked subject get `401 identity_revoked` on every
route, including the public refresh path. Console actors mapped to a
revoked principal are refused before capability checks.

## Limits

This revokes brain-server principals, not JWTs at the identity provider
(a separate layer with its own revocation list). The registry is one row
per principal; per-agent attribution inside a shared identity is not
modeled.
