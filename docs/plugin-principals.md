# Plugin principals — a plugin is a PRINCIPAL, never code

**The boundary, first because everything else follows from it: a plugin
principal authorizes DATA access; it never authorizes code execution; the
kernel does not execute caller input.** An extension is a kernel CLIENT —
its own credential, a scoped role; the kernel's row-level authz IS the
sandbox. Nothing extension-authored ever runs inside the kernel (plan 22 as
amended by M3-f's R6; plan 27 is this page's implementation, M3-g).

There is no plugin runtime here, no loader, no sandbox to escape. A
"plugin" in this architecture is: one DOMAIN (isolated corpus + retention),
one PRINCIPAL (a scoped role + explicitly named tools), and the plugin's
own HTTP/MCP client speaking the kernel's existing wire from the machine
that hosts it.

## The recipe

1. **Create the domain.** Domains are the isolation boundary (per-domain DB
   law, 0600): `brain setup <domain>` provisions it; ingest that names the
   domain creates it implicitly. The plugin will see rows in ITS domain and
   — day zero — nothing else.

2. **Grant the principal.** On the machine that owns the token files (the
   same posture as `install-service.sh`):

   ```sh
   brain principal grant --name edu-plugin --role plugin --domain education \
       [--tools ump.recall,ump.get] [--ttl-days 90] [--out PATH]
   ```

   This writes three facts: a 0600 token file (the raw bearer — it is never
   printed, never stored in the database; the roster holds only its
   SHA-256), a `principals` roster row (name, role, domain, tools, created,
   expires), and a hash-chained audit row. The granted role must already
   exist in the roles store — a grant names an existing posture, never an
   invention.

3. **Point the plugin's client at the kernel.** The plugin speaks HTTP from
   its host to the kernel's loopback address with the minted bearer:

   ```text
   Authorization: Bearer <token from the 0600 file>
   ```

   MCP clients mount the same credential against the `ump.*` tools. The
   bearer authenticates through the kernel's plugin lane: hash → roster →
   expiry → kill-switch, per request, and lands as a typed principal the
   existing gates already govern.

4. **What the grant opens — and what it can never open.** The ship-with
   `plugin` role is a NARROWING of `agent` along every dimension the agent
   names:

   | Dimension | `plugin` | The law |
   |---|---|---|
   | `can` | `read`, `write` | Its own rows, its own domain. No `approve`, no `reject`, no `purge`, no `console`, no `shared_pool` — a plugin that reads the shared pool is the `agent` preset wearing a costume, and the absence is recorded as a named reason on `SHARED_POOL_ROLES`. |
   | `scopes` | `domain` | No `private` rows beyond its own writes, no `team` rows. |
   | `owner_filter` | `self` | The owner fence holds: the plugin reads rows IT wrote. |
   | `tools_allowed` | `[]` — EMPTY | NO UMP/MCP tool until the operator names it. `--tools ump.recall` at grant time is the only widening, each name an explicit operator act, enforced at the UMP verb seam and consulted only for plugin-kind principals (a JWT subject that collides with a roster name inherits nothing). |

   Day zero the plugin can do nothing beyond scoped read/write of its own
   rows in its own domain. Every capability above that line is something an
   operator typed.

5. **The footprint query.** "What did plugin X touch?" is answered by the
   chain, not by a new telemetry:

   ```text
   GET /audit?principal=edu-plugin     (Admin bearer)
   ```

   The filter reads rows at the SQL layer beside `kind`/`tenant`; the chain
   verdict is untouched by it. In the shell: **Governance → Roles → audited
   footprint**, which links the audit surface with the filter pre-seeded —
   the audit surface honors `?principal=` directly, the thing the retired
   GUI's URL promised and the kernel never shipped until now.

6. **The revocation runbook.**

   ```sh
   brain principal revoke --name edu-plugin [--reason "operator revoke"]
   ```

   The kill-switch row lands in `revoked_principals` (the same table the
   mesh and the agent lane consult) with its audit row, and **the minted
   token 401s `identity_revoked` on the next call** — the live proof shape
   of the 2026-09-09 agent-token enforcement. `--ttl-days` grants expire on
   their own (`identity_expired`). Two laws the runbook must respect:

   - **A revoked name stays dead across a re-grant.** The kill-switch wins
     at request time; revive nothing. The remedy for a compromised
     principal is a NEW name (and a new token file).
   - **The token file is yours to delete.** Revocation kills the credential
     server-side; the 0600 file on disk is inert afterwards — remove it
     like any dead secret.

## What this deliberately does NOT do

- **No HTTP mint route.** Minting is an operator act on the machine that
  owns the token files; an HTTP mint is a privilege-escalation surface with
  zero measured need. The refusal is a fact about the tree, pinned by
  source scan (`no_principal_http_route_exists`).
- **No `shared_pool`.** The agent's cross-owner read is the one posture a
  plugin never inherits; widening it is a decision that needs its own
  recorded reason, never a convenience.
- **No code execution, ever.** The boundary sentence at the top of this
  page is the ceiling, not a configuration.
- **No new HTTP surface for the roster.** `x-api-version` does not move for
  the mint; the one wire addition in this design is the audit filter above,
  additive, and the drift gate rode its own commit.

## Where the teeth live

- The preset and its day-zero pin: `src/role.rs`
  (`the_plugin_preset_starts_able_to_do_nothing`).
- The roster, the lane, and the CLI cores: `src/auth/principals.rs`.
- The 0600 writer: `src/secret_file.rs`
  (`the_writer_creates_owner_only_and_refuses_a_clobber`).
- The live round-trip, the cross-domain refusals, and the tool-reach pin:
  `tests/authz_matrix.rs`.
- The register row with the red-proofs: `docs/gate-law-register.md`
  (M3-g).
