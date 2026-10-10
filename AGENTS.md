# Agent Execution Log — brain-server

> **R107 "Tools": `tools_allowed` stops being a description.** The field
> shipped stored-and-surfaced with enforcement deferred to v1.24, under an
> explicit "store now, enforce later" note. It never arrived: the UMP entry
> points read nothing, so every preset's carefully chosen tool set — `agent`:
> `ump.recall|get|feedback`; `supervisor`: plus `ump.revise|remember`; `exec`:
> `ump.recall` alone — described intent rather than policy. A role that grants
> no `ump.forget` could still erase a row through `/ump/forget`.
>
> **`authorize_tool` is the principal-side twin of `cap_gate`**: one
> predicate beside the gates the six entry points already run, no new
> mechanism. The semantics are decided once, in the seam, because a call site
> that invents its own is how a field becomes decorative a second time:
> several roles are the **union** of their grants; `"*"` is every tool; a role
> that does not DECLARE the field is **unrestricted** (narrowing is opt-in, and
> since the field was never enforced an absent set must not silently disarm a
> hand-made role); a role-less principal is untouched, because the action seam
> governs that class and the two must not disagree about who they govern.
>
> **The neighbour that keeps this honest:** the preset the matrix exercises
> changed behaviour with it. The `AgentLoopback` principal carries the `agent`
> role, whose tool set omits remember/revise/forget — so the agent class is now
> refused at those three UMP verbs, and the matrix says so
> (`ROLE_GATED_FOR_AGENT`). A field that is enforced and a matrix that still
> expects the old answers would disagree; both changed in the same commit.
>
> **ponytail:** no new tool vocabulary parsing, no MCP-protocol change, no
> per-tool granularity beyond the stored field, no role-vocabulary change; the
> stored fixtures are untouched.

> **R105 "Cleartext": a bearer stops leaving in cleartext — including by
> accident.** Three ways the operator's own configuration could put a
> credential on the wire in the open, none of which the tree refused.
>
> **(1) `https://` was a SILENT DOWNGRADE.** The dependency-free client
> (`bin_common/http.rs`, shared by `brain`, `mcp`, `bench` and the connector
> stubs) stripped the scheme and defaulted the port, so an operator who wrote
> `BRAIN_URL=https://brain.example.com` got a plain `TcpStream` to port 80 with
> `Authorization: Bearer <operator token>` on it — and no signal that the
> request they asked to be secure was not. It speaks no TLS, so the honest
> answer is a refusal: `HTTPS_BASE_REFUSED`, a FIXED string (a caller may match
> it, and a URL does not belong in a logged error).
>
> **(2) A non-loopback plain-HTTP base was a cleartext bearer by
> construction.** These binaries are loopback-only by contract — the steward
> harness already refuses non-loopback plain HTTP — so a remote authority now
> refuses with `REMOTE_BASE_REFUSED` too. Loopback is checked as an IP or the
> literal `localhost`, never as a name that "resolves locally": resolution is
> not this layer's job, and guessing would be a pin in the wrong direction.
> **This is a deliberate behavior change** for anyone pointing a client at a
> LAN server over plain HTTP: off-host means the harness, which can do TLS.
>
> **(3) A cleartext sink was accepted, and ambient proxy env could route around
> the DNS pin.** The alert and Art-19 DSAR sinks enforce no scheme, so a
> signed, subject-bearing payload could go out over `http://`. Both now refuse
> at the boot guard that ALREADY checks their secrets — and transport is
> checked FIRST, because an unsigned send over https and a signed send over http
> are the same leak with different paperwork. No opt-out: that is a posture an
> operator should not get to pick in an env var. The guard reads the live sink
> strings itself rather than taking presence bits, because a caller resolving
> the URL could pass `false`. And `hardened_egress_builder` now carries
> `.no_proxy()`, the same reason the provider client already did: reqwest
> honours `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`, and a proxy in the
> environment routes a PINNED sink through a third party and around its pin.
>
> **ponytail:** no TLS implementation in a dependency-free client — refusal,
> not a new dependency; no proxy support knob; no DNS change; loopback
> defaults are byte-identical (pinned); the refusals are fixed strings, and the
> sinks keep their existing resolve → validate → pin discipline.

> **R106 "Identity": agent writes are reviewed and labelled BY IDENTITY.**
> The human-promotion invariant was a deployment posture: `write_posture()`
> defaults to `open`, so all six agent-facing write surfaces inserted directly
> unless an operator had set `BRAIN_WRITE_POSTURE=review`. The installer writes
> `review` into a NEW plist — but a manual or default launch, which is how a
> development host and every `cargo run` starts, is `open`. Nothing in the
> server knew whether a write came from an agent, so the same binary promoted
> for one launch and proposed for the next.
>
> **`effective_write_posture(principal)` is now the single seam**, read by all
> six call sites (and only by them — a structural grep is the discipline, not
> the promise). A recognized agent class is proposal-only under BOTH postures:
> the typed `AgentLoopback` principal of the two-lane token file, or a JWT
> carrying the `agent` role. Everyone else sees the knob verbatim, so an
> operator's deliberate direct write still inserts under `open` — pinned in the
> same file, because "make agents propose" that quietly becomes "make everyone
> propose" is a different product.
>
> **The label follows the same seam.** `origin_context` on the plain-ingest
> path chose the row's taint label from the WIRE, so a channel-captured write
> could assert `owner` and land under operator-import provenance. For the agent
> class the server derives it (`channel`); the assertion only stands for the
> classes that are not agents. Derived after the closed-vocabulary check, so an
> agent sending `origin_context: "banana"` still gets the documented 400 rather
> than a silently-relabelled write.
>
> **The ceiling is stated, not implied.** A role-less JWT with write scope is
> NOT recognized as an agent class: the server cannot tell an agent from a human
> holding the same token, so a deployment minting agent JWTs without the `agent`
> role is relying on the posture knob — which still works, and is still the
> operator's call. Also found on the way: the Loom embed pre-pass read the RAW
> env posture, so the agent class would have paid to embed rows it never
> stores; it reads the resolved per-principal posture now.
>
> **ponytail:** the knob keeps its name, its `open` default and its meaning for
> everyone else — no posture rename, no new identity type, no quorum or UX
> change, no migration; `/add`'s manual-vocabulary law and the operator write
> path are untouched.

> **R104 "Identity": the MCP bridge stops inheriting the operator.** The `mcp`
> binary resolved its bearer as `BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` → the default
> install file and took the FIRST token — which in the installer's two-lane file
> (line 1 operator, line 2 agent) is the **operator's**. Every LLM tool call
> therefore arrived as the `None` principal: the superuser. `BRAIN_MCP_SCOPE`
> was a process-local NAME filter (`full` by default, set by nothing) standing
> between a model's tool call and the whole Admin surface — a Rule-of-Two
> violation dressed as a convenience default.
>
> **The lane is now chosen, and the agent lane is the default.** The token
> source ladder is untouched (same three steps, same order); only the LANE
> changed. `BRAIN_MCP_IDENTITY` ∈ `agent` | `operator`, parsed fail-closed like
> `BRAIN_MCP_SCOPE` — including rejecting an EMPTY value, because two knobs
> with different rules for `""` is how a typo becomes a silent posture. Under
> the default, a two-lane source yields line 2: the server authenticates it as
> the typed `AgentLoopback` principal and refuses Admin routes by class.
> `operator` is an explicit operator choice, not a removal — someone running
> the bridge as their own steward needs the Admin surface and says so.
>
> **The single-token deployment keeps working and NAMES its ceiling.** There is
> no agent lane to take, so the bridge takes the only token and the startup
> line says `identity=agent lanes=1 — single-token source, so the OPERATOR token
> is being presented …`: ambient authority, disclosed rather than discovered
> later, with `BRAIN_MCP_SCOPE=read` named as what bounds it. A missing
> identity is not a crash — a crash would push operators back to a hand-rolled
> env that skips the whole resolver.
>
> **The wiring is pinned, not just the resolver.** The resolver itself is pure
> (no env, no filesystem, no server), so the pins cannot pass while the code
> around them is wrong; a structural pin then reads the production region of
> `src/bin/mcp.rs` and fails if any request path calls `auth_token()` directly
> again — i.e. forgets the identity and slides back onto first-token-wins. It
> scans the region BEFORE `#[cfg(test)]` on purpose: the first cut counted the
> test module and matched its own text.
>
> **ponytail:** no server-side change (the typed `AgentLoopback` principal
> already existed and the middleware already classifies it), no auth-protocol
> change, no token rotation, no signing or pinning service, no installer
> artifact setting the new knob — `agent` is the default and `operator` is a
> per-host choice. No claim that this protects a compromised host: it removes
> the ambient superuser, nothing more.

> **R101 "Desk": the export and the review queue disclose to reviewers and
> owners only.** Four surfaces the read gate never reached, closed together
> because they leak the same thing by different routes.
>
> **`GET /export` was a field decision pretending to be a row decision.**
> Redaction replaced `knowledge[].content` and nothing else, so a foreign row
> still shipped its title, source, owner, origin and every other field — and
> `bundle.proposals` shipped VERBATIM, because the bundle's proposal
> projection never read the `owner` column the table has carried since the
> QaQueue migration. Observed red-first against the live tree: a foreign
> private row exporting `title: "Their private salary band"` with
> `owner: "user:them"` beside a `[redacted]` content, plus the foreign
> proposal body verbatim. A redacted row is now REPLACED by a bare
> `{"redacted": true}` stub — no id, because an id is itself a handle on a row
> the caller may not read — and a `withheld` object counts what was taken out
> per collection so the envelope never under-reports itself. The graph is
> narrowed by the ids the caller can see: an edge names its `knowledge_id`, and
> the entity names on it were extracted from the foreign content.
>
> **The three review listings.** `GET /proposals` is owner-bound — the record
> gate's resolved owner set rides into `pending_page` as an `owner IN (…)`
> predicate, reached through the same resolver every read surface uses rather
> than a second opinion about ownership. `GET /quarantine` and `GET /decayed`
> are review POSTURES: both walk content across every owner, and both now ask
> `review_flags_allowed`, which the tree already carried for exactly this
> decision (loopback/opaque, or Admin on the caller's own tenant). No new
> mechanism, no new role vocabulary, no new capability.
>
> **The neighbours are pinned, so "closed" cannot mean "closed to everyone".**
> The operator export is byte-identical (nothing withheld, every row verbatim —
> a redaction that reached loopback would silently break every backup); a
> shared-pool reviewer role still reviews the whole queue; an admin-scoped
> principal keeps both review scans; `/proposals` stays open to an owner-bound
> caller for its own rows. The authz matrix's read/write/agent cells assert the
> new denials through `REVIEWER_POSTURE_ROWS`, pinned against `AUTHZ_GATES` so
> a renamed route fails loudly instead of quietly losing its cell.
>
> **ponytail:** no schema change, no new route, no role-vocabulary change; the
> export envelope gains one additive `withheld` object and no format bump;
> `/clients/{name}/proposals` keeps its own `manages` fence rather than
> inheriting a second, unrelated owner predicate.

> **R102 "Lane": the Signal draft-approve command approves drafts.**
> `draft_proposal_row` selected `WHERE id=?1` with **no kind predicate**, so a
> `[draft N] approve <digest>` message could flip ANY pending proposal to
> `approved` — a `kcs_publish` article or a `complaint_remedy` marked decided
> while the branch that actually publishes or applies it never runs. Observed
> red-first: a pending `kcs_publish` proposal answered **200** through this
> lane. Now the kind travels with the row, the lane refuses anything that is
> not `kind='draft'` before it reads the content, and the refusal is a loud
> `409 proposal_not_draft` plus one audited `Denied` row. `approve_draft_tx`
> keeps the CAS and gains `AND kind='draft'` beside it, so the invariant holds
> even for a future caller that forgets the check; the digest bind and the
> concurrent-approve race are untouched and still pinned.
>
> **The audit row this lane claimed to write was being rolled back.** Both
> refusal arms recorded `Denied` THROUGH the transaction and then returned
> `Err`, so the un-committed transaction took the evidence with it — the
> digest-mismatch arm has always been a 409 with no audit row behind it. Both
> now drop the tx first and write on the connection, and
> `draft_approve_digest_mismatch_is_audited_as_denied` pins the corrected
> claim so the next arm added cannot inherit the mistake.
>
> **ponytail:** no schema change, no new route, no wire change beyond the
> refusal reason joining the existing 409 vocabulary, no new Signal command,
> no quorum redesign, no auth-protocol change on the webhook lane; the
> neighbours (digest bind, non-pending refusal, a real draft still approving)
> are pinned in the same module so "fixed" cannot mean "refused everything".

> **R103 "Seam": the role-less principal gets an answer, and approval
> becomes a role act.** `authorize_role` returned `Ok(())` for every
> principal whose `roles` claim is empty, so `BRAIN_RBAC_ROLELESS_POSTURE`
> was resolved, validated, printed at boot and echoed by
> `/ops/authz/explain` — and read by nothing. The knob was a reported
> configuration, not an authorization input. Two guards, one seam.
>
> **(1) The posture is finally an input.** Under `deny`, a role-less
> principal is refused at every role gate, AND `record_read_gate` compiles
> its record permit to the empty permit — no row matches at any owner or
> scope, which is what `docs/configuration.md` has always promised. The
> enforcement arm alone would have left the promise half-true: the gates
> refusing while every read surface still served the token's own rows. An
> unreadable posture fails closed (boot already refuses an unknown value,
> so reaching that arm means the environment moved under a live process).
> `pass` is byte-identical to before, and the opaque/loopback principal
> (`None`) returns above both guards.
>
> **(2) Approval is a role act.** `approve`/`reject` are refused for a
> role-less principal under BOTH postures — not a `deny`-only behavior a
> deployment could opt back into, because the chain it closes needs no
> misconfiguration. Observed red-first against the live tree: a claim-less
> write token called `/ingest/proposal` and then
> `/proposals/{id}/approve?digest=…` and got **200
> `{"status":"approved","chunk_id":1}`** — promoted memory, zero humans,
> quorum 1, principal-independent digest. Now 403, with the proposal still
> `pending` and no `knowledge` row written; `reject` 403s the same way. The
> seeded `agent` role already excludes `approve`, so this closes the
> claim-less gap only.
>
> **The blast radius was measured, not guessed.** Seven routes carry an
> approval-class role gate (`/proposals/{id}/approve|reject`,
> `/kcs/articles/{id}/approve`, `/workflow/runs/{id}/answer|steering|
> status-ref|rewind`), and the authz matrix's `write` class is deliberately
> role-less — so those rows now assert the refusal instead of the pass,
> under `APPROVAL_ROLE_ROWS`, which has its own pin
> (`approval_role_rows_are_real_write_rows`) so a renamed route fails loudly
> rather than quietly losing its cell. Two shipped pins that encoded the old
> rule were flipped honestly: the role-less arm of
> `authorize_role_gates_can_allowlist` (passes four non-approval gates,
> refused two approval ones) and the domain-scoped reviewer in
> `approve_reauths_row_domain_before_the_cas` now carries `solo`, so the test
> still measures the re-auth and not the role gate.
>
> **ponytail:** no schema change, no new route, no wire change (the explain
> receipt is scoped in place rather than gaining a field — its verdict is
> the ACTION gate's, and the role layer is not derivable from a path: one
> route's capability depends on the request body), no role-vocabulary
> change, no quorum change, no IdP requirement; R86–R90 stay parked; nothing
> here certifies compliance.

> **R100 "Reach": the read gate on the four surfaces that never lowered
> it — and R93 before it, target authority on UMP writes.** Both close
> authorization gaps found by the fresh 2026-10-09 security pass; both are
> red-first behavioural pins. No schema change, no new route, no wire
> change, zero new dependency edges.
>
> **R93 — a write verb was standing in for authority over a row.**
> `revise`, `forget` and `feedback` checked general Write and then mutated
> a caller-selected id, so any write-scoped principal could revise,
> supersede, soft-forget or hard-erase **any** row; a capability token
> carrying only a write verb reached the chunk-erase path at write scope
> (the MCP `ump.forget` seam). All three now resolve the record gate — the
> R85 seam, the same `(owner, access_scope)` pair every read surface
> filters on, keyed on the SQL columns and never the client-controlled
> `ump_meta` overlay — and refuse probe-blind, so the write surface is not
> an id-existence oracle for private data. `hard: true` is the `/purge`
> act and now says so: Admin scope **plus** the purge role capability, with
> a capability bearer always refused because no admin verb exists in that
> vocabulary, checked before row resolution so a 401/403 decides on the
> caller alone. Legal hold stays the second, independent fence inside the
> transaction. `feedback` joins the seam because `record_feedback` upserts
> on `(chunk_id, COALESCE(session,''))` — two principals' NULL-session
> feedback on one row overwrote each other, owner field included.
> Red-first: `tests/ump_target_authz_pins.rs` observed failing against the
> pre-fix tree before the fix.
>
> **R100 — nine read surfaces consumed the gate; four never lowered it.**
> Legacy `GET /search`, `/graph/entity/{name}`, `/graph/relations`,
> `/graph/traverse` and `POST /decision/{id}/evaluate` each read another
> subject's private rows for a no-role read JWT. `/search` assigns the gate
> into its lowered filters using the exact `/recall` idiom; the graph trio
> binds the owner/scope predicate at the `LEFT JOIN knowledge` seam the
> domain scope already binds, through one shared renderer, with traverse
> compiling it into **both** the seed and the recursive step so a foreign
> private edge neither renders nor is walked through; `/decision/{id}/evaluate`
> mirrors the `/procedure/{id}/steps` belt-and-braces. The same seam found a
> pre-existing defect: the traverse scope template formatted `?{ph}` with
> `ph` already carrying `?N`, so `??N` was a SQLite syntax error and the
> scoped walk had answered 500 since the `?N` refactor — an authz-matrix cell
> expecting "neither 401 nor 403" had been reading that 500 as a pass.
> Red-first: `tests/read_gate_completion_pins.rs`, observed red against the
> live leaks before the fix. Exit criterion was a route-table sweep, not an
> assertion: every Read row re-checked against the `knowledge` table, with
> the remaining ungated content surfaces recorded as the next round's scope.
>
> **ponytail:** no role-behavior change, no schema/route/wire change, no new
> query engine, no new dependency; R86–R90 stay parked under the operator's
> no-fork directive; nothing here certifies compliance.

> **R83 "Grammar" — the attribute tier learns the space around `=`.**
> Second round of the parallel programme (lane L2 developed in worktree
> `brain-L2`, L0 shipped; fork lane parked under the operator's
> zero-conflict directive — no `~/Sites/openclaw` edit that could
> merge-conflict with `openclaw/openclaw`). Closes the tenth pass's
> **F4-02** (HIGH) and **P4-02**; lands the **R9-02 falsification
> erratum**. No authz change, no route change, no wire change, no schema
> change (**1.32.26** unchanged), zero new dependency edges.
>
> **(1) F4-02 — one lookahead rejoins what the split tore apart.** The
> whitespace-only tokeniser handed `attr_is_hostile` `None` for every
> spaced `name = value` form, so all five whitespace forms survived
> byte-identical across all seven scheme attributes plus `style`
> (preflight probe: 28 survivals → 0 post-fix). Pins
> `spaced_equals_cannot_smuggle_a_hostile_attribute` +
> `spaced_benign_attributes_pass_through_verbatim`; the mutant
> (lookahead forced `None`) fails with 45 spaced forms survived.
> **Measured, not inherited:** only ` `/`\t`/`\n` were ever live —
> `\r`/`\x0c` already die upstream where the control-strip rejoins the
> tight form, and spaced `ping` already died by name (its URL residue
> rode pre-fix, killed post-fix). The fix is whitespace-agnostic, so
> all five die through one path regardless. Scope correction recorded:
> the plan text said the 7-name URL list carries `data`, the code says
> `background` — no gap, `data=` belongs to `<object>`, which the
> element tier owns.
>
> **(2) P4-02 — the shell twin carries the same fix and the same
> canaries** (`sweepAttributesInTags`/`sweepSurvivingTag`/`attrIsHostile`
> + entity/CSS decoders; `shell/tests/attribute-canary.test.ts` 3/3,
> `pnpm test` 85/85 across 19 files, `pnpm check` clean; the mutant
> kills the identical 45-count). The plugin tier is server-canonical by
> design and inherits via sync; the fork-sanitizer arm lives under K4-02
> (R92, parked with the fork lane).
>
> **(3) The R9-02 erratum.** R78's `CLOSED` row gains the falsification
> sentence: the `css_value_fetches` arm was unreachable through the
> tokeniser's own grammar until F4-02 — and THREAT_MODEL's Attrbane row
> is corrected in the same commit (stamp law).
>
> **Spire at ship:** `cargo test --lib gate::` 63/0; full
> `cargo test --features bench` 50 ok lines / 0 FAILED; fmt + clippy
> (`bench` + default) clean; `badges.sh --selfcheck` clean,
> `--verify-count` re-derived **3170** (+2, the round's own pins);
> `docs-truth.sh` LOW=17/MED=0; `env-truth.sh` clean; doc-links 547
> resolve. Register: F4-02 + P4-02 flipped CLOSED — R83 with evidence;
> `SHIPPED_ROUNDS` → `[&str; 15]`.
>
> **ponytail: NOT** the fork sanitizer gap (K4-02 → R92, fork lane's);
> NOT the `data`-attribute question (answered above); NOT R84's fork
> half (parked — K4-03/05/06/07/10/12, K8-11, K4-09 stay OPEN under the
> zero-conflict directive); no schema/route/authz change; zero new
> dependency edges.
>
> **R82 "Reach" — the live-exposure round: the gateway stops running the
> operator token.** First round of the parallel programme
> (`docs/EXECUTION_PLAN_20261007_PARALLEL.md`; lane L1 developed in
> worktree `brain-L1`, L0 shipped). Closes the tenth pass's **F4-01**
> (CRITICAL), **F4-10**, **F4-11**, and **P4-07**. No authz change, no
> route change, no wire change, no schema change (**1.32.26**
> unchanged), zero new dependency edges (all `Cargo.lock` files
> byte-untouched).
>
> **(1) F4-01 — the fork half, digest-verified.** The gateway wrapper now
> pins `BRAIN_TOKEN_FILE=$HOME/.config/brain-server/auth-agent-token`,
> unsets `BRAIN_SERVER_AUTH_TOKEN`, and `openclaw.json` carries no
> `authToken`; `scripts/secrets-truth.sh --selfcheck` (`9932a4ef`) exits
> 0 with auth line 1 (operator) digest `70fcdd05b6b8`, line 2 (agent)
> `7257711ce377`, effective gateway env `token_file=7257711ce377`,
> `token_var`/`server_auth` absent — digest-only output on every lane
> (the e748d760 #78 law). The hostile fixture — the machine's actual
> pre-fix wrapper (`.pre-r82`), driven on copies — FAILS the selfcheck
> naming the operator-digest match. The **live revoke drill was NOT
> run**; the evidence is the digest truth, and the round notes say so.
> **(2) F4-01 — the plugin half (`c78f3984`).** Rung 3 of the token
> ladder gains the multi-token refusal rungs 1–2 already carry; the pin
> `plugin_config_token_refuses_a_multi_token_value` runs in the fork's
> vitest runner (`1 passed | 15 skipped` by name) and is red-proven by
> stripping the refusal (`expected [Function] to throw an error`) — the
> S9-07 ceiling named, not silently worked around. **(3) F4-10
> (`9501bcf4`).** `BIND_PUBLIC` is value-read
> (`matches!(…, Ok("1") | Ok("true"))`); `cargo test --lib bootstrap`
> 9/0 with `bind_public_zero_does_not_opt_in_to_public_exposure` plus
> the needle-built wiring pin `the_production_bind_public_read_is_
> value_read`; red-proofs: call site reverted to `is_ok()` fails the
> wiring pin (8/1), presence-semantics predicate fails the `=0` arm
> (`left: true, right: false`). **(4) F4-11 (`a343f8c3`, `3fd231ed`).**
> The baseline is tracked, the ignore rule deleted; the pin's own mutant
> drill caught a second defect — plain `git check-ignore` defers to the
> index and NEVER reports a rule over a tracked file, so the ignore-arm
> passed green over the rule until the pin learned `--no-index`.
> Scratch-clone drill: baseline absent → `initializing without drift
> guard` + a planted committed target-side edit silently destroyed
> (exit 0); baseline tracked → the guard refuses target-side drift
> (exit 1).
>
> **Known residual, named:** the fresh-checkout sync (committed baseline
> `8e518690…` + the fork's HEAD) REFUSES naming exactly `package.json` —
> the declared fork-field typebox delta (`1.3.34` vs canonical
> `1.3.33`) trips the guard's byte-exact three-way `cmp`, which has no
> declared-delta exemption. `scripts/sync-plugin.sh` needs the declared
> fork-field delta list taught to the guard; decision carried to the
> fork lane (R84's ship).
>
> **ponytail: NOT** the live revoke/kill-switch drill (digest evidence
> only; live system untouched); NOT the sync-guard typebox exemption
> (fork lane's); NOT the wire fields (F9-02 → R90); NOT the remaining
> K-lane; no schema/route/authz change; zero new dependency edges; NOT
> AUDIT/AGENTS/badge/CHANGELOG/SHIPPED_ROUNDS by the lane (L0's).

> **Fork lane (post-R81, 2026-10-06) — ZERO-CONFLICT POSTURE, operator
> instruction: only fixes that cannot merge-conflict with
> openclaw/openclaw upstream.** Closes the ninth pass's **K9-01** (HIGH)
> and **W9-02**. Measured first: the fork is **0 behind / 96 ahead**;
> every targeted upstream file (`package-mac-app.sh`,
> `package-mac-dist.sh`, `restart-mac.sh`, `tool-results.ts`,
> `external-content.ts`) carries an **empty fork diff** — so the
> conflict question was answerable per-fix, not guessed.
>
> **(1) K9-01 — the wrapper, not the edit.** Upstream's
> `package-mac-app.sh` defaults `SPARKLE_FEED_URL`/`SPARKLE_PUBLIC_ED_KEY`
> to upstream's values (`${VAR:-default}` — measured), so a fork-built
> app verifies upstream's Ed25519 over an upstream zip through Sparkle's
> ordinary update UX and silently replaces itself.
> `scripts/fork/package-mac-app-gated.sh` is fork-NEW: it refuses to
> package a diverged tree without an explicit fork feed, refuses an
> explicitly-upstream feed, passes clean upstream checkouts through,
> then `exec`s the real script verbatim. **Upstream file untouched →
> nothing to conflict, ever.** Drilled all four arms via `--gate-only`
> (two refusals exit 1, two passes exit 0). Ceiling: direct invocation
> of upstream's script bypasses the gate — fork build paths must point
> at the wrapper (README beside it).
>
> **(2) W9-02 — the rename, extension-owned.** All eleven brain tools
> namespaced `brain_*` (plugin **0.6.12**, 106+ name references across
> names/registrations/descriptions/tests/manifest/README; the plugin's
> own CHANGELOG entries left as history). Upstream's `memory-core`
> keeps `memory_get`; ours answer unambiguously. The manifest's two
> stale "the tool path always labels" descriptions fixed to the
> two-path truth. Synced to the fork (byte-parity, the usual baseline
> reset dance); **fork lane measured: 151/151 vitest + tsc clean**.
>
> **What did NOT ship, by the operator's zero-conflict instruction:**
> **K8-01** and **K8-03** — both fixes require in-place edits of
> upstream-owned hot files (fork diffs measured empty today), i.e. real
> future conflict surface; they stay OPEN with the two honest options
> recorded (an additive seam if one exists — unproven until a code
> look — or the in-place edit shipped with a red-first behavioural pin
> so a merge that takes upstream's side fails LOUDLY). Also not:
> K9-02 (typebox five-surface drift — a fork lockfile/catalog decision),
> D9-01/02/03, K8-05/06/07/11, D8-01, and F9-02 (unrouted wire decision).

> Predecessor: **R81 "Types"** — *the plugin validates
> its own boundary, and the exclude posture means what its name says.*
> Closes the ninth pass's **S9-06** (was S8-05, re-routed) and **W9-04**;
> carries the fork re-sync to **0.6.11** (the §5 version-lag row). No
> authz change, no route change, no schema change (**1.32.26**
> unchanged), zero new dependency edges. Predecessor notes below.
>
> **(1) S9-06 — one type error used to defeat whole controls.** A string
> `agents` turned the allowlist gates into SUBSTRING matching
> (`.includes` on a string admits `ops`, `1`, anything contained); a
> string `autoRecallTopK` failed every recall comparison silently. The
> manifest's `configSchema` may or may not be enforced by the host —
> this repo cannot observe that (the S9-06 premise) — so the plugin now
> validates its own boundary: `assertFieldTypes`, a CLOSED field census
> (every declared field: boolean / string / string-array / enum /
> integer-with-range), runs FIRST in `resolveConfig`; a wrong-typed
> value REFUSES registration with the field name, the expected shape,
> and the got-typeof. **Disclosed posture change:** an unknown
> `untrustedOrigins`/`captureMode` enum used to degrade to default —
> read as fail-safe, but a typo of "exclude" silently switched the
> posture DOWN to label, re-injecting exactly what the operator wanted
> dropped. Both enums refuse now.
>
> **(2) W9-04 — `exclude` reaches the tool path.**
> `untrustedOrigins:"exclude"` now drops channel-captured hits from the
> `memory_recall` TOOL result too (a tool result is model context
> exactly like the injected fence); an all-captured result returns the
> no-memories shape with `excludedByPosture: true`. Default "label"
> byte-identical. The three comments that claimed the tool path "never
> excludes / always labels" are rewritten to the two-path truth.
>
> **(3) The fork re-sync rides this round** (`scripts/sync-plugin.sh`,
> after the plugin commit): the extension moves 0.6.10 → 0.6.11 with
> byte-parity checked (typebox specifier = the fork's declared delta),
> and the fork's vitest lane is the ONLY place the plugin's pins execute
> (S9-07's named gap — no runner exists in this repo;
> `plugin/node_modules` is empty). **Measured there: `test/` 74/74,
> `src/` 77/77, `tsc --noEmit` CLEAN.** The sync surfaced two of this
> round's own first-draft defects, both fixed before commit: the census
> initially enforced RANGES (the manifest schema's gate — fixtures
> legitimately exercise sub-second heartbeats; types refuse, ranges
> delegate), and the tool-path test fixtures lacked `untrusted: true`,
> so the Fencepost filter satisfied the exclude assertions VACUOUSLY.
> It also surfaced a PRE-EXISTING one: canonical `format.test.ts` had
> never passed a typechecker (no runner here) — JSON imports without
> NodeNext attributes + two indexed-access holes; fixed canonical-side
> and re-synced. New pins: config.test.ts (string-allowlist mutant incl.
> mixed arrays, string numerics/booleans, the enum refusal, the
> ranges-delegate arm, and the fully-typed anti-vacuity arm) +
> plugin.test.ts (exclude drops captured on the tool path;
> all-captured → no-memories; label default keeps + labels).
>
> **Spire this round:** no Rust tests added (the pins are the fork's
> vitest lane — named here so the badge count is NOT expected to move).
> Register: S9-06 + W9-04 flipped CLOSED — R81 with evidence;
> `SHIPPED_ROUNDS` → `[&str; 13]`. **What did NOT ship:** the fork lane
> proper (K9-01…03, W9-02, K8-* — remediation decisions in that
> repository, untouched by the sync); F9-02 (unrouted wire decision).
> No migration; no irreversible risk.

> Predecessor: **R80 "Gateway"** — *the remedy that
> already existed in-tree becomes the one production uses, and the edge's
> last three law-gaps close.* Closes the ninth pass's **S9-02, S9-03,
> S9-04, S9-05, S9-08**. No authz change, no route change, no wire
> change, no schema change (**1.32.26** unchanged), zero new dependency
> edges (sha2 was already a gateway dep). Predecessor notes below.
>
> **(1) S9-02 — the bounded twin is THE cache, and the inline one is
> deleted.** `worker.rs` re-exports `crate::cache::RecipientCache` (cap
> 4096, oldest-quarter eviction, TTL on the phone leg) and its unbounded
> inline HashMap is gone — with `clear()`, which was dead by ANY measure;
> `len`/`get_phone` stay as `#[cfg(test)]` measured truths (production
> writes and resolves forward; the reverse leg is eviction symmetry,
> asserted in tests, never stubbed). The module now carries a PII LAW:
> no operand rides any log lane — the `[CACHE] Mapping {phone} -> {uuid}`
> and `Self ACI: {}` INFO lines are dead, resolve paths log SHAPE at
> debug, and caller-facing errors may name the recipient the caller
> supplied. The seed verb (`POST /v1/cache/seed`) is audited at WARN
> with sha256 digests (`phone_sha256`/`uuid_sha256`, 12 hex) — a wrong
> mapping sends messages to the wrong identity, so the act is loud AND
> PII-lawful; the bearer gate already covers the route when configured
> (the audit row's "gate or audit" resolved to audit, stated here).
>
> **(2) The three small laws.** S9-03: `Config::load` refuses a
> config.yaml with group/world bits before reading it — the server's
> `secret_file` law mirrored; the refusal names `chmod 600`. S9-04:
> `BrainClient` builds with `redirect::Policy::none()` — the signed HMAC
> headers never ride a redirect cross-origin (the channel-bridge egress
> law, now twin-consistent). S9-05: the relay's inbound dedup id derives
> from the ENVELOPE'S platform timestamp (`inboundDedupId`, extracted +
> exported; absent-ts falls back to forward time; `webhook-timestamp`
> stays wall-clock — freshness is the signature, the id is identity).
>
> **(3) S9-08 — the 0644 outliers join the 0600 family.** `enforce_private_mode`
> (bootstrap) touches the main db at pool build, and the pre-migration
> `VACUUM INTO` backup + marker at their creation: idempotent (heals
> pre-law artefacts, warning the heal), warn-and-continue on failure —
> mode is defence-in-depth on multi-user hosts, not boot correctness,
> matching the surrounding backup block's own posture.
>
> **Pins, red-provable:** `the_bounded_cache_is_the_production_cache`
> (inline struct OR dead-code allow returns → fires),
> `cache_pii_operands_stay_off_the_log_lane` (mapping line returns →
> fires), `brain_client_refuses_redirects` (tests/s9_02_cache_wiring.rs);
> `a_world_readable_config_is_refused_not_read` + anti-vacuity
> `a_private_config_loads`; `resolve_reads_the_bounded_legs` /
> `resolve_fast_paths_are_shape_not_identity` (cache.rs);
> `a_retained_envelope_keeps_its_dedup_id_across_re-polls` (relay — the
> regex anchors the envelope ts, a wall-clock id cannot match);
> `private_mode_is_enforced_and_idempotent` (bootstrap).
>
> **Spire this round:** gateway bin tests **20+8+10+19 + wiring file**,
> relay **19/19** (18 + the new pin), root fmt/clippy clean, register
> flipped five rows — all with evidence. **What did NOT ship:** S9-06 +
> W9-04 (**R81**), the fork lane (K9-*, W9-02 — different repository),
> F9-02 (unrouted wire decision). No migration; no irreversible risk.

> Predecessor: **R79 "Locks"** — *the committed lock is
> the reviewed truth; nothing may move it silently — not a CI runner, not
> a git branch pointer.* Closes the ninth pass's **S9-01**. No authz
> change, no route change, no wire change, no schema change (**1.32.26**
> unchanged). The DEPENDENCY GRAPHS MOVE (that is the point — see below);
> no new dependency EDGES. Predecessor notes below.
>
> **(1) The staleness had a two-day-old root cause, and the re-lock is
> the minimal resolution.** Commit `0a1d48b9` ("the three edge tools, at
> the same versions as the server") bumped both `tools/` manifests on
> 2026-10-04 without re-locking — every bare cargo invocation since was
> a silent re-lock. Measured fix deltas, all patch-level: channel-bridge
> moves clap 4.6.6→4.6.7 ×3, jsonwebtoken 11.0.0→11.1.0, reqwest
> 0.13.4→0.13.5, tokio 1.53.1→1.53.2, uuid 1.26.0→1.27.0; signal-gateway
> the same class plus uuid 1.25.0→1.27.0. `cargo metadata --locked` exit
> 0 on both, and the cargo-audit advisory ID set is BYTE-IDENTICAL
> old-lock vs new-lock — the re-lock introduced zero new advisories
> (236/437 deps scanned).
>
> **(2) The presage decision: pin the REVIEWED rev, and the stack did
> not move.** `branch = "main"` was the defect's teeth: upstream had
> moved to `33dd149` with a newer `libsignal-service` past `bb43e81`, so
> any re-lock under the branch pointer rode the whole stack forward
> unreviewed. Both presage deps now pin
> `rev = f74b96e0…` — the rev the committed lock already resolved, the
> one the stack-policy note and the `version = "0.99.0"` label describe.
> Measured consequence: the re-lock changed the lock's presage SOURCE
> LINE (`branch=main#f74b96e` → `rev=f74b96e#f74b96e`) and nothing else
> in the stack — libsignal core and libsignal-service `bb43e81` are
> byte-unmoved, upstream's newer main is now an explicit future decision
> (change the rev + re-lock + bump the version together). The
> stack-policy comment is rewritten to the new posture; riding main
> forward is the defect, not the fix.
>
> **(3) `--locked` lands on clippy/test — fmt CANNOT carry it, and the
> register's `--no-deps` note is why.** `cargo fmt` rejects `--locked`
> ("unexpected argument", measured) because fmt resolves via
> `--no-deps` metadata — the same form that passes vacuously on a stale
> lock, which is exactly why the register row says the sweep probe must
> use the full form. Both gate lanes' clippy + test now pin resolution;
> the sweep gains `lock-freshness`, a full-form
> `cargo metadata --locked` lane over every TRACKED lockfile — tracked,
> not on-disk, because `fuzz/Cargo.lock` is a gitignored local artifact
> no checkout ever sees, and flagging it would make the lane permanently
> red over a file the repository does not ship (the audit lane stays
> on-disk by its own older reasoning). The sweep is LOCAL-only coverage;
> CI's teeth are the `--locked` flags themselves. Full validation at the
> pinned rev: signal-gateway **53 passed / 0 failed** (fmt + clippy
> clean — including a PRE-EXISTING fmt drift in
> `tests/s8_04_rate_limit_wired.rs` this round had to fix or the lane
> stays red), channel-bridge **39 passed / 0 failed**.
>
> **(4) Three pins, five red-proofs, one fail-closed catch.**
> `tests/lock_discipline_pins.rs`: the freshness law (hermetic
> manifest-vs-lock checker — direct-dep caret satisfaction + git-rev
> equality; the full-graph probe stays in the sweep lane and the CI
> `--locked` flags, because resolving signal-gateway's git graph inside
> the root suite would add multi-repo clones to every cold CI run), the
> CI-lane law (job-block sliced, so a renamed lane FAILS the existence
> arm rather than passing vacuously), and the rev law (rev, never
> branch). Red-proven: stale lock → the tokio 1.53.2 arm; dropped
> `--locked` → the lane arm; renamed job → the existence arm; manifest
> rev ≠ locked rev → the git arm; branch form → the rev law. The
> checker FAILED CLOSED on its own first run — the lock resolves
> `serde_yaml 0.9.34+deprecated` and the parser refused the
> build-metadata suffix rather than skipping past it; taught the form,
> kept the failure mode.
>
> **(5) The round's own gate run found main RED at HEAD — R77/R78's
> src comments carried audit-id labels the comment guard has rejected
> since the errata round.** Three sites: `F9-01` in
> `src/auth/policy.rs:82` and `src/handlers/mesh.rs:425` (shipped with
> the revoke refusal), `R9-02` in `src/gate.rs:687` (shipped with the
> attribute-tier pair). Neither predecessor round claims a full-suite
> run, and the ninth pass's last one predates them — a scoped-test
> discipline that leaves the suite's only red unreadable as
> "pre-existing noise" is the exact failure this file documented at the
> two-unnoted-commits rounds. Fixed at the root, the errata way: label
> dropped, invariant sentence kept verbatim; zero behaviour change.
>
> **Spire this round:** lib count +0 (the three pins are integration
> tests). **The badge moved 3160 → 3168, machine-derived**
> (`badges.sh --verify-count` exit 0, `OK README test-count badge
> matches the build (3168)`) — +8, not +3: R77/R78's own pins were
> never re-derived into the committed badge either, which is the same
> no-full-gate-run discipline §5 names. Register: S9-01 flipped
> CLOSED — R79 with evidence; `SHIPPED_ROUNDS` → `[&str; 11]`.
> **Verification, measured over the working tree:** full
> `cargo test --features bench` **49 ok lines / 0 FAILED** (pipefail);
> default-features CI dry-run clippy clean + **50 ok / 0 FAILED**;
> fmt clean (root + both tools + client); clippy `-D warnings` clean
> (bench + default + both tools); `cargo audit` exit 0 on both new
> locks with advisory ID sets **identical** old-vs-new; the new
> lock-freshness lane: **7 tracked lockfiles, 0 stale**;
> `docs-truth.sh` **LOW=17 (pre-existing, unmoved)**; doc-links 405
> resolve; `badges.sh --selfcheck`, `env-truth.sh`, `lipstyk-gate`
> exit 0. **What did NOT ship:** the OTHER CI lanes' `--locked`
> (root/crates/client/steward-harness/valet-relay stay bare — the
> register scoped the round to "both lanes", and the sweep lane covers
> every tracked lockfile behaviourally; same class, fresh today, named
> here rather than silently widened); NOT a presage/libsignal bump
> (riding main is the defect); NOT S9-02…S9-08/W9-04 (**R80**), S9-06
> (**R81**), the fork lane (K9-*), F9-02 (unrouted wire decision). No
> migration; no irreversible risk.

> Predecessor: **R78 "Attrtwo"** — *the last two
> fetch-capable survivors of the read seam, and the one outbound lane
> without the markdown-ref strip.* Closes the ninth pass's **R9-02** and
> **W9-01**. No authz change, no route change, no schema change
> (**1.32.26** unchanged), zero new dependency edges.
>
> **(1) R9-02 — `style=` and `ping=` join the attribute tier, and the
> DESIGN mirrors the two laws the tier already follows.** `ping` dies by
> NAME (the `on*` law): a click beacon is a fetch primitive — whatever
> URL it carries is sent, so there is no benign form to scheme-check.
> `style` dies by VALUE only when it can express a network FETCH (the
> scheme law's spirit): `url(`/`image-set(` after four bounded
> normalization passes — one HTML entity-decode, CSS-comment strip
> (`ur/**/l(`), one CSS-escape decode (`\75 rl(`, `\u rl(`), and the
> browser whitespace-removal rule — each one pass, no rescans (the house
> rule). `style="color:red"` and benign http(s) hrefs stay byte-identical
> (F7-01 parity law: the tier is fetch-hostile, not attribute-hostile);
> a fetch-bearing style drops WHOLE — the attribute, not the URL, is the
> hostile unit, because the seam cannot rewrite CSS safely. Ten canaries
> (obfuscations included) + neighbour-attr survival + tag/text survival;
> red-proof: disabling both arms fails the pin. The plugin needs no
> change — its attribute tier is deliberately server-canonical (the
> element mirror is its job; documented since .86).
>
> **(2) W9-01 — and the pin caught the fix's own first ordering.** The
> markdown-ref strip initially ran AFTER the newline collapse — which
> DISARMS reference-style definitions (`[x]: https://…` is line-anchored;
> collapse first and the definition never matches). The pin's
> reference-style case failed, the order flipped, and the lesson is in
> the code comment. Bare URLs in prose are untouched by this strip (the
> anti-vacuity arm) — other lanes' policies govern those. Red-proof:
> reverting the strip fails the pin at the image-ref assertion.
>
> **Spire this round:** the two new pins live in `src/gate.rs` (lib) and
> `tests/main_suite.rs` (otel-gated) — lib count +1, badge re-derived at
> ship. THREAT_MODEL's Attrbane row extended with the pair; the Origin
> row carries the OTLP-strip addendum. Register: R9-02 + W9-01 flipped
> CLOSED — R78 with evidence. **What did NOT ship:** everything routed to
> R79/R80/R81 and the fork lane, unchanged; F9-02 still unrouted (a wire
> decision). No migration; no irreversible risk.

> Predecessor: **R77 "Verity"** — *security verbs must not lie, and the
> record must agree with the tree.* The ninth-pass
> (`docs/SECURITY_AUDIT_20261006_NINTH_PASS.md`, untracked by operator
> decision — audit reports are private) R77 band, plus the private-visibility
> commit that landed the register. **No authz change**, no route change, no
> schema change (**1.32.26** unchanged), **zero new dependency edges**.
>
> **(0) Before the round: the ninth-pass register landed, and the audit
> reports went private.** 29 finding rows appended to `AUDIT.md` with the
> register law EXTENDED to them — the eighth-pass pin's slice is now bounded
> at the next `## ` heading (an unbounded tail absorbed the new table: 39
> rows parsed as 67 and the floor fired), and a ninth-pass slice with the
> same arms (floor, anchor ids, duplicate ids, disposition vocabulary,
> no-OPEN-naming-a-shipped-round) parses the new table on its own header.
> The four `SECURITY_AUDIT_*_PASS` files are untracked (`git rm --cached`
> for the three that were tracked): release tags ship to the PUBLIC repo and
> carry their commit's whole ancestry, so tracked audit reports would
> publish with the next tag. They stay on disk; nothing in the gates
> references them as links (backtick prose only).
>
> **(1) F9-01 — the revoke verb now REFUSES the identity it cannot kill.**
> The drill revoked `loopback` and got `{"known":true,"revoked":true}` while
> the same operator bearer kept 200-ing every route — the auth middleware's
> operator arm consults no revocation row, because a static token has no
> principal id to revoke. The fix is the F4-S-01 loud-refusal pattern, NOT
> the A5-01 always-write law (that law protects identities whose rows the
> middleware DOES honor): `400 operator_bearer_unrevocable`, its own code,
> naming **rotation + restart** as the remedy, writing **nothing** (pinned:
> no `revoked_principals` row lands), and naming `agent@loopback` so the
> typo-adjacent revocable identity is reachable. Anti-vacuity both
> neighbours: the loopback agent and an unseen JWT sub still revoke 200 in
> the same pin. `OPERATOR_LOOPBACK_LABEL` is a named const beside
> `AGENT_LOOPBACK_SUB`; `principal_label` consumes it (one definition).
> Openapi's 400 arm and the allow_unknown description were corrected in the
> same commit (they claimed "every well-formed revoke writes
> unconditionally" — now they name the one refusal), `schema.d.ts`
> regenerated. **Red-proof:** disabling the refusal block fails the pin at
> the `expect_err`.
>
> **(2) T9-02 — the tree's two bind docs disagreed, and the false one lost.**
> `docs/security.md` claimed the server "refuses to bind 0.0.0.0 unless
> BIND_PUBLIC=1"; the drill measured warn-and-bind (`/ready` 200 on the LAN
> interface), which `docs/configuration.md` already stated. security.md now
> states the measured truth (including which cases DO refuse: unparseable
> host, tokenless non-loopback) and names the correction.
>
> **(3) T9-03 — R76's controls get their THREAT_MODEL rows one round late,
> and the lateness is the finding.** SECURITY.md's stamp policy says it moves
> in the same commit as any security-relevant claim; R76 shipped two
> controls with SECURITY.md at 2026-09-25 and THREAT_MODEL at v1.28.92. Both
> stamps now read R77 (2026-10-06), and §5b carries BACKFILLED rows for
> R76's two controls (alert-sink ±300 s two-sided freshness + the declined
> id-dedup; the outermost rate limit and why the layering is the content),
> each row naming its own backfill so the record shows the gap.
>
> **(4) F9-S-01 — the legal-holds filter rides the house LIKE fence.**
> `?reason=%` matched every row (two full-table scans answering nothing the
> operator typed). `like_contains_pattern` paired with `ESCAPE '\\'` in the SQL now fence it.
> Red-first: reverting the fix only (tests kept) fails both pins. The pin's
> own first draft carried a wrong assertion — "substring match stays
> case-sensitive" — which SQLite refutes (LIKE is ASCII-case-insensitive,
> fence or no fence); the mutant run caught it and it was corrected, not
> kept.
>
> **(5) The docs/reg band (L9), every correction labelled with what it is
> NOT.** L9-01: the CETS 225 contradiction resolved to ONE date (2025-09-01,
> the CoE-sourced reg_watch constant) across all three sites, recorded as a
> correction, not a verification — no CoE primary was reachable. L9-04/05:
> the OWASP wording now stands on provenance (the 2026 LLM Top 10 numbering
> rests on the DOI'd artifact this repo live-fetched, explicitly NOT on a
> fresh page read; the Agentic 2025-12-09 launch date is WITHDRAWN as
> unconfirmed). L9-03/15: the state map gains a dated 2026-10-06 addendum
> carrying the FR-verified EO rows and the measured export-controls state —
> **the status date is deliberately NOT bumped** (NCSL stayed blocked; the
> quarterly pass did not run). W9-05 + F9-S-04 became THREAT_MODEL ceiling
> rows (Rule of Two; static workload identity with the per-boot ephemeral
> bearer DECLINED and its reasons recorded). T9-04: the register's own
> `:129`→`:143` citation drift fixed in the row.
>
> **What did NOT ship, stated plainly.** **Not** F9-02 (ingest verdict /
> withheld-count wire fields — a wire-contract decision). **Not** R9-02,
> W9-01 (**R78**), S9-01 (**R79**), S9-02/03/04/05/08 (**R80**), S9-06/W9-04
> (**R81**) — the register routes them and this round did not touch them.
> **Not** the fork lane (K9-01…03, W9-02, K8-* — a different repository).
> **Not** L9-16 (CT statute text still unread; carried open with evidence).
> No migration; no irreversible risk.

> Predecessor: **R76 "Cadence"**.
> Predecessor: **R75 "Greenlight"** — the round whose two commits shipped with
> no round notes at all. R76 theme: **the two messaging edges never asked *when*
> or *how often*.** Closed **S8-02** (alert-sink freshness) and **S8-04** (the
> limiter was a dead module). **No authz change**, no route change, no schema
> change (**1.32.26** unchanged), and **zero new dependency edges** — both
> `tools/*/Cargo.lock` files stay byte-identical, so R75's lockfile decision is
> not re-opened.
>
> **(1) S8-02 — the obvious fix would have rejected every legitimate envelope.**
> The Standard Webhooks spec defines `webhook-timestamp` as epoch seconds; the
> kernel's alert sink actually signs with `chrono::Utc::now().to_rfc3339()`
> (`src/alert.rs:510`). An epoch-only freshness parser `NaN`s on all real
> traffic — a green suite over a fix that refuses every genuine alert. So
> `freshTimestamp` parses **both**: all-digits → epoch, otherwise RFC3339 via
> `Date.parse`. The `±300 s` two-sided tolerance is a *mirrored* law (spec
> reference `TOLERANCE_IN_SECONDS = 5 * 60`; kernel `WEBHOOK_REPLAY_SECS` and
> `WEBHOOK_TS_FUTURE_SKEW_SECS`, enforced together in `enqueue_ts`), and
> deliberately **not** an env var.
>
> **(2) S8-02 — id-dedup is DECLINED, and the reason is in the producer.**
> `src/alert.rs:508-535` sets `ts` once and retries up to three times with the
> **same** `delivery_id`. A receiver-side id-dedup would swap a duplicate alert
> for a *silently lost* one whenever the response was lost after the forward.
> `the same id and ts is admitted twice — retries must not be eaten` pins the
> decision so the next reader cannot "helpfully" add a Set.
>
> **(3) S8-04 — the limiter's deadness was a REFACTOR artefact, not a design
> choice.** `main.rs`'s `mod ratelimit;` compiled a *private copy* the
> integration tests could not reach, and `#![allow(dead_code)]` made it compile
> silently. The module is now `pub mod ratelimit` in the lib target. **Honest
> caveat:** removing that blanket does **not** make rustc police deadness —
> once `pub` in a library target every `pub` item is externally reachable. The
> structural pin in `tests/s8_04_rate_limit_wired.rs` is what actually holds the
> line, and it is why deleting the `apply_rate_limit(app,` wrap fails a test.
>
> **(4) S8-04 — the layering is the security content.** `apply_rate_limit` wraps
> the **finished** router, after `.with_state(...)` and after the auth `match`,
> so the limit is outermost. In the tokenless loopback posture there is no auth
> layer at all: a layer placed inside `create_router_with_auth` would sit
> inside only one of its two arms and leave the unauthenticated flood unbounded
> exactly where the operator chose the loosest posture. Pinned over a real
> socket — 401s inside the budget, 429 outside it.
>
> **(5) Two executor traps, both hit and both recorded.** (a) `reqwest` **is** a
> dependency of `signal-gateway`, but `Client::new()` **panics** in 0.13 —
> it resolves `rustls-no-provider`, and installing a crypto provider needs
> `rustls` as a *direct* dep, i.e. a new dependency edge. The e2e harness is a
> hand-rolled `TcpStream` HTTP/1.1 GET instead. (b) The plan's "evict empty
> per-key vecs" is **unfirable as written**: a key that is refused while empty
> requires `max_requests == 0`, so the code would have been dead. What actually
> bounds the map is a **sweep** of keys whose newest entry has expired, run on
> the next request. Both were caught because the tests were written first.
>
> **Red-first, both edges.** All 18 JS tests fail against the unfixed relay.
> Deleting the rate-limit wrap fails 2 Rust tests; making the layer never refuse
> fails 5.

---

> **R75 "Greenlight"** — the round after R74, whose two
> commits shipped with **no round notes at all**. Theme: **the tree `main` actually
> ships must pass the gates that guard it.** Closed **S8-01**; registered **S8-02**
> and **S8-04**; re-routed **S8-05**. **No authz change**, no schema change
> (**1.32.26** unchanged).
>
> **(1) `main` was RED at R74's tip — two independent jobs plus the badge gate.**
> Not a mid-edit artefact. `openapi.yaml` carried `operationId: verifyClaim` bound
> **twice**: `:1525` on `/verify` and `:9163` on `/workflow/claims/{id}/verify`,
> and `shell/tests/registry-contract.test.ts` hard-fails Redocly's
> `operation-operationId-unique` rule. `shell/src/lib/api/schema.d.ts` was stale
> against it (the `cmp` gate, exit 1). The committed README badge read **3158**
> against a derived **3160**. Verified at **committed HEAD** via
> `git show HEAD:openapi.yaml`, not merely in the working tree.
>
> **(2) An execution prompt LIED about this defect, and the lie was checkable.**
> `docs/EXECUTION_PROMPT_R70_Seams.md:323-325` states the duplicate was "**already
> fixed** in R69's follow-up" and instructs a reader who finds it still
> duplicated that "you are on a stale tree". `git log -S'verifyClaimGate' --
> openapi.yaml` returns **nothing** — zero commits, ever. The rename lived only in
> the working tree until R75, **three releases** after the prompt claimed it
> shipped. **A prompt that asserts a fix is as much a claim as a commit subject,
> and this one was never diffed against history.**
>
> **(3) S8-01's finding was NARROWER than its own row claimed, and fixing it made
> the residual visible.** The row said the bind guard and auth guard were
> independent `if`s. The **bind guard was already present and is unchanged in
> substance** — `main.rs:105` still refuses a non-loopback bind without the
> opt-in — so the audit's own suggested remedy (re-assert loopback in the `None`
> arm) describes what that line already did. The real defect was the **auth
> posture being INDEPENDENT of the bind**: an unauthenticated router was built
> whenever no token was configured, on any interface. `resolve_api_auth`
> (`tools/signal-gateway/src/lib.rs:42`) makes the credential a function of the
> address, so `Ok(None)` is reachable **only** on loopback, and it is consulted
> **before the socket is bound**. Ten behavioural tests drive the production
> function, including `the_refusal_is_a_distinction_not_a_blunt_refusal`, which
> fails if the fix were to refuse everything — the anti-vacuity arm this repo
> keeps finding missing. **The new `signal-gateway-gate` CI job exists because that
> crate's tests ran in NO workflow**, which is exactly how "a refactor could drop
> one without failing any test" stayed true.
>
> **(4) The register gained two rows and lost a wrong route.** **S8-02**
> (`tools/valet-relay/relay.js:71-77` verifies the MAC correctly and
> constant-time, and **never checks that `ts` is recent**; the only gate is `:139`,
> so a captured envelope replays indefinitely via `sendSignal()`) and **S8-04**
> (`tools/signal-gateway/src/ratelimit.rs` is a **dead module** — `is_allowed` is
> called only from its own tests, so `POST /v2/send`, an outbound messaging
> primitive, has **no request-rate control**) are registered **OPEN — UNROUTED**:
> real, in-repo, and owned by no round. **S8-05 was routed OFF R71** because the
> defective file is **in this repo** (`plugin/src/config.ts:234-235`, a bare type
> assertion) while `untrustedOrigins` (`:247-250`) and `teamDomain` (`:269-275`) in
> the *same function* both validate at the boundary. The audit's open question is
> now **ANSWERED, and it resolves against reachability-by-anyone**:
> `brainConfigSchema` (`:16`) is referenced only at its own declaration and by the
> type alias at `:78`, and `plugin/package.json` carries **no `configSchema` key** —
> so nothing validates this config at any boundary.
>
> **(5) Two register defects repaired, both against the code rather than a commit
> subject.** The **enforcement map** at `AUDIT.md:466-483` had drifted four floors
> behind the constants guarding them: `CRATE_TEST_FLOOR` said **1 196** and is
> **2 758** (`spire_inventory.rs:177`); `ROUTER_SITES_FLOOR` said **199** and is
> **255** (`:51`); `OPENAPI_ROUTE_ROWS_FLOOR` said **161** and is **214** (`:189`);
> `AUTHZ_TABLE_ROWS_FLOOR` said **145** and is **200** (`:199`). Each now carries
> its `file:line` so the next reader can check rather than trust.
> **`SHIPPED_ROUNDS` (`tests/main_suite.rs:18064`) was TWO ROUNDS STALE** —
> `[&str; 5]` naming only R68–R73 — so a row reading `OPEN — R74` would have
> passed unchallenged, which is the precise drift the register exists to catch.
> Now `[&str; 7]`, including **R74** and **R75**.
>
> **Verification, and what is deliberately NOT claimed.** The **complete**
> verification suite has now run and is **green**, and the figures are recorded
> **with their source** because R74's lesson is that a number nobody diffed against
> a measurement is the defect itself. **No count here is hand-typed** — the README
> badge is **machine-derived** by `scripts/badges.sh --verify-count` (exit 0,
> `OK README test-count badge matches the build (3160)`), and that command, not
> this sentence, is the authority. The register pin `r73_register` is **GREEN**
> (`scrim::r73_register_rows_carry_a_disposition ... ok`, 1 passed) with both new
> rows parsed, no duplicate ids, and no row naming a shipped round while reading
> OPEN — confirmed by re-implementing the pin's own slice in a throwaway probe
> (**39** rows).
>
> `cargo test --features bench` → exit 0, **3 150 passed / 0 failed / 3 ignored**
> across 48 result lines; that and the badge's **3 160** are **not** a
> disagreement — the badge derives over the wider `bench,migrate` lane, so the two
> count different sets. `cargo fmt --all -- --check` exit 0; clippy
> `--all-targets --features bench` exit 0; `cargo test --all-targets` (default
> features) exit 0; `crates/`, `steward-harness`, `channel-bridge` (**39 passed**)
> and `signal-gateway` (**35 passed** = 5 lib + 20 pre-existing + 10 new) all
> exit 0; all **seven** feature lanes clippy-clean. Spire floors printed exactly:
> `main.rs 124≤300 · region absent · main routes 0=0 · router routes 258≥255 ·
> crate tests 2958≥2758 · coverage rows 217≥214 · authz rows 203≥200`.
> `badges.sh --selfcheck`, `env-truth.sh`, `lipstyk-gate.sh` exit 0;
> `docs-truth.sh` exit 0 at **LOW=17 (pre-existing, unmoved)** with **0 HIGH /
> 0 MED**; `check-doc-links.py` exit 0 (405 links); `cargo audit --file
> Cargo.lock` exit 0 (514 deps, 0 vulns). Shell: the `openapi-typescript` regen +
> `cmp` exit 0 with **0 bytes differ** (the gate R75 opened to fix); `pnpm test`
> **82 tests / 18 files** with `drift-gate.test.ts` and `registry-contract.test.ts`
> both PASS; `pnpm check` 0 errors; `tsc --noEmit` and `pnpm lint` clean;
> `pnpm build` ok with CSP injected and **no `'unsafe-inline'`**; `pnpm audit
> --prod --audit-level high` no known vulnerabilities.
>
> **Two lanes were NOT run, and no green above should be read as covering them:**
> **`client-gate`** — `client/` is untouched by this diff and the lane is scoped to
> client changes; **shell E2E (`pnpm test:e2e`)** — it needs a Tauri build this
> environment does not provide. Named absences, not passes.
>
> **The caveat that outranks every green: these were measured over the WORKING
> TREE, not over committed HEAD.** Per R74's own lesson, a green number measured
> over a dirty tree is not a property of HEAD — and this tree carries exactly the
> uncommitted wire and CI work this round produces. So this records **what was
> measured**; it does **not** claim `main` is green. That belongs to the commit
> and must be re-derived at the tagged SHA with `--verify-count`.
>
> **A NAMED RESIDUAL the verification pass surfaced, PRE-EXISTING and NOT fixed by
> this round: `tools/channel-bridge/Cargo.lock` and
> `tools/signal-gateway/Cargo.lock` are STALE against their own committed
> `Cargo.toml` manifests** — `channel-bridge` locks `tokio` **1.53.1** vs a
> manifest **1.53.2**, `clap` **4.6.6** vs **4.6.7**, `reqwest` **0.13.4** vs
> **0.13.5**, `uuid` **1.26.0** vs **1.27.0**, `jsonwebtoken` **11.0.0** vs
> **11.1.0**; `signal-gateway` locks `uuid` **1.25.0** vs **1.27.0** among the
> same class. **Measured consequence:** `cargo metadata --locked` **fails on both**
> (exit **101**, `cannot update the lock file … because --locked was passed`), and
> because both CI gates — `channel-bridge-gate` and the **new**
> `signal-gateway-gate` — invoke cargo **without** `--locked`, the runner
> **silently regenerates the lockfile and reports green against versions that are
> not the committed tree**. **No `Cargo.toml` and no `Cargo.lock` is in this
> round's diff**, so it is a property of HEAD, not of this round. **Not fixed
> here on purpose** — re-locking is a dependency change this round deliberately
> avoided, and the remedy is a **decision, not a patch**: re-lock and commit, or
> add `--locked` and let CI fail loudly until someone re-locks. The new job
> **inherits** the property rather than introducing it.
>
> **What did NOT ship, stated plainly.** **Not** S8-02 and **not** S8-04 —
> registered, routed nowhere, and **unfixed**. **Not** S8-07 (6 of 13 `crates/`
> members are unconsumed islands; it needs a **wire or a delete**, not a patch).
> **Not** F8-02's *enforcement*: the oracle still does not read `required_action`,
> and the decline is recorded accurately rather than reversed. **Not** F8-03's
> idempotency/receipt registry (a wire contract and a new table; the ~50 other
> `spawn_blocking` write handlers still admit the window, and a write killed
> mid-commit by a crash is still uncovered). **Not** the openclaw fork's
> K8-01…K8-15 or D8-01 (**R71**, a different repository; K8-04 needs a
> **decision**). **Not** L8-05 (the federal EOs — a single uncorroborated source,
> so writing them would be an **unsupported legal claim**), **not** L8-06's
> quarterly refresh (an external act), **not** L8-04/L8-11 (external: a deployer
> identity; BIS/ECFR), **not** L8-07's Aug 3/4 half (seven DOI-backed repo sources
> against one unsourced claim). **Not** D8-02's gate-law register — the standing
> missing artifact, still missing. **No migration is added, so this round carries
> no irreversible risk.**
>
> Predecessor: **R74 "Dirty"** — shipped as two commits (`15964613`, `50406b29`)
> with **no round notes at all**, recorded here for the first time. Theme: **a
> green suite that does not describe the committed tree is not evidence.**
> **No authz change**, no schema change (**1.32.26** unchanged).
>
> **(1) SIX SUITES FAILED AT COMMITTED HEAD, and the reason outranks the fix:
> every green figure reported for R69, R70, R72 and R73 was measured over a DIRTY
> WORKING TREE.** Two distinct root causes, not one. **Class A (5 suites):
> schema-version drift** — `src/` carries **1.32.26** (R69's
> `proposals.promoted_chunk_id` migration, `src/migration.rs:3188`) while five
> cross-round re-pins still asserted **1.32.25**. Every repair was a pure literal
> re-pin; **no assertion was softened and no test removed**. The refuse-newer probe
> moved **with** the ceiling (`src/storage_layout.rs:786-787` probes `1.32.27`
> against a `1.32.26` ceiling) so it still exercises *Greater*, not *Equal* — the
> failure mode its own message names. **Class B (1 suite, unrelated to the
> schema):** a self-flagging pin. `tests/no_engagement_name.rs` scans
> **git-TRACKED** files, so it always flagged **itself**, on the two `NAMES`
> literals it must hold to police the vocabulary — which made the control
> permanently red and **trained everyone to read it as pre-existing noise instead
> of a failure**.
>
> **(2) The second commit found the first commit's anti-vacuity check was a
> TAUTOLOGY, and its own first rewrite was too.** `NAMES.iter().all(|n|
> own.contains(n))` is `x ∈ S` with `x` drawn from `S` — `own` **IS** this file
> and `NAMES` is built from literals in it — so it holds for **every possible
> value of `NAMES`**. **Proven red-first:** replacing the whole vocabulary with a
> token occurring nowhere in the tree left both versions **fully green, policing
> nothing**. The rewrite failed identically, because the matcher finds the literals
> on the `const NAMES` **declaration** line — so the declaration satisfied the
> check meant to police the declaration. **What is worth checking is a USE, not a
> declaration.**
>
> **(3) A latent hang, fixed in passing.** `occurrences()` looped **forever** on an
> empty name, because `str::find("")` returns `Some(0)` and so `end == start`.
> Unreachable behind the hand-written literal, but a function whose contract is
> "return the occurrences" must not be able to hang.
>
> **What did NOT ship.** **Not** the wire change — `openapi.yaml`
> (`verifyClaim` → `verifyClaimGate`) and the regenerated
> `shell/src/lib/api/schema.d.ts` were **explicitly deferred** as a wire-contract
> change needing its own decision. **That deferral is why `main` shipped red, and
> it became R75's first finding.** No schema change.
>
> Predecessor: **R73 "Receipts"** — the round after the five
> `docs/audit8/08-design-grid-remediation-gates.md` §9.3 releases (R68–R70 shipped
> here, R71 is the fork, R72 shipped). Theme: **the register disagrees with the
> code.** Findings **S8-06**, **S8-09**, **L8-02**, **L8-03**, and the
> **F8-01…F8-10 register drift** from
> [`docs/audit8/`](docs/audit8/README.md). **No authz change**, no new route, **no
> wire change**, no new dependency edge, no schema change (**1.32.26** unchanged).
>
> **(1) The register was THREE RELEASES stale, and every one of the ten rows was
> wrong in the same direction.** `AUDIT.md:996-1005` filed all ten F8-* findings as
> `OPEN — R68/R69/R70` — naming the rounds that had already shipped them — while
> **all ten are closed in code**. Verified by reading the fixing code, not the
> commit subjects: F8-01 a second structural counter (`count_direct_db_calls`,
> `service/mod.rs:183`); F8-02 an independent pin (`gates.rs:222`); F8-03
> `write_deadline.rs` as the closure's first statement (`domains.rs:275-277`); F8-04
> `LogValue` (`memory.rs:294`) at nine call sites; F8-05 `strip_rust_comments`
> **used** by the counter (`spire_inventory.rs:905`); F8-06 `WEBHOOK_PATHS` with the
> prefix gone; F8-07 both rows and `ipv4_compatible` (`webhook.rs:395,516`); F8-08
> `promoted_chunk_id` + chunked IN-delete; F8-09 the roster arm; F8-10
> `resolve_bind_port` refusing port 0. **Every mismatch said OPEN for something
> closed — not one row claimed OPEN for a genuinely-open item.** Each is now
> stamped with **what the code does**, and six rows record where the audit itself
> was wrong (F8-04 found **eight** log sites not two; F8-06's fix would have
> **disabled all six webhooks** had the template/concrete-path mismatch not been
> caught; F8-07's `::169.254.169.254` was a **live admission**; F8-08 needed a
> migration the audit said was unnecessary; F8-09's "unreachable" arm was reachable
> because a **BLOB survives TEXT affinity**; F8-03 was two findings and the VACUUM
> half was already closed by R68).
>
> **(2) F8-02 is recorded as PARTIALLY CLOSED, and saying why is the point.** The
> prose was corrected and the self-asserting pin replaced, but
> `decide_gate_verdict` still does not read `required_action` (`policy.rs:198-229`)
> and both dead `DenyReason` arms remain. The audit offered two remedies and
> **neither was taken**, by deliberate decision on second-opinion-surface grounds.
> Writing a flat CLOSED would **misrepresent a declined design decision as a fix** —
> the same defect the finding was filed about.
>
> **(3) S8-06 was a real injection sink sitting directly above an `eval`, and BOTH
> of its halves were latent — the audit filed one as "live export corruption".**
> `safe_filename` refused traversal, separators and control chars but let `'`
> through, and the web arm spliced it raw into `a.download='{safe}'`. Measured:
> it returned `Some("x';alert(1)__.json")` and the emitted script carried
> `a.download='x';alert(1)//.json';`. Latent because all three call sites pass a
> literal or `i64`-derived name — **one call-site edit from live.** The quote is now
> REFUSED (not escaped): a name the browser cannot accept as a download attribute
> is not a safe one, and an anti-always-refuse pin covers the real callers. The
> body moved from `{body:?}` to `serde_json::to_string` — the helper
> `client/src/panels/mod.rs` already uses for this job, rather than a second
> hand-rolled escaper.
>
> **(4) CORRECTED MID-ROUND, and the correction is recorded because the first draft
> was wrong twice over.** The initial pin asserted U+2028/U+2029 must not appear raw
> in the JS, on the premise they are invalid string content. **They are not** —
> ES2019's JSON-superset proposal made them legal (verified in Node v24/V8 13.6:
> parses to length 3, no `SyntaxError`), and `serde_json` emits them raw. The pin
> was **red against its own fix**. The hazard that DOES remain is the **legacy
> octal escape**: `Debug` writes NUL as `\0`, so a following digit becomes `\05`,
> which JS reads as octal (`"nul\05"` → length 4 in Node). Separately, extracting
> `download_script` so the pins could drive the real builder made it **dead code
> on the host bin target** (`-D warnings` refused the build); it is now
> `cfg(any(wasm32, test))`, and `cargo check --target wasm32-unknown-unknown`
> proves the real caller still compiles.
>
> **(5) L8-03: the pin that looked like it guarded the constant CANNOT fail on a
> miscitation — which is why the correction alone would have repeated the defect.**
> `reg_watch.rs:79` cited *recital 38* (explanatory, confers no obligation) as the
> basis for the 2026-12-02 horizon; the operative provision is **Article 111(4)**.
> `ai_act_art50_marking_deliverable` asserts the DATE, the provenance surface and
> two date strings — it never read the comment, so it stayed green on a wrong legal
> instrument. **Proven:** reverting only the comment leaves that pin passing. The
> new `art50_transitional_cites_an_operative_provision_not_a_recital` reads the
> file's **own source**, slices the comment to the constant, and asserts the
> operative cite is present, the recital is not stated as *granting* the period, the
> provenance is recorded, and `docs/compliance.md` does not repeat the defect.
> **PROVENANCE LABELLED, NOT LAUDED:** no EUR-Lex fetch is reachable from a build
> and Context7 carries no AI Act coverage, so the article number is recorded
> **audit-asserted, not source-verified** — in the code, the doc, and as an
> assertion. Only the citation's *kind* was corrected; the date was independently
> confirmed and is unchanged.
>
> **(6) A gate was itself wrong, and the round found it by tripping it.** Writing
> the R73 receipts doc introduced **six new `docs-truth` MED** findings — all of them
> for *correctly prefixed* citations like `client/src/download.rs:35`. The cause was
> `scripts/docs-truth.py`'s regex: `` `?src/(...) `` — the **optional** backtick left
> no boundary before `src/`, so the pattern matched the tail of `client/src/…`,
> discarded the `client/` segment, and tested `ROOT/src/download.rs`. The
> diagnostic re-printed only the truncated path, which is why it looked like my
> citations were wrong. Fixed by requiring the backtick and capturing the whole
> path. **Anti-vacuity:** a probe doc with two genuinely non-existent paths still
> produces exactly two MED findings — the checker is more precise, not more
> permissive.
>
> **(7) S8-09 was ALREADY CLOSED and the audit read it backwards.** `release.sh:54`'s
> "push a tag manually" sits inside the **`gh`-MISSING refusal branch, followed by
> `exit 1`**, and `git blame` shows the guard (`a0eae553`) *introduced* it — it is
> the cause, not an escape. `release.sh:57-88` refuses unless the run is
> `completed` + `success`, and `release.yml:253-299` is the in-workflow backstop for
> a manual `git tag`. Reclassified rather than re-fixed, with the residual recorded:
> the manual-tag sentence is still printed, and "public push URL DISABLED" is a
> **local git-config fact not observable from the tree**, so it is recorded, never
> pinned.
>
> **(8) L8-02 closed as a CLAIM, not a duty.** `COMPLIANCE.md` said the server
> "serves the Art 50 disclosure **itself**" and a deployer could "close the
> model-origin transparency loop" by pointing at the URL. Art 50(5) requires
> disclosure **at first interaction** — a duty at the deployer's own UI seam, because
> only the deployer knows when a user's first interaction happens. Reworded to an
> **input the deployer builds the notice from**, with an explicit "what this
> component does NOT discharge". **No wire change:** `build_ai_notice` keeps its
> seven fields, because a `disclosure_timing` field would not discharge the duty
> anyway — named as a residual.
>
> **Spire at ship:** lib **2 326** passed / 0 failed / 2 ignored; `main_suite` **339**
> passed / 0 failed / 1 ignored; client **245** passed; full suite **green**;
> `crates/` green; harness green; `cargo fmt --check` clean; clippy clean on **bench
> and client**; `badges.sh --selfcheck` clean; `env-truth.sh` clean;
> `docs-truth.sh` **LOW=17 (pre-existing, unmoved), MED 6 → 0**;
> `check-doc-links.py` clean (405 links). Raw needle **2 974**, stripped **2 958**
> (gap 16, unchanged). **The floor was NOT raised: `CRATE_TEST_FLOOR` is unchanged
> at `2 758`** (headroom 200). **Zero new dependency edges: all `Cargo.lock` files
> byte-identical**; `src/authz/` **0 diff**; `openapi.yaml` and
> `shell/src/lib/api/schema.d.ts` **0 diff this round**; `src/migration.rs` **0 diff**;
> schema **1.32.26**. R70's seven pins and R72's two still green.
>
> **Red-first, and four of the pins caught defects in this round's own first draft.**
> The S8-06 pins were proven red by reverting the production change (three of four;
> the pre-existing traversal test stayed green through the revert, so the reds are
> attributable to the change and not a weakened harness). The L8-03 pin was proven red
> by reverting only its comment — **and the anti-vacuity control proved the finding**,
> because the pre-existing pin stayed green on the miscitation. The register pin was
> proven by reverting the `F8-10` row to `OPEN`, which fires the **per-id arm** rather
> than an earlier assertion. Three defects were caught this way: the U+2028
> over-strict assertion; the register pin's `.find` matching the **first of seven**
> identical table headers (so it read all seven, and its `rows.len() >= 30` floor
> passed at both 73 and 38 rows — **it could not detect the very scope bug it
> existed to catch**); and a status vocabulary with no word for `K8-04`, which is
> filed as a DECISION rather than a patch.
>
> **What did NOT ship, stated plainly.** **Not** the openclaw fork's K8-01…K8-15 or
> D8-01 (**R71**, a different repository; K8-04 needs a **decision**). **Not** L8-05
> (the federal EOs — a single uncorroborated source, and Context7 has no federal EO
> coverage, so writing them would be an **unsupported legal claim**), **not**
> L8-06's quarterly refresh (an external act), **not** L8-07's Aug 3/4 half (seven
> repo sources carry `2026-08-04` with a DOI and a prior live fetch, against one
> unsourced audit claim). **Not** L8-04 or L8-11 (external: a deployer identity; BIS/
> ECFR). **Not** S8-01, S8-05, S8-07 or D8-02 — genuinely open, genuinely out of
> this round's theme, and now **re-routed with a reason** rather than left pointing
> at a round that never owned them. **Not** F8-02's *enforcement*: the oracle still
> does not read `required_action`, and that decline is recorded accurately rather
> than reversed. **Not** a `disclosure_timing` wire field. **No migration**, so **no
> irreversible risk in this round**.
>
> Predecessor: **R72 "Truth"** — the fifth and last of the five §9.3 releases.
> Theme: **a number nobody diffed against a measurement** — the failure this file's
> own header documents as having occurred six times. Findings **R8-01, R8-02,
> **R8-03**,
> **S8-11**, **L8-01** (HIGH), **L8-06**, **L8-07**, **P8-01** from
> [`docs/audit8/`](audit8/README.md). **No authz change**, no new route, no new
> dependency edge, no schema change (**1.32.26** unchanged).
>
> **(1) THREE of the audit's eight premises were WRONG, and that is the round's
> first finding.** Every figure was re-measured at `c36f5840` rather than
> transcribed, and the tree won three:
>
> - **R8-01's fix value was stale.** The audit prescribed re-baselining the
>   hand-typed test count to **3 122** — measured at `e9c71919`, eleven commits
>   before this row was written. Applying it would have shipped a stale number
>   in place of a stale number. It also cited `AGENTS.md:1418`, which is
>   unrelated prose; the real line was **1826**. **3 122 was not written anywhere.**
> - **R8-02 was ONE dead reference, not two**, and its stated *reason* was also
>   wrong: `check-doc-links.py` **does** walk `docs/` (and `docs/AUDIT.md` is a
>   symlink to the root file). The real reason it was invisible is that the
>   checker only matches markdown-link syntax `](…)` and the reference was bare
>   backtick text in a table cell.
> - **P8-01's premise is REFUTED.** There are **four** in-repo fixture lanes, not
>   two — `client/src/main.rs:2965` consumes the canonical fixture cross-tree and
>   **runs in CI** (`client-gate`), which the audit missed. Its proposed remedy
>   would have **duplicated working cross-tree consumption**.
>
> **S8-11's replacement number is wrong too, in the same way:** the audit says
> re-baseline to eight, but there are **8 on disk / 7 tracked** (`fuzz/Cargo.lock`
> is gitignored). "Eight" is a working-tree figure a CI checkout never sees.
>
> **(2) R8-03 was a LIVE green gate that could not fail.** `--selfcheck` claimed
> to be a drift guard but grepped only for the string `"not selfcheck-verified"`;
> the derivation sat **below** the selfcheck path's own `exit 0`, so the
> comparison was physically unreachable. **RED-FIRST, recorded:** the badge read
> 3 120 against a derived 3 156 and selfcheck exited **0** — a green gate on a
> lie — and a planted `999999` also passed, while a control planted *version*
> drift correctly failed, proving the exit path was live and the missing arm was
> the only defect. Fixed by splitting the modes by cost: `--selfcheck` stays cheap
> and now declares what it does NOT check, while **`--verify-count`** re-derives
> and refuses on drift, wired into `ci.yml`'s `lint-test` job.
>
> **(3) A second defect surfaced while fixing the first, and the house gate caught
> two more of my own.** The disclaimer arm was a **whole-file** grep satisfied by
> a sentence 28 lines below the badge, so the badge could be arbitrarily wrong
> while green; it is now scoped to the badge's own block, proven non-vacuous (the
> same bytes at a distance still pass the old grep and now fail). Separately: a
> pre-existing gate at `src/docs_truth.rs:115` requires the literal
> `not selfcheck-verified` in README — **not visible from the prompt** — and
> rewriting the sentence without it turned the lib suite red; the disclosure was
> restored and only the surrounding comment updated. Finally, `--verify-count`
> caught its own first re-baseline: the badge was set to 3 157 from a run where
> the `docs_truth` pin was still failing (and therefore counted as `failed`, not
> `passed`); fixing it added exactly one test and the derive said 3 158.
>
> **(4) S8-11's real defect was not the sentence.** `scripts/verification-sweep.sh`
> ran bare `cargo audit`, which covers the **ROOT LOCKFILE ONLY** — so the local
> gate was the **weaker** of the two, on exactly the surface this finding is
> about: RUSTSEC-2026-0285 (rustls, 0.23.43 → 0.23.45) landed in the `tools/*`
> trees, which the root lockfile never saw. `ci.yml` already looped over every
> lockfile; the sweep now mirrors it. Non-vacuity proven: **8** distinct scans,
> each with its own dependency count, all clean.
>
> **(5) L8-01 (HIGH) is a bookkeeping correction, and its scope is stated IN THE
> FILE.** The CT CART general duties (Oct 1 2026) had passed and were still filed
> under "Scheduled" — the repo asserted this duty, set its own clock, and never
> re-armed it. Moved to the live clause; the checklist re-tenced. But the date
> arithmetic is provable from the repo while the **statute text remains
> UNVERIFIED** (`cga.ct.gov` unreachable), so no legal conclusion was added, and
> **no `reg_watch.rs` constant**: the deliverable is deployer-side notice copy the
> server cannot observe, and a pin asserting an artifact it cannot see is theatre.
>
> **(6) Three items were DEFERRED with a reason, and none is closed.** **L8-05**
> (two federal EOs) is unverifiable from this environment — the EOs appear only
> in the register that cites them, and Context7 carries no federal EO coverage;
> writing them would be an **unsupported legal claim about a live instrument**.
> **L8-06**'s quarterly refresh is an **external act**; the map now says the pass
> has not run and why, and the status date is **deliberately NOT re-stamped**
> because bumping it would claim a verification that never happened.
> **L8-07**'s Aug 3/4 half is **not** changed: seven repo sources carry
> `2026-08-04` backed by a DOI and a prior live fetch, against one unsourced
> audit claim — a DOI-backed claim is not swapped for an unsourced one.
>
> **Spire at ship:** lib **2 325** passed / 0 failed / 2 ignored (baseline 2 325,
> **+0** — the round's two pins live in `tests/main_suite.rs`, not the lib);
> `main_suite` **338** passed / 0 failed / 1 ignored; full suite **green**;
> `crates/` green; harness green; `cargo fmt`
> clean; clippy clean on **bench**; `badges.sh --selfcheck` clean;
> `env-truth.sh` clean; `docs-truth.sh` **LOW=17 (pre-existing, unmoved)**;
> `check-doc-links.py` clean (404 links); `cargo audit` clean across **8**
> lockfiles; shell gate **82 passed / 18 files**, `tsc` clean. **The floor was
> NOT raised: `CRATE_TEST_FLOOR` is unchanged at `2 758`.** **Zero new dependency
> edges: all `Cargo.lock` files byte-identical**; `src/authz/` **0 diff**;
> `src/migration.rs` **0 diff**; schema **1.32.26**. `docs-truth.sh` LOW is
> **unchanged at 17**. R70's seven pins all still green.
>
> **What did NOT ship, stated plainly.** **Not** L8-05, **not** L8-06's refresh,
> **not** L8-07's Aug half, **not** any K8-* (R71, a different repository), **not**
> a CI job for the plugin's fixture lane (its `package.json` has no `scripts`
> block and depends on `workspace:*`, which cannot resolve outside the openclaw
> workspace — that lane is R71's). **L8-02 is explicitly re-scoped OUT of R72**
> and **S8-09 remains open** — neither was in this round's scope and the register
> says so rather than leaving them looking addressed.
>
> Predecessor: **R70 "Seams"** — the third of the five §9.3 releases. Theme:
> **the cheap enforcement wins — six seams where the machine was right for the
> wrong reason, or right by luck.** Findings **F8-03** (HIGH), **F8-04**,
> **F8-06** (MEDIUM), **F8-07**, **F8-09**, **F8-10** (LOW) from
> [`docs/audit8/`](docs/audit8/README.md). **No authz change**:
> `git diff src/authz/` is **empty**, no new route, no wire field, no new
> dependency edge, no schema change (**1.32.26** unchanged).
>
> **(1) Every §1 premise was re-measured, and TWO of the round's own claims were
> wrong — which is the round's first finding.** All six §1 figures matched (raw
> needle **2 957**, stripped **2 941**, lib **2 319**, 52 handler files, schema
> **1.32.26**), and F8-03's `VACUUM` half is confirmed **already closed by R68**
> (`domains.rs:266` is `if let Err(e) = …`, not `let _ =`) — so it was **not
> re-fixed**, exactly as the note warned. But (a) the prompt's suggested reuse of
> `spire_inventory::strip_rust_comments` is **IMPOSSIBLE and was not attempted**:
> it is `pub fn`, but `spire_inventory` is `#[cfg(test)] pub mod`
> (`src/lib.rs:346`), so it does not exist in the lib an integration test links
> against — the `cfg` is the blocker, not visibility. The F8-04 pin therefore
> lives in `tests/main_suite.rs` and reuses the two existing **test-side** house
> lexers (`strip_line_comments` / `strip_cfg_test_regions`); **no second `src/`
> stripper was written and `dup_guard` is untouched**. And (b) **F8-09's
> reachability claim was wrong in the direction that matters**: the note predicted
> the row-mapping arm unreachable because `TEXT` affinity coerces every storage
> class, and that is true for `INTEGER` and `REAL` but **FALSE for `BLOB`**
> (verified against SQLite directly). So the honest **behavioural** pin was
> available after all. Had the premise been carried — or the note's suggested
> `42`/`1.5` fixtures used, which coerce to `'42'` — the pin would have been
> **green before the fix** while proving the arm that was not changed. It now
> asserts `typeof(roster_json) == 'blob'` as a **precondition**.
>
> **(2) F8-04: the log seam is unskippable, and the guard found THREE offenders
> the audit did not name.** `sanitize_log_value` had **one** production call site
> and fourteen tests, **none** asserting any call site USES it — a seam nothing
> forces through is a convention. The guard found **eight** request/config-derived
> sites before any was fixed, across five files: `recall.rs` `{domain}`,
> `domains.rs` `{name}`, `webhooks.rs` ×2, `mod.rs` `error = %message`,
> `observe.rs` `{url}`, `ump_ops.rs` `owner`/`declared`. The fix is a `LogValue`
> newtype beside the seam whose **only** constructor is `sanitize_log_value`: no
> `From<&str>`, no `From<String>`, no `Deref`, no `Default`, private field — each
> pinned, because any one re-opens the hole. The scan reads **both** value-carrying
> syntaxes (`{ident}` placeholders **and** `%ident`/`?ident` structured fields),
> because the `webhooks.rs` offender is the field form and a placeholder-only scan
> would have passed it; multi-line invocations are scanned whole. The remaining 31
> sites are exempt **by category**, justified in code, with the integer-id set a
> **closed list rather than a shape** — a shape rule would have exempted exactly
> the request-derived names (`name`, `domain`, `path`, `url`, `owner`).
>
> **(3) F8-06: the prefix rule is gone, and the pin caught a fail-open
> regression this round nearly shipped.** `path.starts_with("/webhooks/")`
> exempted whatever landed under `/webhooks/`, including any future route — not a
> live hole (all six verify and fail closed) and precisely an **unenforced
> convention**. Replaced with `WEBHOOK_PATHS` naming all six. **The three
> `is_public_path` call sites DISAGREE:** `auth.rs:129` passes axum's
> `MatchedPath` (the TEMPLATE, `/webhooks/{kind}`) while `:277`/`:549` pass
> `req.uri().path()` (the CONCRETE path, `/webhooks/github`). A `contains()` on
> the template list would have exempted the template and **refused every real
> request — silently disabling all six webhooks.** `is_webhook_path` matches
> segment-wise instead. The pin then caught **two fail-open bugs in the first
> draft**: `split('/')` on `{kind}` never equals the literal `"{kind}"`, and a
> stale list entry would keep exempting a path nothing serves — so both
> directions are checked against the **router** (the authority), never the list
> against itself.
>
> **(4) F8-03 (the surviving half): the deadline moves INSIDE the closure.**
> `TimeoutLayer` drops the handler future at 30 s, but a `spawn_blocking`
> closure is **not cancellable** — it runs to completion and **COMMITS**, so the
> client sees a 408 while the row lands anyway and a retry double-commits. A
> check *outside* the closure is **decorative**: the work is already queued and
> nothing can call it back. `src/service/write_deadline.rs` reads the clock **at
> the moment work starts** and refuses before any statement runs, on
> `DELETE /domains/{name}` — the gate is the closure's **first** statement,
> before `pool.get()`, so a refusal provably took no connection and opened no
> transaction. The 30 s is now `config::REQUEST_TIMEOUT_SECS` with
> `WRITE_DEADLINE_MARGIN_SECS` held back, so handler and middleware cannot drift.
>
> **(5) F8-10 and F8-07, both measured rather than assumed.** `BIND_PORT` was
> `.parse().unwrap_or(8765)`: a typo bound **the production port** with no
> diagnostic. It now reuses the `WRITE_POSTURE` shape (absent = default; only
> present-and-invalid refuses; empty = unset), so **no deployment changes
> behaviour**. A throwaway probe (since deleted) established what `u16::from_str`
> actually does: `abc`/`65536`/`-1`/`""` all fail to parse, and **`0` parses
> successfully** — so a parse-only fix would **not** have closed the finding,
> since port 0 binds a kernel-chosen ephemeral port that changes every restart.
> `876` is deliberately **not** a refusal (a valid `u16`; refusing every
> "surprising" number would invent policy the audit did not ask for). F8-07's
> harder half: `to_ipv4_mapped()` unwraps **only** `::ffff:0:0/96` (confirmed
> against the std source — bytes 10..12 == `0xff,0xff`), **not** the
> IPv4-**compatible** `::/96`, so `::169.254.169.254` was **ADMITTED** while the
> v4 table sat fully present and never consulted. **Normalised, not "add a
> row"** — a row refuses the `::/96` block; normalisation subjects the embedded
> v4 to the **whole** v4 table and names the real reason. `::` and `::1` are
> deliberately not embeddings.
>
> **Spire at ship:** lib **2 325** passed / 0 failed / 2 ignored (baseline 2 319,
> **+6**); full suite **green**; `crates/` green; harness green; `cargo fmt`
> clean; clippy clean on **bench, default, otel, `crates/`** and **all six**
> feature lanes; `lipstyk-gate` **0 findings**; `badges.sh --selfcheck` clean;
> `env-truth.sh` clean; `docs-truth.sh` **LOW=17 (pre-existing, unmoved)**;
> `check-doc-links.py` clean (404 links); **`cargo audit` clean** (514 deps);
> shell gate **82 passed / 18 files** incl. the drift gate, `tsc` clean. `main.rs`
> 124≤300, router routes 258≥255, coverage 217≥214, authz rows 203≥200. **The
> floor was NOT raised: `CRATE_TEST_FLOOR` is unchanged at `2 758`** (measured
> **2 954** stripped — headroom 183 → **196**; raw **2 970**). Raw and stripped
> moved by the **same +13**, so this round added **no** fixture-string inflation
> (the raw−stripped gap is unchanged at 16 and belongs to the baseline). **Zero
> new dependency edges: all `Cargo.lock` files byte-identical**; `src/authz/`
> **0 diff**; `openapi.yaml` and `shell/src/lib/api/schema.d.ts` **0 diff** this
> round so no regeneration was owed; **no new `OPENAPI_ROUTES`/`PUBLIC_PATHS`
> row**. R69's `no_sql_in_handlers_enforced` green.
>
> **Red-first: all eight pins were proven ABLE TO FAIL**, and **two caught real
> defects in this round's own first draft** — which is what writing them was for.
> The load-bearing one is **#8**: keeping the F8-03 gate but moving it one line
> down, after `pool.get()`, is exactly the "machine checks under-delivered" shape
> and **a presence-only guard would have passed it**. #4/#5 caught the `::/96`
> normalisation and the two v4 rows (reverting each admits the address). #7 caught
> `.flatten()` returning `Ok(SweepReport { crew_rows: 0, .. })` where a refusal was
> required. One house gate fired on this round's own code
> (`comments_never_reference_versions_plans_audit_ids` rejected the finding labels
> in fifteen source comments) and was fixed at the root — labels dropped,
> reasoning kept.
>
> **What did NOT ship, stated plainly.** **Not** the write **idempotency/
> receipt registry** or the per-route openapi ceiling annotations — a wire
> contract and a new table, the next decision. **Named residual:** a write that
> *starts* within budget and is then killed **mid-commit** (crash, not timeout)
> is still uncovered. **Not** F8-03's `VACUUM` half (**R68 closed it**). **Not**
> every DB-touching handler — the deadline lands on the named route plus the
> shared helper, and the ~50 unreached `spawn_blocking` write handlers in
> `src/handlers/**` are **named as a residual, not silently skipped**. **Not**
> F8-01 (R69). **Not** K8-* (**R71**, a different repository; K8-04 needs a
> *decision*). Not R8-01/02/03, S8-11, L8-01/05/06/07, P8-01, K8-15 (**R72**).
> **No runtime authorization change.** **No migration is added, so this round has
> no irreversible risk.**
>
> Predecessor: **R69 "Erasure"** — the second of the five §9.3 releases. Theme:
> **a compliance certificate can certify an erasure that did not happen** — the
> DSAR erasure deleted the memories it could attribute to a subject and left the
> approved proposals behind them, because its only reach into `proposals` was a
> literal `content LIKE '%subject%'` and a candidate body almost never contains
> its owner's identity. Finding **F8-08** (HIGH, drill-proven) from
> [`docs/audit8/`](docs/audit8/README.md). **No authz change**:
> `git diff src/authz/` is **empty**, no new route, no wire field, no new
> dependency edge.
>
> **(1) The §9.3 plan's prescribed fix was IMPOSSIBLE as written, and the tree
> won.** The plan said *"carry the approved chunk ids the erasure just deleted
> and delete their proposals by `id IN (…)` — the proposal that produced a
> memory is reachable from the memory."* Reproduced by hand at `1c00c83a`,
> **there is no id to carry**: `knowledge` carries no proposal ref (base
> `CREATE TABLE` + every `ALTER TABLE knowledge ADD COLUMN`); neither
> `promote_chunk_insert` (`service/gate.rs:186-208`) nor `kcs_draft_insert`
> (`:87-116`) binds one; `cas_proposal_approved` (`service/review.rs`) records
> none; no linking table exists; and the two tables share no hash column
> (`proposals` has **no** `content_hash`). Option (a), the audit chain, was
> **measured** closed first: `audit_events` has no later `ALTER`, and both
> `target_hash` and `detail_hash` are SHA-256 digests — **a hash is not
> reversible**, so the raw proposal id cannot be recovered from the chain.
>
> **(2) So the fix is a schema migration, and it contradicts the audit's "no
> migration is needed" note — which was right about that reasoning and wrong
> about the schema.** `proposals.promoted_chunk_id INTEGER`, schema **1.32.25 →
> 1.32.26**: additive, NULLable, `pragma_table_info`-guarded. **The proposal is
> the row extended; `knowledge` gains nothing**, so the FK-children maps in
> `service/dsar.rs:25-44` and `service/purge.rs:10`/`:29` stay accurate — and the
> migration comment says so explicitly, because a reader will look for a map
> entry that legitimately does not exist.
>
> **(3) The edge is a SEPARATE write beside the CAS, not a CAS parameter.**
> `cas_proposal_approved` has **seven** call sites and four promote nothing, so a
> NULL `promoted_chunk_id` is the correct recorded state there. `record_promoted_chunk`
> rides the caller's approve tx. Wired into the two arms that create a memory:
> the generic promote, and the KCS draft — which deliberately records **no** edge
> for `KIND_LINK_ONLY`, because that arm reuses an existing article and a
> fabricated edge is exactly the kind of lie this round exists to remove.
>
> **(4) The erasure arm erases those proposals by id, in the caller's tx, after
> the knowledge purge** so it walks the chunks genuinely deleted. The
> `content LIKE` arm is **KEPT**: removing it would *reduce* erasure coverage for
> subjects whose text genuinely appears in a proposal. The IN-list is chunked at
> **900**, under a **measured** ceiling — this crate's bundled SQLite prepares
> 32,766 bound params and refuses 32,767 ("too many SQL variables"), measured
> with a throwaway probe that was then deleted. The count goes to `tracing`,
> **not** the certificate JSON: §2 keeps the round off the wire, and a
> certificate field nothing consumes is a field no one verifies.
>
> **(5) Red-first, and red TWICE.** The §3 pin failed before the fix
> (`left: 1, right: 0`, the proposal still present, with both anti-vacuity
> assertions above it passing so the fixture was known-real), and the red-proof
> was **re-run on the finished fixture** by disabling the arm — which failed
> identically. Six further tests from §4.4, and **four planted mutants were all
> killed** (wrong column, chunking removed, swallowed delete error, arm
> disabled). **Two tests could not fail** under the first mutant run and were
> **rewritten rather than kept**: their fixtures did not collide proposal ids
> with purged chunk ids, so an arm keyed on the wrong column passed them. The
> chunking test needed the same treatment — at 925 ids it passed with the
> chunking removed, so its fixture now spans the measured 32,766 ceiling.
>
> **Spire at ship:** lib **2 319** passed / 0 failed / 2 ignored (baseline 2 310,
> **+7**); `main_suite` **329**; `crates/` **308**; harness **44**; default
> all-targets **3 013**; clippy clean on bench, default, otel, `crates/` and
> **all six** feature lanes; `cargo fmt --check` clean; lipstyk **0 findings**;
> `badges.sh --selfcheck` clean; `env-truth.sh` clean; `docs-truth.sh` LOW=**17**
> (pre-existing); `check-doc-links.py` clean (404 links); **`cargo audit` clean**
> (514 deps); `main.rs` 124≤300, router routes 258≥255, coverage 217≥214, authz
> rows 203≥200. **The floor was NOT raised: `CRATE_TEST_FLOOR` is unchanged at
> `2 758`** (measured **2 941** stripped, headroom 137 → **183**; raw needle
> 2 957, and the raw−stripped delta is this round's fixture strings, disclosed
> here rather than absorbed by re-baselining). **Zero new dependency edges: all
> `Cargo.lock` files byte-identical**; `openapi.yaml`, `route_guards.rs` and
> `src/authz/` all **0 diff**. R68's `no_sql_in_handlers_enforced` still green.
>
> **Known pre-existing, NOT fixed here.** `tests/no_engagement_name.rs` fails —
> **re-verified at the baseline this round** by stashing the entire diff and
> re-running it there, where it fails identically; it is the only red in the
> suite and it is not R69's. One **intermittent** flake appeared during
> validation and is **not** R69's: `handlers::webhooks::inbound_signal_becomes_
> screened_steering` sets a process-global env var without taking an env lock
> (the class `config.rs` guards with `TOKEN_ENV_LOCK`/`IP_ENV_LOCK`), raced once
> across three runs, and passed on three subsequent full-suite runs; R69's diff
> does not touch that file. `shell/src/lib/api/schema.d.ts` is stale against
> `openapi.yaml` (§6.1 of the prompt) — which is precisely why R69 adds no wire
> field. `AGENTS.md`'s own header figures disagree with measurement (that is
> `R8-01`, assigned to **R72**, and one theme per release means it is **not**
> re-baselined here).
>
> **§7 note — both "known pre-existing" items above were closed AFTER R69's own
> scope, by a separate follow-up. Stated here so the header does not outlive its
> debunking.** (1) `tests/no_engagement_name.rs` was **not** a content leak: the
> pin scans git-TRACKED files and flagged **itself**, on the two `NAMES`
> literals it must hold to police the vocabulary. That is a structural
> false-positive, and it made the suite's only red permanently unreadable as
> "pre-existing noise". Fixed by exempting the pin's own file — **named** in
> `ALLOWED_FILES`, never a blanket skip — plus an anti-vacuity assertion that
> fails if the vocabulary ever leaves the file, so the exemption cannot rot
> into a silent pass. **Proven non-vacuous:** planting the name in
> `src/storage_layout.rs` makes the pin fail. (2) The webhooks flake is closed
> by a `tokio::sync::Mutex` fence around `BRAIN_SIGNAL_WEBHOOK_SECRET_FILE`,
> with **two** deterministic red-proof tests — the natural race fired ~1 run in
> several, which is not evidence a fence works. Measured: **20/20 green** with
> the fence, **20/20 red** with it bypassed, and the mutant still passes the
> original signal test. `tokio::sync::Mutex` rather than `std::sync::Mutex`
> because the guard is held across `.await` (clippy's `await_holding_lock`
> correctly refuses the `std` form), and **non-reentrant/FIFO** — an early
> version acquired it twice in one test and deadlocked.
>
> **§0 note — the prompt's baseline was stale and was re-verified rather than
> carried.** The prompt pins schema **1.32.24**; measured at `1c00c83a` it is
> **1.32.25**, and the prompt's `is_newer_than_known(Some("1.32.26"))` probe was
> *already in the tree* — i.e. the prompt was written against the 1.32.24
> ceiling and the tree had moved twice since. Every other §1 figure was
> re-measured and matched: floors `2 758` / 255 / 214 / 200, 52 handler files,
> raw needle **2 954**, stripped needle **2 934**, headroom **137**.
>
> **The largest residual, named.** Historical approved proposals keep a NULL
> edge and are **not** retro-linked: a proposal approved before this release
> promoted a memory that may be purged tomorrow, and the correspondence was
> never stored, so the erasure reaches it only if the subject's string happens to
> appear in its body. Backfilling by inferring the edge from `content` would be
> the substring match this round exists to stop trusting.
>
> **What did NOT ship, stated plainly.** **Not** the §9.3 fix as specified (it is
> impossible). **Not** a certificate wire field (blocked by the shell drift).
> **Not** an owner column on `proposals` — declined by the audit, and the edge
> belongs on the proposal→chunk correspondence, not on a subject attribution
> proposals do not carry. **Not** F8-03/04/06/07/09/10 (**R70**), K8-* (**R71**,
> the openclaw fork), R8-01/02/03, S8-11, L8-*, P8-01, K8-15 (**R72**).
> **No runtime authorization change.**
>
> Predecessor: **R68 "Silence"** — the first of the five §9.3 releases. Theme:
> **the machine checks under-delivered** — three guards/pins passed while their
> subject was violated, or asserted a property they could not fail. Findings
> **F8-01** (HIGH), **F8-02** (HIGH), **F8-05** (MEDIUM) from
> [`docs/audit8/`](docs/audit8/README.md). **No runtime authorization behaviour
> changes**: `git diff src/authz/policy.rs` is **empty**, and the authz half is
> prose plus pins.
>
> **(1) The Architecture Law's SQL guard was blind, and ten production
> violations were live under it.** `count_sql_statements` recognised exactly
> four keyword openers (`select` / `insert` / `update` / `delete … from`) and
> was structurally blind to `PRAGMA`, `VACUUM`, `BEGIN`/`COMMIT`/`ROLLBACK`,
> `REPLACE INTO`, and the whole rusqlite method surface. The guard reported
> `ok` the entire time. **RED-FIRST, recorded:** extending the guard *before*
> migrating anything made it fail naming exactly **10** violations across four
> files — `domains.rs` ×2, `govern.rs` ×4, `shifts.rs` ×3, `ump_ops.rs` ×1 —
> matching the audit's census (which named 2; a full scan found 10). The audit's
> suggested fix — deny any direct rusqlite surface — is **refused as stated**:
> a keyword extension over comment-stripped handler source yields **25**
> production hits of which only **10** are real and **15** are ordinary Rust
> method calls (`h.update(`, `policy.insert(`, `headers_mut().insert(`). A
> guard that false-fires 15 times per run gets deleted. **So the counter is
> STRUCTURAL — it matches CALL SHAPES including the `(`.** That is what makes
> it both sensitive and silent-on-clean-Rust, and it needs **no comment
> stripper** to be correct (comment-stripping changes the DIRECT hit count in
> `src/handlers` by exactly **0**: raw 19 == stripped 19), so the deliberate
> "comment residue counts too" self-pin on the keyword counter is untouched and
> `count_sql_statements` is **byte-identical**. All ten sites now live in cores:
> `domains_admin::file_domain_counts_at`, the post-delete `VACUUM`,
> `ump_ops::record_forbidden_scope_at_db`, and the new
> `service/snapshot_probe`. The `#[cfg(test)]` exemption is **structural-counter
> only**, and test regions remain held to the keyword counter.
>
> **(2) The highest-value fix is one the guard found by accident: `shifts`'**
> **hand-rolled transaction.** `BEGIN IMMEDIATE` / `COMMIT` / `ROLLBACK` became
> the RAII `WorkflowTx`, closing **three** defects in one move — the discarded
> ROLLBACK error (`let _ =`, forbidden by the fail-closed law), a **panic
> unwinding past the rollback and returning an OPEN TRANSACTION to the pool**,
> and the lost `note_busy_error` contention telemetry the raw `execute_batch`
> bypassed. The `VACUUM` migration closed a fourth instance of the same law:
> `let _ = conn.execute_batch("VACUUM;")` is now a `tracing::warn!`.
>
> **(3) A security improvement, disclosed.** The snapshot probe's own doc
> comment called it "Read-only — it never creates or mutates a snapshot", but
> `Connection::open` does **not** set `SQLITE_OPEN_READ_ONLY`, so it opened
> every `.bak` read-**WRITE**. The new core opens `SQLITE_OPEN_READ_ONLY |
> SQLITE_OPEN_URI`, so a probe can no longer alter the evidence it reports on.
> The round's §9 named this as the only irreversible risk ("if `integrity_check`
> refuses a read-only handle, revert that one line") — **measured, it does
> not**: the new pins prove a real fixture passes `integrity_check` on the
> read-only handle, so no revert was needed.
>
> **(4) `CRATE_TEST_FLOOR` is no longer gameable, and was NOT re-baselined.**
> Ten `#[test]` written inside a doc comment satisfied the floor. The needle now
> strips comments first, via a **string-aware** stripper (`strip_rust_comments`):
> `//`, **nested** `/* */`, `"…"` with escapes, raw strings at ANY `#` count
> (`r"…"`, `r#"…"#`, `br##"…"##`) with or without a `b`/`c` prefix, and `'"'` as
> the char literal it is. **Every defect in the naive form pushed the count
> DOWNWARD** — a broken stripper looks *safe* — so the pin asserts BOTH
> directions (prose removed AND real code surviving), and the anti-vacuity case
> is a fixture that must count **1**. **RED-PROOF, recorded:** a planted
> 10-attribute doc comment moved the raw needle by **+11** and the stripped
> needle by **+0**; reverted. **The floor stays `2 758`** — headroom was 137,
> so raising it would have spent the guard's budget on a measurement.
>
> **(5) The second-order hazard was worse than the round claimed.** The prompt
> said `tests/rbac_evaluation_pins.rs:802-810` re-implemented the *unstripped*
> needle and reported 2 908 against the spire pin's 2 895 — a 24-unit
> disagreement. True, and it is worse: that file **contains the literal
> `"#[test]"` it counts**, so the counter inflated the number it validated.
> The needle is now built from parts. **The prompt's preferred fix (option a,
> "make the spire helper `pub`") is IMPOSSIBLE and was not attempted.**
> `spire_inventory` is `#[cfg(test)] pub mod`, so it does not exist in the lib an
> integration test links against — visibility is not the blocker, the `cfg` is.
> A second `src/` definition would fire `dup_guard`. So the copy lives in the
> test crate and **the two are pinned to agree** (`r68_both_counters_measure_the_
> same_tree`), which is what makes the duplication defensible.
>
> **(6) F8-02: the authz prose is now true, and the pin is no longer
> self-asserting.** The middleware claimed three enforced properties; **two were
> unreachable in production** — the agent-class arm (deliberately removed; two
> matrix rows measured the agent posture and broke, see `policy.rs:205-220`) and
> the deny-only capability arm (dead because the sole production constructor
> hardcodes `required_capability: ""`, `gates.rs:167`). The
> **opposite-direction** overclaim is corrected too: the authz matrix pins
> each row to the handler's `authorize()` literal, which substantiates
> **handler-side** agreement and does **not** make the *oracle* enforce the
> action. `r47_gate_rows_read_their_declared_action` used `gate_for` as its own
> oracle — delete every enforcement use of `required_action` and it stayed
> green. It now reads its expectation from the **`AUTHZ_GATES` table literal**,
> and a new behavioural pin drives the production-shaped `Gate` through
> `decide_gate_verdict` with a real `Principal::agent_loopback()` and asserts
> the verdict is **not** a `Deny` — green today, **RED the moment someone adds
> the agent arm without updating the doc**, which is the drift F8-02 names.
>
> **Spire at ship:** lib **2 310** passed / 0 failed / 2 ignored;
> `main_suite` **329**; `crates/` green; harness green; `badges.sh --selfcheck`
> clean; `env-truth.sh` / `docs-truth.sh` (LOW=17, pre-existing) /
> `check-doc-links.py` clean; **`cargo audit` clean** (514 deps); clippy clean
> on bench, default, otel, crates and **all six feature lanes**; `lipstyk-gate`
> **0 findings**; `main.rs` 124≤300, router routes 258≥255, coverage 217≥214,
> authz rows 203≥203. **The floor was NOT raised: `CRATE_TEST_FLOOR` is
> unchanged at `2 758`** (measured **2 934** stripped — headroom 137 → **176**,
> of which +39 is this round's own fixture strings, disclosed as a ceiling in
> the CHANGELOG rather than absorbed by re-baselining; **10** real `#[test]`
> attributes added). **Zero new dependency edges: all tracked `Cargo.lock` files
> byte-identical**; `src/migration.rs`, `openapi.yaml`, `route_guards.rs` and
> `src/authz/policy.rs` all **0 diff**.
>
> **Known pre-existing, NOT fixed here (§6, each measured).**
> `tests/no_engagement_name.rs` fails at the baseline commit — **proven** by
> running it in a pristine worktree of `d11326c5`, where it fails identically;
> this is the only red in the suite and it is not R68's. `shell/src/lib/api/
> schema.d.ts` is stale against `openapi.yaml` (drift gate already red).
> `AGENTS.md`'s own header figures disagree with measurement (claims needle
> 3 069, measured 2 908) — that is `R8-01`, assigned to **R72**, and one theme
> per release means it is **not** re-baselined here.
>
> **§0 note — the prompt's baseline was stale and was re-verified rather than
> carried.** The prompt pins `7dc6cb47` with `M AUDIT.md` + `?? docs/audit8/`;
> actual HEAD was **`d11326c5`**, which had committed the audit8 report as its
> own docs-only commit (what §6.2 anticipated). Every §1 figure was
> **re-measured** and all matched: floors `2 758` / 255 / 214 / 200, 52 handler
> files, 8 router files, raw needle **2 908**, DIRECT hits **19** (10
> production), stripped needle **2 895**, headroom **137**.
>
> **What did NOT ship, stated plainly.** Not the `/ops/authz/explain`
> disclosure field (blocked by the shell drift above; named the next round's
> first wire item). Not end-to-end pins for the two unreachable deny reasons —
> they would assert a fiction; they are machine-pinned as ceilings instead. Not
> `ROUTER_SITES_FLOOR` hardening (measured 258 raw == 258 stripped; gameable in
> principle, no honest lever this round). Not the 16 `cfg(test)` handler sites
> (no shared test-DB helper exists; a design decision, not a move). **No
> runtime authorization change.** Not F8-08 (erasure) — that is **R69**, and it
> is the highest-severity item still open. Not F8-03/04/06/07/09/10 (**R70**),
> K8-* (**R71**, the openclaw fork), R8-01/02/03, S8-11, L8-*, P8-01, K8-15
> (**R72**).
>
> Predecessor: **R2 "Consumer"** — the replay gate gets the
> production consumer it was shipped without, and the one verification gap in
> this programme's history closes. Theme: **a gate that could never fire, wired
> to a path that can — and the fixture the measurement corrected on the way.**
> The §0 sweep (`scripts/verification-sweep.sh`, 15 lanes) **ran to completion for
> the first time ever**: **15/15 PASS, `SWEEP_EXIT=0`**, all-targets **2 989
> passed / 0 failed** across 36 binaries, `crates/` 306, harness 44. Every named
> gate had passed before at `dd5ecd95`; **no full run had ever finished**, so
> nothing carried a release sign-off. It does now. (1) **The replay-determinism
> gate is wired to the LIVE release promotion** — `releases::promote_release`,
> after `chain_defect` and before `brain_delivery_core::promote`, refusing with
> an audit `Denied` and **no state change**, and never repairing. It refuses
> because it detects, not because it is broken: an identical trace still reaches
> `allowed`. **`DenyReason` was NOT extended** (frozen crate enum) — the reason
> travels as a server-namespaced `replay_divergent` /
> `replay_insufficient_evidence` string, derived from a new `refusal_code()`
> kept deliberately separate from `refusal_reason()`. **Both red-proofs fired,
> and the second is the load-bearing one:** removing the gate promoted the
> divergent trace (`left: "allowed"`), and a planted **always-refuse** gate left
> that first proof **GREEN** — caught only by the anti-vacuity pin. Without that
> pin the round could have shipped a gate that refuses everything and proves
> nothing. (2) **Two premises the brief states measured FALSE, and both are
> refusals rather than implementations.** `classify_replay` is **not** on the
> classifier axis — it compares re-derived vs recorded **stage digests** in
> `delivery_traces` and **never calls the classifier**; `classifier_gold_set` /
> `predicted_category` appear **zero times** in `src/ crates/ tools/ client/`.
> And the promotion the brief reaches, `create::promote`, is **inert by
> construction**: `PROMOTION_ENABLED` is a compile-time `false` checked FIRST,
> and its route never even calls `promote()`. Wiring a gate there would have
> produced a green suite over a function no caller can reach. (3) **Two things
> the round found rather than assumed.** The first fixture rewrote a `seq` and
> expected one mismatch; it produced **three**, because the sort order *and* the
> digest (`canonical_bytes` includes `seq`) both moved — **the exact defect the
> gate's own fixture doc warns about**. The pin now asserts the **verdict, not
> a count**. And `InsufficientEvidence` is **unreachable at this seam**:
> emptying the traces makes `live_subject_digest` answer `no_artifact` first, so
> the artifact law precedes the gate — recorded as a **ceiling**, and
> `r2_c` asserts that fact rather than a discrimination the seam lacks.
>
> **Spire at ship:** needle-measured `src`+`tests` **3 069** `#[test]` (the
> canonical `find src tests -name "*.rs"` invocation), `crates/` **310**,
> `tools/` **100**. `CRATE_TEST_FLOOR` **2 758 in code — DELIBERATELY NOT
> RAISED**: the needle reads 3 069, so the floor was already 311 under the
> truth and raising it to the needle would have spent the guard's headroom on a
> measurement rather than on a round. The three other floors stay
> **255 / 214 / 200**. Lib **2 242** passed / 0 failed / 2 ignored
> (`--features bench`); all-targets **2 716** passed / 0 failed. Schema
> **1.32.23** unchanged (probe 1.32.24). `verify_claims.py` **53/53** — it was
> **50/53 and exiting non-zero**, from three pre-existing drifted figures in
> files R2 never touched; re-baselined to measured truth with the movement
> **attributed, not absorbed** (see below). **Zero new dependency edges: all
> tracked `Cargo.lock` files byte-identical**, `src/migration.rs` and
> `openapi.yaml` and `route_guards.rs` all **0 diff**. `cargo audit` clean over
> the root lockfile. Diff: `releases.rs` +336, `replay_gate.rs` +33/−6.
>
> **The debt batch (R8), landed with this line.** `chacha20` **0.10.1 → 0.10.2**
> in `tools/steward-harness/Cargo.lock` — the harness had drifted a patch behind
> the root lockfile, which already carried 0.10.2; one package, checksum-only
> delta, harness **44 passed / 0 failed**, and `cargo audit --file
> tools/steward-harness/Cargo.lock` clean over 162 dependencies. **This is the
> one `Cargo.lock` that MOVED this round**, and it moved because the brief asked
> for it, not because a new edge appeared. **And this file's own figures were
> stale:** the header said `CRATE_TEST_FLOOR` **2 618** and lib **2 818**; the
> code said **2 758** and the needle said **3 069**. The 140-line gap the brief
> flagged was real, and it was **document drift, not a typo** — every figure in
> the R59 header predated R60–R65.
>
> **What did NOT ship, stated plainly.** **Not a classifier-fidelity gate** —
> nothing in R2 reads the gold pack; that consumption is a named separate
> increment, because attaching it here would mean the round's red-proof proved a
> property of a function that does not exist. **Not a measured gate-quality
> figure** — "divergence is detectable" moves from non-claim to machine-checked
> **and no further**: no out-of-sample rate, no false-promotion rate, no owner.
> **Not R53.** `gates_vacuous` is still advisory-only and R55/R58a/R58b/R61
> remain blocked: **R2 does not unblock the critical path, and a reader who sees
> R2 land must not infer that it did.** Not an autonomy change —
> offer-never-assign stands, `refuse_agent` is untouched, and
> `PROMOTION_ENABLED` is still `false`. **No SME sign-off was performed** and no
> simulated approvals were used; **D-6 stays OPEN.** Preregistration committed
> before any code at `3f80d0c`, evidence after, code at `9fcb7adc`.
>
> Predecessor: **R59 "Gate"** — the admission-gate suite, and
> two DoD items refused. Theme: **a filter becomes a control loop — two things
> the tree already *stated* and did not *do*, and two DoD items refused rather
> than worked around.** (1) The **admission-gate suite** in `crates/gold-sets`
> (209 lines before the round; the round adds **2 428** lines across 7 files,
> **zero deletions, zero existing bytes modified**): 12 checks in the plan's
> numbering, **faithfulness executed first** (its 42.5% failure rate makes it
> the dominant property), each resolving to exactly one of mechanical /
> adversarial / **inapplicable-with-a-reason** / **negative-routes-to-a-stage**.
>
> **The real deliverable is the routing, not the vertical** — "more than the
> vertical", as the prompt put it. A negative check **names the stage that owns
> the defect** (Create/Solve/Evolve/Deflect/Operate) and the case goes THERE.
> `patch_at_gate` returns `Result<Infallible, PatchRefusal>`, so "patched at
> the gate" is **not a state the type can represent** and the SWE-Proof
> revision-routing rule is mechanical rather than advisory. That guarantee is
> **compile-time**: widening the `Ok` type breaks the build inside the pin's own
> `fn`-pointer binding, so it never produces a `test result: FAILED` line.
>
> **Inapplicable is a THIRD state, not a pass.** 3 of 12 checks
> (ContentDisposition, ReadSeam, AuditCalibration) are unreachable from this
> workspace node, each with a measured reason **and a `REOPEN` condition**. A
> case with an UNDECLARED inapplicability is refused; an admitted case **names
> its gaps in its receipt**, so **admitted never looks green**.
>
> The suite caught a defect in **its own attacker panel**, and **the pin caught
> it before a human did**: `Mutant::DropSurface` popped an element from
> `required_surface` and declared Faithfulness as its watcher, but popping a
> requirement can only make coverage look BETTER, so a coverage check is
> structurally incapable of noticing. It is now `UncoveredSurface`, which
> **widens the CLAIM**. A **declared survivor** mutant
> (`HumanVerdictContradiction`) is declared, not assumed: no mechanical check
> reads the corpus's `human_pass` verdict, so it survives — and the panel now
> *verifies* the declaration, reporting a stale one as its own negative.
>
> **What did NOT ship, stated plainly.** **`I59.4` STOPPED and `D59.5` recorded
> UNSATISFIABLE** — not "hard", unsatisfiable. All four routes to a production
> consumer for `brain-care-core` were measured closed **between the prompt's own
> §7 and §10**: the root manifest edge moves `Cargo.lock` (§7 forbids it); the
> harness edge is not production (`publish = false`, dev-dependency only, and
> §0's own table classifies harness-only as NOT consumed); the internal
> unconsumed-island edge is forbidden by name (§10); and R57's
> `the_census_reaches_the_corpus_without_a_new_dependency_edge` forbids the
> manifest edge outright. `cargo tree --invert brain-care-core` still returns a
> **bare node** — the dead vertical points at itself via `brain-interview-core`.
>
> **`D59.6` reports "insufficient corpus"; `D59.7` reports the panel
> UNCALIBRATED.** Both escapes were preregistered *for use* and used. **No check
> was shrunk to fit the corpus**, and **no threshold was invented for a
> calibration the assessment never supplied** — its only figure is the phrase "a
> third to two thirds", and **a band that is a sentence is not a measurement**.
> The claim R59 was bought for — §7's "One vertical is real", falsified by "a
> number without provenance" — **is NOT MADE**: the suite is real, the vertical
> is unwired, and **by the execution order's own falsification criterion the
> round did not buy the claim.**
>
> **The critical path did not move.** R59 does not unblock R55/R58a/R58b/R61;
> they wait on the R53 decision. `R53's halt is unchanged:` `gates_vacuous` is
> still advisory-only, and **a reader who sees R59 land must not infer that it
> did.** `R59.4` is **not** `R53`'s fork — R59.4 is about a gold case's
> assertion, R53 is about the harness's `gates_vacuous`; neither resolved,
> evaded, nor pre-empted the other.
>
> **The fifth wrong number — and this time in the handoff instrument itself.**
> The prompt's §9 baseline said lib **2 158** passed; measured **2 818**: the
> same digits transposed, **a number that was never a measurement**. That is
> the **fifth** inherited figure wrong in consecutive rounds (after `crates/`
> 281 vs 275), and sharper than the last — **the corruption is now in the
> instrument that hands the work over, not only in predecessor ship notes.** Two
> more of the round's own brief's premises measured false at the first
> measurement: the pre-existing pin count is **11, not 7** (the 7 is the frozen
> *case* count), and `lib.rs` had **5** pre-existing tests, not 4.
>
> Spire at ship: `crates/` **275 → 306**; lib **2 818** passed / 0 failed / 3
> ignored (31 binaries); `main_suite` unchanged; harness **29**, `--test gold`
> **6** unchanged; `gold-sets` **5 → 36**. `CRATE_TEST_FLOOR` **2 618 —
> DELIBERATELY NOT RAISED**: the needle `find src tests -name "*.rs"` does not
> walk `crates/`, so hand-counting this round's 31 additions would have set it
> wrong by 31. The three floors stay **255 / 214 / 200**. Schema **1.32.21**
> unchanged (probe 1.32.22). `verify_claims.py` **53/53**. Full gate **22/22** —
> including two steps that **FAILED first on the round's own new code** and were
> fixed at the root (`expect_used` denied in library code, restructured into a
> `let-else` returning the same Negative, which also removed a duplicate bounds
> check; and 42 unformatted hunks → `cargo fmt`). **Zero new dependency edges:
> all tracked `Cargo.lock` files byte-identical.** `cargo audit` clean over every
> audited workspace in that round. All six frozen gold fixtures hash-verified
> **IDENTICAL** to `6dcfff13`. Preregistered before any code at `6182f47`,
> evidence after at `b7b829c`, shipped at `7001e478`.
>
> **Honest note on the "80-line vertical".** `brain-care-core` is 80 lines, but
> its only production decision is a **two-element array membership test**
> (`CARE_KINDS.contains`), its one production import (`DraftStore`) is held in a
> field **no production path reads or writes**, and every ambiguity/draft/repair
> call lives in `#[cfg(test)]`. With `brain-interview-core` the island is 552
> lines, but 472 of those are interview machinery this crate does not exercise
> in production either. **The behavioural figure, not the line figure, is the
> load-bearing one** — which is how the round's preregistration was amended.
>
> Predecessor: **R57b "Cite"** — decision lineage + the trace
> citation. Theme: **two things the tree already *stated* and did not *do*, and
> three plan premises that measured FALSE before a line was written.** (1) The
> **three-site lineage append**: `write_handoff_transition`,
> `write_back_referral_return` and `advance_pipeline` already emitted a
> session-store row and an audit row, and **no lineage event** — while the
> operator's timeline (`GET /workflow/runs/{id}/events`) is served by the
> `outbox`. A decision was on two of three chains and invisible on the one an
> operator reads. One helper over the EXISTING `append_lineage`, topic
> `case/decision`, refs-only payload, idempotency key `(run, kind, subject,
> outcome)`, each append inside its caller's EXISTING transaction. (2) The
> **trace citation**: three additive nullable columns on `decision_run_traces`
> using the same names `decision_evaluation_runs` already uses, written at the
> host seam from **what `resolve_for_execution` returned** — never re-derived,
> never the requested key. Schema **1.32.20 → 1.32.21**, the coupled migration
> shipped as one unit and the refuse-newer probe moved to **1.32.22**.
>
> **Three premises measured false, and the tree won each time.** (a) The plan's
> **second write seam does not exist**: `grep -c "decision_run_traces"
> src/workflow/delivery.rs` → **0**; the delivery loop writes `delivery_traces`,
> which **already** carries `model_ref` + `config_digest`. The plan conflated
> "two callers of `resolve_for_execution`" (true) with "two writers of the
> trace" (false) — **one** seam, and a pin now holds it at one. (b) `P57b.2`'s
> re-anchor claim is **partly false in its load-bearing half**:
> `anchor.rs:238` deliberately does NOT compare `schema_version`, so a migration
> cannot read as the behind-the-chain tamper class — `--verify` printed OK across
> the bump. (c) `P57b.5` predicted `main_suite`'s global `COUNT(*) FROM outbox`
> assertion survives "because the steering control is about a refused write" —
> the outcome held, the **reason did not**: those tests are per-test isolated
> (`drawbridge_state(&tmp)`), and the plan named **one** such assertion where
> the tree has **two**.
>
> **The round's own most instructive failure was authored by the round.**
> `I57b.3` first shipped `citation: Option<&ModelCitation>`, which leaves "a
> trace row cannot be written without its citation" true only of a convention;
> it is now `&ModelCitation` and `NULL` is unreachable on a new row. The pin
> proving that ALSO string-scanned the handler for `registry_row.…` — the exact
> R57 anti-pattern — failed on its first run because `cargo fmt` reflowed the
> line, and was **deleted** in favour of driving the real seam. No new route (so
> no `openapi.yaml`/authz/guard-table row is owed; the three floors stay
> **255 / 214 / 200**), **no new table**, **zero new dependency edges**,
> `Cargo.lock` byte-identical. Spire at ship: crate floor **2 613 → 2 618**
> (walk-measured), lib **2 158** passed / 0 failed, `main_suite` **329**,
> `crates/` **275** measured, `verify_claims.py` **53/53**, `cargo audit` clean
> over all three workspaces, full gate **18/18**. A sixth gap, found at the
> floor and not in the code: the predecessor's record claimed `crates/` **281**,
> and `crates/` is **byte-identical to R57's own commit** (no commit touching
> it since `0a0315f1`, no working-tree change), so the true count under the
> canonical invocation is **275** — R57's number was overstated by 6 and is a
> *record* defect, not a regression. The source-needle reads 279, so 281
> matches neither the run nor the needle. **A number shipped without anyone
> diffing it against a measurement** — the exact failure this programme's
> preregistration discipline exists to prevent, found in the record the
> discipline produced. `R53's halt is unchanged:**
> `gates_vacuous` is still advisory-only and R55/R58a/R59 are still blocked —
> **this round did not advance the critical path, and a reader who sees R57b land
> must not infer that it did.**
>
> Predecessor: **R57 "Census"** — the scoreboard, scoped down to
> what the tree can actually prove. Theme: **four of the plan's eight items
> shipped; the other four are named findings, because their premises measured
> FALSE.** (1) The **drift census**: the frozen gold corpus re-scored every run
> and diffed cell-by-cell against a committed baseline under **one** global
> tolerance (500 units, preregistered before any measurement). A cell with no
> baseline is **refused, never scored**; a breach past tolerance lands as a
> hash-chained `findings` row (`source=drift_census`, the **second** writer under
> the run-less `run_id=0` sentinel, named rather than incidental) and the verb
> exits **non-zero**. A clean pass writes **nothing** — green rows are noise and
> a breach buried in them is unread. Cadence is **externally cron-driven on
> purpose**: a shipper inside the server it measures is a correlated failure.
> (2) The **ranked gap queue** with a preregistered exploration quota, a spend
> ceiling over **cost** (never over the forecast — a budget that rewards a flood
> for promising more inverts), and a kill condition measured on the census's own
> tolerance; one item traced end to end to `promotion_disabled`, and **the refusal
> is the traceable end state** — the R50 non-claim was not weakened to make a
> trace complete. (3) `ttr` ships as an explicit **NON-CLAIM, pinned**: no
> resolution event exists anywhere, and a backfilled number is a fabrication
> wearing a measurement's name.
>
> **Four plan premises measured false, and the tree won each time:** `I57.4`
> schedules falsification for a disproof representation **that does not exist**;
> `I57.5` attributes to model versions **recorded on no row** (independently
> confirming R53a §9.4's ceiling, now load-bearing); `I57.7`/`I57.8` fall back
> to an R55 registry that **has not shipped** (blocked on R53's halt);
> `I57.1`'s "lineage keyed to the case" is **not expressible** because `claims`
> has no `run_id`. A fifth, found this session: **the gold corpus is not
> reachable from the server crate** without a new workspace path edge — so the
> census compiles the packs in by `include_str!` and **`Cargo.lock` is
> byte-identical, zero new dependency edges**. A bespoke per-cell tolerance is
> **unrepresentable**: planting one first failed to COMPILE, because `Cell` has
> nowhere to put it.
>
> Schema `1.32.20` **unchanged**; **no new route** (a CLI verb, so no openapi
> row and no guard-table row are owed), **no new table** (the existing `findings`
> table is reused, not duplicated), **zero new dependency edges**. `cargo audit`
> clean over all three audited workspaces. A prior round's pin proved itself: a
> tolerance-inheritance pin **passed with the pathology planted**, and was
> rewritten to read the type's field list instead of a hand-chosen string list.
> **No existing test was weakened and no baseline row was edited to make a gate
> pass** — four gates fired and each was fixed at the root. Spire at ship: crate
> floor **2 544 → 2 613** (walk-measured; the needle does not walk `tools/`), lib
> **2 158** passed / 0 failed, `crates/` **281** (the `gold-sets` calibration lane
> is no longer dead), `verify_claims.py` **53/53**. R53's halt is **unchanged**:
> `gates_vacuous` is still advisory-only and R55/R58a are still blocked — this
> round did not make that decision, and a reader who sees R57 land must not infer
> the critical path advanced. It did not.

> Predecessor: **R50 "Create"** — the first loop that *authors*
> knowledge, shipped **INERT**. Five phase cores, a typed claim record, and a
> four-trigger database fence. **No claim reaches durable state**: the promotion
> route returns `promotion_disabled` in every configuration for every actor,
> behind a compile-time constant with no env var and no flag. Four non-claims
> are stated in `docs/create-loop.md` in those words — the out-of-sample
> false-promotion rate is **not yet measured**; the promote route is
> **disabled**; gap generation has **no reliable published detection method**;
> the set-level control has **no published prior art** and is a declared
> approximation over declared predicate interactions. The fence keys on
> application-set strings, so it defends a compromised **model path**, not host
> compromise — the boundary the audit chain already draws. Schema `1.32.19`,
> additive only, four tables, no rebuild; the gated read is a query, never a
> view. One new **workspace path edge** (the evidence crate the gate calls and
> does not reimplement); **zero new registry edges**, so `cargo audit` over the
> root lockfile is unaffected. The R46 pin that forbade the kernel edge is
> **amended, not deleted**, and still refuses a registry edge. If a later round
> quietly gives promotion a production caller, that is a broken invariant — say
> so in the evidence file, do not quietly enable it.

> Predecessor: **v1.28.92 "Ledger"** (2026-09-22) — the governed loop's
> record layers. Theme: the 1.32.x line stamped through 1.32.7 "Diagnostic
> Closure", with two preregistered record layers riding the loop's own rows
> additively (no new table, no migration) and the System-1 decide modules
> landing pure. Fifty-four commits, six prereg-first rounds (R16–R21), every
> round hash-pinned before its first edit with evidence after; zero new
> runtime dependency edges. (1) **1.32.7 "Diagnostic Closure"**: R17
> TreeHandoff (session tree + compaction admission as harness substrate) +
> R18 triage duty (ESI/MTS acuity, monitor-only beside P-class), red-flag
> forcing function + monotonic lock + per-domain must-miss catalog (with a
> shipped 6-entry `health` table), NAM-step-6 closure gate at the single
> resolution seam, back-referral contract + overdue HITL sweep that never
> auto-resolves, I-PASS pre-fill sender-owned only. (2) **R20 disagreement
> corpus (Reflect/learn)**: closing-tx reflection derived ONLY from audited
> gate rows (free text can never mint a tuple), proven retrospective-only
> (same case twice byte-identical), DPO dual-gate export with seam
> de-identification + frozen train/holdout partitions. (3) **R21 StewardOS
> accounts (deliberately-not-a-CRM)**: `kind=account` workflow rows,
> identifiers only, `decision_ref`-gated pipeline the machine never advances,
> history as pure decision join; six account routes + corpus export = seven
> new record-layer routes. (4) **R19 LAYA Phase 0**: decide pure modules
> (lang/router/sequence/calibration/presets + splitter), 134 tests, zero
> Cargo change, zero behavior change — ungated with no callers. (5) **Exec
> OS boundary**: typed sandbox seam (deny-default sandbox-exec / Landlock,
> fail-closed on unavailable backend); the X-W7 unwired pin retired by
> design with the Loop landing. Security posture: machine-refusal law at
> surface AND core (`decision_ref` required, agent class refused pre-write),
> DPO dual gates + per-call audit on both bulk reads, probe-blind 404s on
> every new id-scoped route, CI on the `cargo-audit` binary over all
> lockfiles + the conformance two-door rule. DELIBERATELY ABSENT: the 1.32.8
> classifier consume (opener-gated on the operator labeling round — the lane
> stamps when that lane ships, not before). Spire at ship: routes 216,
> tests 2007, coverage 180, authz 164; crate floor 1,568.
> Predecessor: **v1.28.91 "Notary"** (2026-09-15) — the operator-held
> evidence pair. Theme: two disclosed ceilings narrowed by two CLI verbs —
> the off-host witness and the physical shred. Zero wire; zero routes; no
> schema. (1) `brain anchor` / `--verify`: a deterministic state
> fingerprint (audit chain head + knowledge content census + row counts)
> the operator records OFF-HOST; `--verify` recomputes and diffs — any
> state change trips it, the audit chain explains legitimate ones, and a
> moved knowledge census on a clean chain is exactly the R7-08
> behind-the-chain tamper class no in-tree verifier caught. Read-only by
> design (an anchor's own audit row would move the head it fingerprinted;
> the off-host copy IS the evidence). Pins: tamper detection (the attack
> reproduced), chain truncation, reopen determinism, VACUUM stability,
> line round-trip/refusal. (2) `brain shred`: the physical residue drop
> after a logical purge — secure_delete=ON (readback asserted) →
> wal_checkpoint(TRUNCATE) → VACUUM → second TRUNCATE → integrity_check →
> one hash-chained `forget` row; freelist reads back 0; the marker pin
> proves the deleted bytes greppable pre-shred and absent from main AND
> wal post-shred. Ceilings printed per run: filesystem copies, `.bak`,
> standby chunks, SSD wear-leveling stay operator-level. (3) Register:
> the aarch64 CI-execution gap CLOSED not-applicable (no Jetson/fleet
> deployment; reopen at first aarch64 fleet deploy); K7-01/02/04 FINAL
> (no upstream PRs — procedural compensating controls in THREAT_MODEL
> §5b); CodeQL #74 rode ahead (`b695c77`). Floor walk: 1,455 → 1,462
> (seven new pins, all in the two new lib modules, CLI-only consumers —
> the standby precedent). **Log-gap note:** v1.28.89 "Bounded" and
> v1.28.90 "Refresh" shipped without rows in this file — per-release
> detail lives in `CHANGELOG.md` §[1.28.89]/§[1.28.90].
> Predecessor: **v1.28.88 "Clocktruth"** (2026-09-14) — seventh-pass
> closures, release 3 of 4. Theme: clocks, labels, and guards at law — the
> one legally-wrong clock in the repo, the guard that couldn't see two
> subdirectories, and the docs rows that outlived their debunkings. Zero
> wire; zero routes; no schema; mostly docs + two small code fixes.
> (1) L7-01: the CRA runbook's final-report clock was legally wrong for the
> vulnerability trigger (one month for BOTH; law: ≤14 days after the
> corrective/mitigating measure is available, Art 14(2)(c); one month binds
> severe incidents only, 14(4)(c)); runbook split by trigger, CSIRT framing
> corrected to single-platform → coordinator CSIRT (main establishment) +
> ENISA, reg_watch citations re-numbered to final-OJ (14(1)-(2)/(3)-(4)/(5),
> Art 71(2)) and the AI Act horizon re-cited to Regulation (EU) 2026/1744
> (OJ CONFIRMED — L7-04's fallback not needed); Annex III 2027-12-02 /
> Annex I 2028-08-02 deployer horizons stamped in COMPLIANCE.md; drill
> script template + timing report carry both clocks. New pin
> `reg_watch_runbook_clock_anchor` anchors the 14-day wording (RED→GREEN).
> Citations re-verified 2026-09-14. (2) R7-09: the transport-free guard's
> collector extracted + made recursive (the no-SQL walker idiom) — the four
> `dsar/`+`lifecycle/` files were invisible to the top-level walk;
> `transport_free_guard_walks_recursively` floors subdir files at the
> measured 4 (plan's draft ≥5 declined — walk-measured truth); red-proof:
> planted `use axum::` in `lifecycle/` failed the new guard, never landed.
> (3) The docs-truth batch: T7-02 tamper-evidence scope sentence; T7-03
> staleness rows re-stamped (THREAT_MODEL ×3 + R-14 + R-06's same dead
> cell; residual = registry-unavailability-fails-closed); T7-04 the
> crypto-inventory PRIMITIVE CENSUS (closed 8-row crate→inventory mapping +
> crypto-family heuristic over `[dependencies]`; planted `p256` fails it,
> never landed); T7-05 stamps moved + the standing same-commit stamp
> policy; T7-06 verify-JSON scoped as the consumer's out-of-band act;
> R7-10 docstring math fixed ("systme"→"sysetm" + boundary negative pin, no
> verdict change); R7-11 cross-chunk weld scope disclosed (chunker arm +
> THREAT_MODEL ceiling); L7-02 TIDA dates un-inverted; L7-03 CA 2026-09-10
> package (SB 1119) + multi-state chatbot family (GA SB 540, OR SB 1546);
> L7-06 AI RMF mid-revision footnote. (4) L7-05 HONEST CEILING: SBOM spec
> 1.3 → **1.5** — cargo-cyclonedx 0.5.9 (latest) emits 1.3/1.4/1.5 only and
> reads NO config file (the plan's `.cargo/cyclonedx.toml` route does not
> exist); `--spec-version 1.5` pinned in sbom.sh, one-flag bump when
> upstream ships 1.6/1.7. Floor walk: 1,455 (1,452 → 1,455; all three new
> pins ride plain `#[test]`). Predecessor: **v1.28.87 "Ownerstamp"**
> (2026-09-14) — seventh-pass
> closures, release 2 of 4. Theme: the seams' last mile — the DSAR root
> semantics question, the one roster that attested a seam it lacked, the
> admin-evidence surfaces the unconditional read-seam law hadn't reached,
> and the site-table guard hardened to read code, not prose. (1) F7-02: the
> STAMP decision (a) — every content write carries an owner stamp
> (`content_owner_stamp` + the fixed `loopback` label for the opaque
> superuser; JWT subs unchanged) at five write edges; the DSAR locate
> (`owner = subject`) now covers operator-authored ingests. Write-side only,
> no migration: historical NULL-owner rows stay stamp-blind by declaration
> (dated); NO OR-arm sweep (a legacy arm would mis-attribute every
> NULL-owner row in multi-principal trees); `suggest_feedback` keeps
> sub-or-NULL (disclosed ceiling); procedure rows carry no owner column at
> all (schema-level, beyond the no-schema scope). Live drill: ingest →
> `/dsar` export for `loopback` → `roots:1` (was 0); sqlite readback
> `owner=loopback`. (2) F7-05: both crew views ride the seam at the emission
> map — the roster core's invisible pass covered `principal`/`current_case_ref`
> only; `roles`/`skills`/site now strip too; the skills-view comment is true
> now; site-table rows for both. HONEST RED-FIRST NOTE: the first pin
> attempt planted only the two core-stripped fields and PASSED pre-fix —
> reshaped to plant hostile roles/skills/site before the fix landed.
> (3) F7-06: `sanitize_value_strings` (deep string-leaf seam composition) at
> nine emission sites — breach list/detail, TIA/DPA, roles/profiles list+get,
> `/audit` actor; no digest impact (none bind `review_digest`); static TIA
> prompt text verified seam-clean. (4) F7-07: `handler_body` comment-strips
> sources (string-aware: line/block/doc comments, strings, the `'"'` char
> literal, `r#"…"#` raw strings) before the substring assert — owned-body
> signature change inherited by every consuming guard (authz coverage,
> screen routing, read-seam table, audit-order); red-proof pin covers the
> false-pass + honest call site + lexing hazards; the same-commit site-table
> row is now a release-checklist standing rule. Pins: the three surface pins
> RED→GREEN + `handler_body_ignores_comments_naming_the_symbol` +
> `content_owner_stamp_always_attributes`; site table +12 rows. Floor walk:
> 1,452 needle-visible (1,450 → 1,452; the three surface pins ride
> `#[tokio::test]`, which the spire needle doesn't count — floor set to the
> walk-measured truth). No schema; no routes; openapi.yaml untouched;
> x-api-version unchanged (no wire move — content-level hardening).
> Predecessor: **v1.28.86 "Attrbane"** (2026-09-13) — seventh-pass
> closures, release 1 of 4. Theme: close every open seam-door and wire every
> dormant defense. (1) F7-01: the read seam gains the ATTRIBUTE tier inside
> the hostile-element fixpoint — `on*` handlers and
> `javascript:`/`vbscript:`/`data:` schemes (one bounded entity-decode pass,
> whitespace/control compaction) drop from SURVIVING elements; scheme-hostile
> not attribute-hostile (benign http(s) hrefs byte-identical; the .76 weld
> family unchanged; `sanitize_read_cow` fast path untouched). (2) F7-03: the
> graph family rides the seam — four handlers + the traverse mapper via
> `sanitize_read_cow` (site-table rows added); the markdown write edge is
> DECLINE-AND-COUNT (`normalize_name`/`normalize_rel_type` or skip, response
> `edges_skipped` + in-tx audit note; hostile heading 200s, never 400s, never
> becomes graph structure); structured `entity_type` gains the closed
> charset (400s; lowercased first). (3) F7-04: `AuditKind::Procedure` +
> in-tx row in `store_procedure`; knowledge-row audit beside the edge audits
> in `store_record`; `/add` + `/ingest/markdown` audits moved INSIDE their
> txs (the post-commit crash window closed; rollback twin = trigger poison).
> (4) S7-01/02/03: plugin 0.6.9 — `sanitizeForBlock` INVOKES the
> hostile-element mirror (server-canonical position); proposal details
> become a sanitized projection (sourcePrompt dropped), traverse paths +
> decision rule text + label fields ride the boundary; `provenance`/`evidence`
> get a deep string-leaf sanitize. Fork synced (vitest 71/71, tsc clean,
> byte-parity). (5) S7-04: sync-plugin's post-sync check is the
> declared-exception form AND the format.test.ts delta is eliminated
> canonical-side. **DIGEST DISCLOSURE:** the tier widens `sanitize_read` →
> rows with strip targets move `review_digest` → outstanding approvals 409
> at approve (observed LIVE: pre-M1 digest → 409, re-review → 200) —
> re-review required, expected, not avoided. Live drill: canary rows raw on
> .85 / attribute-free on .86; hostile-heading ingest 200 `edges_skipped:2`;
> procedure evidence row on the chain (verify 6/6 signed); live DB untouched
> (hash-verified). Floor walk: 1,450 crate `#[test]` (the plan's +6 are real;
> four ride `#[tokio::test]` which the spire needle doesn't count — floor set
> to the walk-measured truth). Plugin attribute tier stays server-side by
> design (the mirror is the element backstop). x-api-version moves with the
> crate version (informational stamp; the wire contract delta this release:
> additive `edges_skipped` only). No schema; no routes. Proof commits:
> `713748a`, `0d797ba`, `48fef68`, `15a7c99`.
> Predecessor: **v1.28.85 "SixthPass"** (2026-09-13) — the sixth-pass
> audit's closures. Theme: evidence labeled what it is, mirrors matching.
> (1) Forget erasure audit rows carry the Forget kind (G6-02): the erasure
> and per-proposal scrub rows wrote kind `ingest`; both now write kind
> `forget` (red-first pins failed pre-fix, green post-fix). Historical rows
> keep their meaning. (2) Fork extension carries the hostile-element
> mirror (G6-01): synced to plugin 0.6.7, byte-parity verified, 70
> extension tests green, typecheck clean. Prev: v1.28.84 "Quarterly"
> (SSE revocation kill + required webhook signing + 26-element strip +
> docs-truth pass; see `CHANGELOG.md` §[1.28.84]).
> (4) Newer-schema databases REFUSE to open (was: undefined behavior on
> unknown columns) with migrate-rehearse parity (55 tables); the static
> embedder gains a std-only saturation gauge (`SatGauge`/`SatGuard`) so
> the .76 serialized-inference cost class is measured, not discovered
> under load. (5) Docs-truth: README UMP badge derives from the CI
> conformance gate (loud degrade to "self-attested" when absent);
> `scripts/env-truth.sh` is the docs-vs-code env gate; the release
> checklist discloses SBOM scope per CISA-2026 (runtime closure, 375 vs
> 520 lockfile packages — NOT the dev+build tree), the 8-route
> intentional OpenAPI exclusions, and the 7-route well-known wiring. (6)
> A 25-row error taxonomy with operator-safe `Display` impls is pinned
> (`tests/error_taxonomy.rs`); `tests/singularity_pins.rs` adds 7 pins
> (incl. revocation-cache ZERO staleness — the "60s" claim debunked).
> (7) CodeQL #73 cleared (generated test key, `7d63f32`). WIRE: the
> `/ready` probe returns JSON (was text/plain) — the release's only
> wire-visible delta; consumers scraping the body must read `status`.
> No schema; no routes; x-api-version unchanged. CRATE_TEST_FLOOR
> 1,418 → 1,448. Ceilings (honest): SSE re-auth is polling (a revocation
> lands within the interval, not instantly); signing covers the two env
> sinks (hostcall HTTP keeps its .69 loopback allowlist); the gauge
> observes the static embedder only. Proof commits: `7d63f32`,
> `2567d84`, `60c344c`, `935d215`, `89a6233`.
> **Log-gap note:** releases v1.28.77–v1.28.83 ("Recall" is .83, the
> fifth-pass fix release; "Vigil" .82, "AgBOM" .81) shipped without
> notes in this file — per-release detail lives in `CHANGELOG.md`
> §[version] and the SECURITY.md history table for that span.
> Predecessor (last noted here): **v1.28.76 "Selfheal"** (2026-09-09) — the second-pass
> audit's fix release (docs/SECOND_PASS_AUDIT_20260909.md; 30 findings
> across both trees). Theme: nothing stripped may reassemble, and no gate
> has a side door. (1) BOTH read-seam strips are bounded FIXED-POINT now —
> `<scr<script>ipt>` welded into a live `<script>` and
> `[![a](i) c](o)` into a live auto-fetch image under the one-pass
> strips (demonstrated, then fixed; overflow fails closed by dropping the
> trigger bytes). (2) The ONNX scorer is budgeted (64 sentences / 16k
> chars — every inference serializes on one mutex; a 1 MiB ingest pinned
> all screened writes); embedder input budgeted at 8k chars (stored text
> verbatim; one helper, all backends). (3) X-W4 completion: valet `what`
> vets at the CAS seam too (`valet_what_refused` + Denied audit). (4)
> Kill-switch reach: `/auth/refresh` (public route — 401
> `identity_revoked`) and console actors (`actor_revoked` before
> capability). (5) Live `valet/due` SSE rides the opt-in + domain gate
> (replay already did; private labels streamed unfiltered). (6) Egress
> table: mapped-v6 normalizes into the v4 table + NAT64/6to4/Teredo/
> discard rows. (7) MCP read-scope denies `ump.feedback`. (8) Purge
> deletes suggest_feedback rows for purged chunks (+ sweep tenant arm).
> **.75 correction of record:** the `exec_spawn_carries_kill_on_drop` pin
> was VACUOUS (its target string occurred only in the assertion) and the
> exec spawn is std::process (no kill_on_drop) — the honest mechanism is
> the deadline block, re-pinned behaviorally (`exec_deadline_kills_child`,
> injectable `exec_effect_for`). Docs truth: THREAT_MODEL §5b (the
> .63–.75 controls; was frozen at .68), SECURITY history (was stopped at
> .17), OWASP matrix re-stamp, AI_LITERACY/openclaw-integration/plugin-
> CHANGELOG current, repo-brief.sh un-crashed (route sites live in the
> router). FORK: Truthglass + Pin merged to fork main 09-09
> (cherry-picks `1e5d31fb85e`/`fde920cb23a`/`6d54452a029` + the
> localeCompare→code-unit pin fix; fork CHANGELOG/lockfile byte-untouched;
> branches deleted, tips recorded in the audit doc). Digest-invalidation
> disclosure: the strips move `review_digest` for weld-bearing rows →
> fail-closed 409 at approve, re-review required. CRATE_TEST_FLOOR
> 1,363 → 1,372 (+10 pins). No schema; no routes; x-api-version unchanged.
> Planned: v1.28.77 "Erasure", v1.28.78 "Unconditional", v1.28.79
> "Ceremony" (audit doc §4).
> Predecessor: **v1.28.75 "Preflight"** (2026-09-08) — the PROGRAM
> EXIT GATE; the 1.32.x Loop line may open. (1) X-W7: the dormant exec
> mediation HARDENED (argv0 canonicalize-and-refuse-divergence closes
> the symlink-masquerade door; the danger screen is the documented
> TRIPWIRE and gains the pipe-to-shell family; kill_on_drop pinned at
> the spawn seam) + `hostcalls_mediation_stays_unwired_until_loop_line`
> — dormancy is a declared, machine-checked state (the Loop wiring
> commit DELETES the pin by design). (2) X-A4b: the installer writes
> `BRAIN_WRITE_POSTURE=review` for plists with NO explicit posture only
> — operator-set values are never stomped (the old unconditional
> remove+insert did, every re-run); the completion message names the
> posture + the opt-out; compiled default stays `open`. (3) X-C5/C6:
> the two ceilings are DOCS TRUTH in THREAT_MODEL + SECURITY reporter
> scope (chain key + pin share the host — detects SQL-level tampering,
> not host compromise; live DB + `.bak` plaintext on the primary,
> follower-only encryption law). (4) X-C8: `badges.sh --selfcheck`
> REFUSES without the committed `sbom/brain-server-<version>.cdx.json`
> — the human generate+commit step is unforgoable. (5) M5: the program
> close-out in `docs/AUDIT.md` — 55 findings × disposition (the plan's
> "41" undercounted), the four-leg exit-gate drill (channel/out forge,
> steering launder, revoked principal, poisoned-memory canary — ALL
> fail closed), per-release deltas, surviving ceilings. CRATE_TEST_FLOOR
> 1,358 → 1,363. No schema; no routes; x-api-version unchanged.
> RELEASED as tag `v1.28.75` (2026-09-08) at the follow-up fix commit
> `4890d89`: Ubuntu CI (merged-usr, /bin -> /usr/bin) caught what macOS
> could not — the admission canonicalized only argv0, turning honest
> textual allowlist entries (/bin/ls) into refusals. The fix
> canonicalizes the ALLOWLIST ENTRY too (honest aliases admit; planted
> symlinks and un-named siblings still refuse, pinned); release.sh
> refused the tag on the red matrix exactly as designed, and the
> release shipped only after green. docs/AUDIT.md carries the close-out.
> Predecessor: **v1.28.74 "Origin"** (2026-09-08) — taint labels
> survive the whole trip; THREE TREES (X-S2 proportionate, X-F3). (1)
> brain: `/ingest` + `/ingest/proposal` accept `origin_context:
> "owner"|"channel"` (absent = owner; unknown = 400); channel captures
> store origin `channel-capture`; the review-queue proposal SOURCE
> carries the badge and approval promotes it onto the row. (2) plugin
> 0.6.0: hit lines prefix `[memory | channel-capture]` inside the fence
> (owner untagged); `untrustedOrigins: "label"|"exclude"` (default
> label) drops captured hits from AUTO-INJECT under exclude; the
> memory_recall TOOL path always labels; autoCapture sends `channel`
> from the gating chat-type. (3) openclaw fork: the inbound boundary
> marks quoted/replayed `[memory | …]` prefixes as `[quoted memory ·
> origin: … — untrusted replay, not fresh prose]` (no brain-store
> coupling — one textual convention). (4) X-F3: span attribute values
> pass ANSI/C1 strip + unconditional PII redaction (`domain` at recall/
> gate spans); `query_hash` untouched; the OTLP export path logs
> "telemetry attributes are sanitized; treat any collector as
> untrusted infrastructure" at startup. No lattice, no policy engine —
> ONE boolean-grade label. CRATE_TEST_FLOOR 1,356 → 1,358. openapi
> additive; x-api-version unchanged; no schema (origin value extension).
> Released as tag `v1.28.74` (2026-09-08; the fork half in the openclaw
> repo at d724cb8).
> Predecessor: **v1.28.73 "Keyring"** (2026-09-08) — key + evidence
> lifecycle (X-C4, X-C3, X-W8). (1) X-C4: the operator signing key is
> DETERMINISTIC — fixed filename `operator.ed25519` (one-time
> transparent rename from the legacy first-file scan, logged);
> wrong-size/leaked seed → LOUD refusal (the silent L2 degrade dies);
> `brain key rotate` (inside the existing `brain key` dispatcher) moves
> current → `.prev` (verify-only, ONE key deep, second rotate refuses),
> writes a new 0600 seed, bumps `operator_key_generation` in schema_meta,
> and writes the hash-chained audit row (the register IS the audit
> chain — the plan's lineage event maps there honestly); cards gain
> `signing_epoch` (additive column, schema 1.28.73, contract test
> extended) and `verify_card` picks current/prev by epoch (legacy NULL
> = try both). (2) X-C3: a chain-less backup image REFUSES restore
> (`chainless_image_refused`) unless `--allow-chainless`; legacy-epoch
> chains restore marked `legacy_unkeyed_chain: forgeable: true` +
> disclosure evidence row naming `--re-audit`; head-pin rollback stays
> disclosed-not-refused. (3) X-W8: the replay cache evicts the OLDEST
> QUARTER at cap (was: clear-all); the revocation drain pages 10×200 and
> writes a `drain_incomplete` audit row naming the remainder.
> CRATE_TEST_FLOOR 1,345 → 1,356. No routes; wire additive only;
> x-api-version unchanged. Drills + migration notes in CHANGELOG
> §[1.28.73]. Released as tag `v1.28.73` (2026-09-08).
> Predecessor: **v1.28.72 "Scrim"** (2026-09-08) — every emitted
> surface is shaped (X-R3, X-W6, X-L4, X-E5). (1) X-R3: the read seam
> (`sanitize_read`) gains `strip_hostile_elements` AFTER the markdown-ref
> strip — a CLOSED case-insensitive attribute-greedy element-name set
> (script/img/iframe/svg/object/embed/link/meta/form/input/video/audio/
> source/track/base); prose angle-brackets survive (pinned); storage
> stays verbatim so `review_digest` does NOT move; the
> `sanitize_read_cow` fast path now requires a `<`-free row. (2) X-W6:
> the `GET suggestions` KCS evidence side-effect requires Write + the
> `workflow` role; Read-only principals get the body unchanged +
> `evidence_recorded: false` (additive, openapi'd). (3) X-L4: a denied
> `/events` subscriber gets HTTP 403 BEFORE the stream opens (was
> 200+error-event); mid-stream failures keep the error-EVENT path; the
> client driver already handled non-200; the authz matrix moved
> `/events` out of `SSE_SOFT`. (4) X-E5: the KB generator escapes
> operator-configured base_url/locales everywhere (hreflang, sitemap
> loc/alternates, canonical already escaped at render) and the library
> enforces the CLI locale contract (alnum+hyphen ≤12;
> `locale_matches_cli_contract`). Bytes ledger: stored markup vanishes
> from read seams; digests unchanged (corpus green). CRATE_TEST_FLOOR
> 1,336 → 1,345. openapi additive; x-api-version unchanged. No schema.
> CHANGELOG §[1.28.72]. Released as tag `v1.28.72` (2026-09-08).
> Predecessor: **v1.28.71 "Pores"** (2026-09-08) — the screen sees
> what the model sees (X-R4, X-R6, X-R7; plan + execution prompt in the
> repo root). (1) X-R4a: layer-1 `contains_suspicious_pattern` runs on
> `strip_invisible(trimmed)` at the `Screen::screen` seam — the
> raw-vs-stripped disagreement (bidi-split `sys\u{202E}tem:` dodging the
> line-anchor matcher) is CLOSED; verdicts can only move
> Clean→Quarantine/Reject. (2) X-R4b: the blocklist stops being 13
> English phrases — translation families (es/de/fr/nl/fil, same six
> intents, data-driven `FAMILIES` table pinned by
> `blocklist_families_cover_five_languages`), the typoglycemia anagram
> tier (first+last + sorted-middle equal, length ≥ 4; EXACT keywords
> never trip — bare "system" stays prose), and the bounded encoding tier
> (base64/hex runs ≥ 24 chars, first 8 runs, ≤ 4 KiB decode, ONE decode
> level — `encoding_scan_bounded` pins run #9 stays undecoded). (3)
> X-R4c: the layer-2 classifier AUTO-LOADS when the artifact resolves
> (`~/.config/brain-server/models/injection-classifier/{model.onnx,
> tokenizer.json}`); `BRAIN_INJECTION_CLASSIFIER=off` opts out; a
> non-existent explicit path REFUSES the boot; `/health/db` echoes
> `injection_classifier: on|off|absent`; poison posture unchanged
> (fail-open 0.0). (4) X-R7: `sanitize_log_value` routes through the
> shared `strip_control_chars` — ANSI/C1 escapes leave log VALUES (\r
> now removed, \n→space + tab-survival pinned). (5) X-R6: the bridge
> edge strips the canonical invisible class + dereferences markdown
> refs before the 4000 clamp (kernel screen stays authoritative).
> **Verdict-shift disclosure: quarantine-ward at the margin only; the
> clean-corpus pins hold; the .65 Meridian canary keeps its Clean
> screen verdict (pinned — it is a read-seam catch by design).**
> CRATE_TEST_FLOOR 1,318 → 1,336. Tests: full bench 1,426 passed /
> 7 ignored; bridge crate 39 green (its clippy is deny-lint strict —
> the scanner port is checked-arith/.get by construction). No schema;
> no routes; openapi untouched; x-api-version unchanged. Drill + the
> standing "tripwire, not boundary" note in CHANGELOG §[1.28.71].
> Released as tag `v1.28.71` (2026-09-08).
> Predecessor: **v1.28.70 "Twokeys"** (2026-09-08) — the opaque-mode
> operator/agent split; the REGISTER LINE opens. Token-file line 2
> becomes `PrincipalKind::AgentLoopback` (scoped `write:*/global`, ship
> `agent` preset, Blackout kill-switch + `agent_forbidden` audits);
> `/health/db` full body Admin-on-global (Read gets the reduced probe);
> `/metrics` per-domain labels render only in-scope (`domain="other"`
> summed collapse). Single-token deployments keep the legacy superuser
> posture byte-identically (honest F-W1 disclosure — the boot warn is
> the nudge). Full note: docs/AGENTS_HISTORY.md + CHANGELOG §[1.28.70].
> Predecessor: **v1.28.69 "Deadbolt"** (2026-09-08) — the egress and
> process boundary. THE SEAM LINE CLOSES (X-E3, X-M4, X-M5, X-M6; plans +
> execution prompt in the repo root). (1) X-E3: the shared egress client
> stops reaching private networks — resolve → validate → PIN per the OWASP
> bypass-proof form (`validate_public_addrs` in `webhook.rs`: every resolved
> address must clear the IANA IPv4/IPv6 special-purpose table — real
> `IpAddr` math, the plan's exact table incl. 100.64/10 CGNAT; metadata
> hostnames refused pre-resolution; `IpAddr` parsing + the url crate's
> literal canonicalization kill the hex/octal/dword class). The two env
> sinks (`BRAIN_ALERT_WEBHOOK_URL`, `BRAIN_DSAR_WEBHOOK_URL`) resolve +
> validate + pin AT BOOT (post-tracing-init — the drill caught the
> pre-init placement swallowing the pin logs); private sink without
> `BRAIN_EGRESS_ALLOW_PRIVATE=1` → REFUSE BOOT (WRITE_POSTURE pattern;
> the env parses fail-closed in `config.rs`); opt-out = loud warn + still
> pinned; boot DNS failure → warn + lazy fail-closed on first send
> (`egress_unresolved`); pins are insert-only (first resolution wins for
> the process lifetime — `pinned_client_survives_dns_rebind` pins an
> RFC 6761 `.invalid` name and proves the shadow listener sees zero
> connections). reqwest 0.13.4 `resolve_to_addrs` is the seam (SNI
> preserved — verified against the vendored source). The hostcall HTTP
> path KEEPS its allowlist (loopback mediation is a pinned feature — the
> public-only table does NOT apply there) and gains validate-on-first-use
> + an insert-only per-host client cache bounded STRUCTURALLY by the
> allowlist. (2) X-M4: `resolve_harness_bin`'s PATH scan DELETED —
> absolute `BRAIN_STEWARD_BIN` only (relative → named refusal, no silent
> exe-dir fallback) or beside-the-kernel; the CLI's own PATH resolution
> stays (operator context — documented ceiling). (3) X-M5: crank spawn
> gains `kill_on_drop(true)` (`run_harness_crank_bounded` carries the
> scaled test seam); the hostcall exec path's `try_wait`-error early
> return now kills+reaps too; the DRILL found the router's 30 s
> TimeoutLayer drops the handler future first — and the child dies on
> THAT drop as well (previously it survived every abandonment path).
> (4) X-M6: console `pending` requires the mapped actor's `read`
> capability via the existing `resolve_console_actor` machinery (empty
> grants nothing); decide/due/crank unchanged. Migration notes (both in
> the CHANGELOG): private-sink operators need the opt-out env; PATH
> users need absolute `BRAIN_STEWARD_BIN`. Tests: 6 egress pins + 1
> hostcall cache pin + 3 harness pins + 2 crank pins + 3 console role
> pins; the Art-19 loopback drill test now runs under the opt-out env
> (the fail-closed default REFUSES loopback sinks — the suite wedged on
> the old expectation before the fix). CRATE_TEST_FLOOR 1,303 → 1,313
> (needle re-measured at the release commit). No schema; no routes;
> openapi untouched; x-api-version unchanged. Drill transcript (4 legs)
> in CHANGELOG §[1.28.69].
> Predecessor: **v1.28.68 "Shutter"** (2026-09-07) — the image + beacon
> egress seats close upstream (docs half).
> DOCS-ONLY in this tree: THREAT_MODEL §5 "Exfiltration surfaces" (the two
> carried EchoLeak-class seats F-E1 doc-mode remote images + F-E2 favicon
> beacon, closed default-OFF + host-allowlisted in the openclaw fork —
> commits ef6a84a/f1a8b0f/c9afcb6 there; X-E4 data-URI 64 KiB decoded
> budget) + the standing ceilings (bare URLs linkified-but-inert, the
> gate.rs ceiling; allowlists are trust, not safety); SECURITY.md names the
> image/beacon class in reporter guidance; CHANGELOG §[1.28.68]. Built in
> PARALLEL from a v1.28.63 cut in the `brain-server-68` worktree, rebased
> onto the post-.67 main (ship order .64 → .65 → .66 → .67 → .68 held).
> New `gateway.controlUi.remoteImageHosts` config surface lives in the
> FORK (exact hosts, empty = fail-safe; one setting, two consumers — UI
> images + server favicon route, server re-verifies). CRATE_TEST_FLOOR
> UNCHANGED at 1,303 (docs-only); measured 1,384 bench / 1,401 default /
> 1,403 otel at the release commit. No code, no openapi, no schema.
> Predecessor: **v1.28.67 "Pin"** (2026-09-07) — identity is pinned.
> Built in PARALLEL from a v1.28.63 cut in the `brain-server-67` /
> `openclaw-67` worktrees, then REBASED onto the post-.66 main once
> Truthglass shipped (floor/version/changelog reconciled in the rebase;
> ship order .64 → .65 → .66 → .67 held). Closes the 2026-09-06 audit's
> X-M1, X-M3 (HIGH), X-C1 (HIGH), X-C2. **Honest disclosure: attribution
> was self-asserted until .67** — marks verified that SOMETHING signed the
> bytes, never WHO. (1) M1 (openclaw fork, `pin-1.28.67`): MCP catalog
> pins — per-tool `sha256(name+\0+description+\0+stableStringify(schema))`
> + per-server digest, reconciled EVERY run (openclaw materializes per
> RUN — per-run re-hash IS the per-execution cadence; OWASP §2/§7),
> `mcp-catalog-pins.json` beside the agent bundle (agentDir discipline,
> 0644, not a secret); drift notifies (name+description+fp prefix) and
> drifted tools carry `pendingAck` meta; ack is an explicit operator
> touch; corruption reads as empty → LOUD rebuild (it can never silence
> drift); colliding display renames pin the ORIGINAL server-side name;
> rug-pull demo GREEN (`scripts/rug-pull-demo.mts`). (2) M2 (X-M1):
> `BRAIN_MCP_SCOPE` ∈ read|full — fail-closed parse (unknown refuses
> boot), dispatch-time gate BEFORE the network seam on `brain_ingest`/
> `ump.remember`/`ump.revise`/`ump.forget`, additive `tools/list`
> `"x-brain-scope": "read-denied"` annotation, default `full`
> byte-identical, startup logs the scope. (3) M3 (X-C1, THE ONE BREAKING
> WIRE CHANGE): parcels `expected_signer` REQUIRED (400
> `signer_required` — migration: name your counterparty) + `signer_alias`
> 409 when the local operator did is aliased for a foreign-produced
> parcel (self-parcels pass and verify on their own signature);
> provenance verify gains the operator pin —
> `verify_artifact_detailed(value, pinned)` returns
> `Ok|ForeignSigner{signed_by}|Unsigned|Tampered|Malformed` (the pin
> checks LAST, so tampering reports Tampered under a pin),
> `verify_artifact_json` always surfaces `signed_by` (self-assertion
> visible in L2), and the four emission-adjacent verify sites (remedy,
> ADR, campaign, KB manifest) wire the pinned variant. (4) M4 (X-C2):
> `/ump/audit/verify` carries the additive `integrity` census
> `{verified, signed, hash_only}` under the serve posture + the
> transitional `note: hash_only_records_present` (key exists AND
> hash-only seen — dead code today by design); serve behavior
> UNCHANGED. CRATE_TEST_FLOOR 1,290 → 1,303 (re-measured on the rebased
> tree). Gates: full suite green; clippy bench/default/otel clean; fmt +
> lipstyk clean; engine-crates + steward-harness green; badges
> selfcheck clean; wire additive except the parcel signer
> (x-api-version unchanged, schema untouched, no new deps). Ceilings:
> drift is SURFACED not gated (no ack UX; .73 owns rotation); the MCP
> scope is process-lifetime (correct for stdio's single parent); the
> alias gate needs the local key (keyless → signer_mismatch instead);
> the fork's committed pnpm lockfile disagrees with its own typebox
> catalog (upstream drift predating this line). See CHANGELOG
> §[1.28.67].
> Predecessor: **v1.28.66 "Truthglass"** (2026-09-07) — the approver
> sees the truth. BUILT IN PARALLEL from a v1.28.63 cut, then REBASED onto
> the v1.28.65 main once Blackout+Meridian shipped (floor/version/changelog
> reconciled in the rebase; ship order .64 → .65 → .66 held). All four
> Lies-in-the-Loop findings closed (audit §4.6): (1) X-L1 (HIGH, fork):
> plugin approvals carry `args` — the EFFECTIVE tool-call arguments (base ⊕
> approval overrides) as display JSON, persistence-redacted, capped 2000
> chars with a VISIBLE exact-count marker — on BOTH transports (embedded
> broker + gateway), computed once (`buildApprovalArgs`) so both surfaces
> see identical truth; gateway sanitizes + re-caps at its boundary (the
> `detail` discipline); TypeBox closed-object schema registers the field,
> Swift model regenerated in-commit (Kotlin doesn't model these params);
> the plugin's title/description STAY (prose claim AND raw act; OWASP MCP
> Cheat Sheet §4 now true at this surface). (2) X-L2 (fork, shipped AFTER
> Meridian's host half landed — the dependency rule, dependency HELD):
> truncation keeps head AND tail UNCONDITIONALLY (the keyword-gated
> "important tail" heuristic is GONE), last 400 chars always ride, middle
> marker carries the EXACT count (`[... N chars elided between head and
> tail ...]`); the tail reservation is bounded to half the budget minus
> the marker's widest form so tight budgets shrink the tail, never fall
> back to head-only; aggregate elision marker is COUNT-FIRST (crushed
> budgets cost the rerun guidance before the count); a notice larger than
> the result it replaces is skipped (net increase — the honest refusal);
> 16k cap + budget discipline untouched. The audit's shaping scenario is
> THE fixture: 100k, injection at 500, disclaimer at 99k — disclaimer
> survives, injection visible. (3) X-L3 (CLI): `brain client dsar`
> requires `--action` (closed vocab purge|export|both from the server's
> OBSERVED truth, not the plan's `rectify`); purge/both prompt with the
> subject digest (`sha256:<12-hex>`) + resolved domain (fail-loud) +
> irreversibility line; `--yes` is the automation seam; CLI BREAK:
> scripted purge adds `--action purge --yes`. (4) X-L5 (CLI): restore
> ALWAYS prompts unless `--yes`; `--force` skips the liveness PROBE only;
> prompt shows resolved ABSOLUTE target + size + audit chain head;
> passphrase files must be 0600 (`check_secret_file_mode` extracted +
> shared with the rotator); CLI BREAK: scripted restore adds `--yes`.
> Tests: 9 CLI pins (red-first) + 5 fork approval tests + 4 truncation
> pins (incl. exact-count arithmetic + the shaping fixture + source drift
> locks for the already-counted compact/default notices); the FULL
> embedded-agent lane re-run green after M2 (1,771 tests / 85 files);
> CRATE_TEST_FLOOR 1,281 → 1,290 at the rebase (Blackout's 1,281 + 9).
> main.rs untouched; no routes/schema/wire change (openapi + x-api-version
> unchanged). Fork-side: the typebox 1.3.15-lock/1.3.18-manifest drift is
> repaired IN COMMIT (main's working tree still carries it uncommitted —
> land or absorb). Manual ceilings: the approval-surface screenshot is
> pending a human run; `args` renders as a plain field (monospace block =
> cosmetic follow-up). See CHANGELOG §[1.28.66].
> Predecessor: **v1.28.65 "Meridian"** (2026-09-07) — content hygiene
> across the model seam, THREE TREES the same day. Closes the 2026-09-06
> audit's content-door findings: X-R1, X-R5, X-S1 (HIGH), X-M2 (HIGH).
> Theme: nothing enters model context unstripped and unlabeled, regardless
> of which door it used — four doors, four small structural layers.
> (1) X-R1 (brain): `/suggest` hits carry `untrusted: true` (recall/search
> parity — the one content-returning surface breaking the consumer
> contract); openapi additive; pins `suggest_hits_carry_untrusted_true` +
> the three-surface parity source pin; CRATE_TEST_FLOOR 1,267 → 1,269
> (Blackout's .64 raise to 1,281 lands with ITS release commit — floor
> counts are per-commit monotone, both truths hold at HEAD). (2) X-R5
> (plugin 0.5.1): `sanitizeForBlock`'s invisible set is now EXACTLY the
> Rust canonical set, exported as `INVISIBLE_CLASSES`; the parity fixture
> (`plugin_invisible_set_matches_rust_canonical`, one probe per Rust-set
> class) is THE drift pin — either tree changing alone fails CI; the
> plan's five-member add list was itself short of parity (missed
> U+FE00–FE0F, U+2060–2063, U+00AD, U+034F) — the fixture forced the full
> set. (3) X-S1 (openclaw fork): the ONE merge seam strips + neutralizes —
> every plugin context segment passes `sanitizePluginContextSegment` (new
> `src/plugins/context-hygiene.ts`) inside `mergeBeforePromptBuild` (the
> convergence point BOTH runners ride): canonical invisible strip +
> ZWSP-split of forged `⟦openclaw:ctx⟧` markers and the built-in
> `<active_memory_plugin>` fence tags (strip runs FIRST → idempotent; the
> JOINED accumulator is sanitized → no cross-segment synthesis; the
> built-in's own emitted tags split too — uniform, no per-plugin logic);
> the brain plugin's `UNTRUSTED_BEGIN/END` fence survives BYTE-IDENTICAL
> (pinned). The fork's `stripInvisibleUnicode` widened to the canonical
> set (bidi isolates U+2066–2069 + ALM were missing — the same drift
> class, closed host-side). (4) X-M2 (openclaw fork): MCP tool results
> ride the external-content idiom — text blocks invisible-stripped, the
> joined result enveloped ONCE (never per block) via the extracted
> `createExternalContentEnvelopeSegments` (`wrapExternalContent` composed
> on it, byte-identical); `Source: MCP Tool Result` label; the
> `untrustedMcpOutput` flag finally renders as prompt framing; the
> host-authored empty placeholder stays unwrapped; forged wrapper
> boundaries neutralized by the existing sanitizer. THE LINE'S FIRST LIVE
> END-TO-END PROOF (GREEN): `docs/MERIDIAN_PROOF_20260907.md` — a memory
> carrying the U+E0000 tag block + forged host markers, recalled through
> the real plugin fence + host merge + CLI composition on a TEST server
> (fresh DB, test port, copies-only): all forgeries absent from the
> composed prompt. Ledger: Blackout (1.28.64) SHIPPED FIRST off the same
> main push that carried the Meridian fix commits (0b66d3b, 03819bf);
> keep-a-changelog order .65-above-.64 is the honest history. Gates: full
> suite 1,360 passed / 7 ignored; clippy bench/default/otel clean; fmt
> clean; engine-crates + steward-harness green; lipstyk green (one
> comment-only condensation 4e6c477 rode the pre-push gate — density
> finding on plugin/src/format.ts, zero behavior change); badges
> selfcheck clean; the known connector-stub race fired once under the
> parallel sessions' load (passed on rerun, pre-existing test-infra).
> Ceilings: X-R2/X-R3 stand (consumers fence; HTML strip is .72); no
> taint lattice (X-S2 → .74 Origin); truncation-after-wrap can elide the
> envelope end marker until Truthglass (.66); MCP schema pinning is .67;
> no server-side fence envelope on HTTP JSON; the fork's `pnpm-lock.yaml`
> typebox bump predates this line. openapi additive only; x-api-version
> UNCHANGED; schema untouched; no new deps in any tree.
> See CHANGELOG §[1.28.65].

> Predecessor: **v1.28.63 "Wardline"** (2026-09-06) — the SEAM LINE opens:
> reserved vocabulary at the workflow input seam (`RESERVED_OUTBOX_TOPICS`
> at `enqueue_child`, closed run statuses, the valet fence function-held,
> alert-bus kind auth), closing the only code-false security law in the
> repo's history (X-W1..X-W5). Full note retired to
> `docs/AGENTS_HISTORY.md`; see CHANGELOG §[1.28.63].
> Predecessor: **v1.28.62 "Attestation"** (2026-09-06) — the Enterprise
> Line CLOSES. Four milestones, all additive (no breaking wire change, no new
> crypto primitive, no C2PA claim anywhere). (1) PROVENANCE MARKS
> (`src/provenance.rs`, the Art 50(2) posture): engine-generated TEXT
> artifacts carry `{"provenance": {mark: AIGEN|HUMAN, generator,
> generated_at, signed_by, sig}}` — Ed25519 via `sign_manifest_bytes`, but
> the SIGNED MESSAGE is the claim-bound wrapper `{artifact, claim}` (the
> body-only design let a flipped mark verify; the pin caught it in
> development). The four classes ride their REAL emission shapes: remedy
> drafts (`handlers::workflow::remedy_response`), ADR packets +
> outreach exports (`seal_adr_packet` / `seal_campaign_packet` — read-seam
> shaping THEN the seal, the mark signs boundary bytes), KB manifests
> (`kb::sealed_manifest_json` — the pure digest rule byte-unchanged). No
> operator key ⇒ mark present but visibly unsigned (verify refuses). Pins
> `provenance_marks_present_on_all_four_classes` + `tampered_provenance_
> fails_verify`; reg_watch ai_act WATCH → DELIVERABLE (horizon 2026-12-02
> stays stamped). openapi additive; x-api-version UNCHANGED. (2) THE
> PRINCIPAL KILL-SWITCH (ASI03/07): `revoked_principals` (schema → 1.28.62);
> `verify_card` checks revocation BEFORE signature work AND before the row
> lookup (probe-blind); dispatch/result re-check at decision time; re-
> provisioning does NOT resurrect. The drain: revoke upsert + hash-chained
> auth audit row + in-flight-run cancel via the EXISTING cas_update path
> (`status 'cancelled'`) + `delegation/revoked` lineage events, all in the
> caller's tx. Routes `POST /ops/agents/revoke` (Admin on global —
> identity-wide) + `GET /ops/agents/revocations`; openapi + both guard
> tables + api.md in-commit. Pins `revoked_principal_cards_fail_closed` +
> `revoked_owner_no_new_dispatch`; the authz matrix carries the route's body
> template. NOT JWT revocation (that is auth/revocation.rs — separate
> layer). (3) APPROVAL-FATIGUE TELEMETRY (ASI09): the client detector's
> arithmetic server-side — `scoreboard::approval_uniformity`, verdict
> expression the client's f64 form VERBATIM (0.9 boundary identical), ratio
> = integer ten-thousandths; the data fn mirrors the client's fetch (7d on
> created_at, latest 200/status, decided-only); scoreboard JSON gains
> `review_independence_risk` + `approval_uniformity_ratio` +
> `review_decisions_window`; role gate UNCHANGED (DPO/admin). Dictionary
> twins (metrics.md ASI09 section + metrics.json) same-commit; parity pin
> `scoreboard_uniformity_matches_client_math`. (4) CRYPTO INVENTORY
> (`docs/crypto-inventory.md`, SP 1800-38B shape): every shipped algorithm ×
> HNDL verdict × swap path; the JWT ML-DSA landing procedure against the
> real `ALLOWED_ALGS` seam; the UMP did:key multicodec version-prefix rule.
> NO PQC deployed — classical signatures are the printed ceiling (JWT waits
> on the IdP). reg_watch pqc WATCH → DELIVERABLE (2030-12-31 stamped); the
> watch clock keeps its own self-test. SOC 2 proof-map refreshed; brain-fuzz
> corpus rides a nightly CI schedule (corpus replay + harness-kernel +
> libfuzzer compile check; caught a latent libfuzzer-feature warning).
> CRATE_TEST_FLOOR 1,244 → 1,256. DRILL 2026-09-06 vs a COPY of the live
> 50.6 MB db: kill-switch end-to-end (cards 403, dispatch 403, owner
> revoked → `runs_drained:1`, run cancelled CAS 0→1, lineage event,
> `/audit/verify` ok); ADR packet + kb manifest carried signed AIGEN marks
> (digests unmoved); 21 digest-bound approvals flipped the scoreboard to
> risk 1 / ratio 10000. Runbook section + dated record + reg_watch
> `revocation_drill_recorded`. main.rs untouched (net delta 0); wire/schema
> additive only. See CHANGELOG §[1.28.62].
> Predecessor: **v1.28.61 "Standby"** (2026-09-06) —
> WARM standby from shipped mechanisms — encrypted base + WAL chunks + a
> rehearsed promote; NO hot failover claims anywhere. Library
> `src/standby.rs`, consumed ONLY by the `brain standby` CLI subcommands
> (operator-run process; a shipper inside the server it protects is a
> correlated failure). ship_cycle order is LOAD-BEARING (commented): PASSIVE
> checkpoint → base.v3 via the SHIPPED backup v3 writer → wal/NNNN
> frame-chunk copied AFTER the base (the writer TRUNCATEs the wal — an
> earlier copy would replay pre-base frames and roll the restore back) →
> manifest signed via the shared `ump_integrity::sign_manifest_bytes`
> (Ed25519 over the hex-SHA-256 STRING — the parcels convention; parcels
> refactored onto it byte-identically, `parcel_signature_bytes_unchanged`)
> and written LAST. Chunks ride `backup::encrypt_v3_blob` — NO unencrypted
> byte at rest on the follower. `verify_follower` = signature over exact
> bytes + recomputed artifact hashes, fail closed; `promote_check` reuses
> the shipped restore path, registers sqlite-vec first (vec0 tables),
> integrity_check, timed RTO, RPO = interval + measured checkpoint lag
> (`promote_check_rpo_math`; the follower-side twin of the .58
> brain_wal_pages_pending gauge). Pins incl. `manifest_sig_verified_after_
> copy`, `follower_tamper_detected` (one flipped chunk byte → status exit 1),
> the roundtrip proptest, `cli_reference_covers_subcommands` (the
> cli-reference law — closed four pre-existing doc gaps: parcel,
> wfm-import, valet, ropa), reg_watch `standby_drill_recorded` (dated
> record with measured timings, CRA-drill precedent). CRATE_TEST_FLOOR
> 1,228 → 1,244. DRILL 2026-09-06 vs a COPY of the live 48.8 MB db: RTO
> 0.55s, RPO 10.4s, 9,091-row fidelity, tamper fail-closed. DISCLOSED: the
> .bak safety mechanism proved itself live — a development-phase
> `restore --force` mis-aimed at the live db (target is BRAIN_DB_PATH/
> default, not the positional); the automatic snapshot carried the memory
> back; lesson encoded in the runbook promote procedure. The CodeQL
> security closures (7 alerts: path-injection ×3 storage_layout traversal
> refusals, log-injection ×1 sanitize_log_value seam, cleartext-logging ×3
> DSAR cert out of assert messages) landed in the same line — full record
> in CHANGELOG §[1.28.61]. main.rs untouched (net delta 0); wire/schema
> unchanged. See CHANGELOG §[1.28.61].
> Predecessor: **v1.28.60 "Loom"** (2026-09-06) —
> CPU parallelism as an OPT-IN tier: feature `loom = ["dep:rayon"]`
> (rayon 1.12.0; the ONLY new dep this line allows) × capacity target !=
> jetson × `BRAIN_LOOM=1` — fail-closed parse; pool capped `min(cores-1,4)`;
> `/health/db` echoes all four states. EXACTLY TWO fan-out sites
> (batch-ingest embed pre-pass; consolidate near-dup pure-CPU preprocessing
> — the KNN loop STAYS SERIAL on the shared `&Connection`). THE INVARIANT:
> no cross-chunk reduction — adding one breaks `loom_preserves_fused_ranks`.
> Seven pins (CRATE_TEST_FLOOR 1,221 → 1,228). Live proof
> docs/LOOM_PROOF_20260906.md + BENCHMARKS §v1.28.60: byte-identical vec
> index (sha256) across loom/serial postures; eval floors identical to 3
> decimals; HONEST: the static potion tier is too cheap for the fan-out to
> pay — the value case is the CPU-bound neural profile, unmeasured.
> See CHANGELOG §[1.28.60].
> Predecessor: v1.28.59 "Headroom" (2026-09-05) —
> A documentation-first release wearing a test harness; nothing behavioral
> changes on any default target (the envelope-defaults pin enforces). (1)
> DURABILITY POLICY: `CapacityEnvelope` gains `synchronous_mode` +
> `wal_autocheckpoint_pages` with defaults == the MEASURED pre-change
> behavior (empirical readback: a fresh pooled connection ran
> synchronous=FULL / autocheckpoint=1000 — the compile defaults; the
> migration connection's NORMAL never covered the pool). Applied beside
> `busy_timeout` at every pooled connection's init via the new named fn
> `main_pool_connection_init` (the inline closure is gone; boot file
> shrinks); env overrides `BRAIN_SYNCHRONOUS` | `BRAIN_WAL_AUTOCHECKPOINT`
> fail closed (unknown value refuses boot, the WRITE_POSTURE pattern;
> autocheckpoint 0 = off is refused); `/health/db` gains the static
> boot-time `durability` echo (additive). Pins:
> `envelope_defaults_equal_current_behavior` + `pool_init_pragmas_read_back`
> + the resolver bounds pins. (2) LOCK-WAIT TELEMETRY: every production
> `Mutex`/`RwLock` site (19 fields — 2 found beyond the plan's list) carries
> a Lock-bounds comment; 15 request-path holders record CONTENDED-only
> acquire waits (try_lock fast path = zero clock reads) into the new fixed-
> edge `LockWaitHistogram`; `/metrics` gains `brain_lock_wait_micros_p50|
p95` (bucket-quantile edges, no histograms crate); poison postures
> preserved per-site via the five measured-lock helpers. Named pins:
> limiter purity, token-swap single-assignment (>1k reads, >50 real
> rotations, zero torn), helper contention/poison contracts. (3) THE
> WRITE-DISCIPLINE RATCHET (`tests/write_discipline.rs`): the plan's
> "allowlist empty on arrival" was EMPIRICALLY FALSE (handlers construct
> transactions and delegate statements to service cores — the no-SQL gate
> counts statements, not BEGINs; plus sanctioned seams) — shipped instead
> as the Plumb debt-lock: DEFERRED inventory frozen at 38 sites / 21 files
> (down-only, unlisted hits fail with file:line), IMMEDIATE floored at 20
> (up-only), RED-PROOFED against a planted violation. (4) LIVE PROOF:
> docs/HEADROOM_PROOF_20260905.md + the BENCHMARKS.md §v1.28.59 table —
> WAL trajectory flat 0 in both cells (the 6000-doc burst showed the one
> mechanistic delta: 34-page transient under full/1000 vs flat 0 under
> 256); p95 24.52 → 24.19 ms (noise); lock-wait gauges' first live reading
> ≤10 µs; durability echo verified in both postures. CRATE_TEST_FLOOR
> 1,207 → 1,221. Pre-existing rerank-tier build break fixed in passing
> (`search::` → `crate::search::` in bootstrap). Ceilings (honest): the
> ratchet is NOT the plan's zero-allowlist; the split idiom's mid-file
> blind spot is shared with every house gate; quantiles are bucket edges;
> Jetson durability unmeasured; lock-wait coverage excludes the mcp
> binary, the connector token cache, and scrape-path locks (reasons
> inline). See CHANGELOG.md §[1.28.59].
> Predecessor: v1.28.58 "Throughput" — the Enterprise Line opens: concurrent truth
> (BENCH_CLIENTS fan-out, same-seed determinism), visible contention (pool-timeout,
> busy-error, WAL-pending gauges + the /health/db concurrency echo), the calendar
> as code (CRA 2026-09-11, AI Act, PQC seams) + the CRA reporting runbook + drill.
> Full note: CHANGELOG.md §[1.28.58]; retired detail: docs/AGENTS_HISTORY.md.
> **Version note:** **v1.28.54 "Scaffold" — THE SPIRE LINE OPENS.** The
> monolith dismantling starts with the risk-zero third: measure-and-freeze.
> Scope 1: `src/spire_inventory.rs` (cfg(test), sibling of the retired
> `sql_inventory_baseline` idiom) — the frozen ledger over main.rs:
> ceilings `MAIN_RS_LINES`, `TEST_REGION_LINES`, `ROUTE_CALL_SITES` (down
> only, in the commit that earns the shrink); floors `MAIN_RS_TEST`,
> crate-wide `#[test]` total, guard-table rows 151/141 (up only). Shipped
> red-then-green: commit 1 asserts deliberately tight wrong ceilings and
> fails loudly on all three; commit 2 sets the measured session-start
> truth (19,906 lines / region from L6,565 / 234 route sites / 139 pins —
> the roadmap's v1.28.35-era numbers were stale; re-measured per the
> executor stop-rule). Scope 2: the route-coverage table (151 paths) +
> route-authz table (141 gates) are data in `src/route_guards.rs` —
> declared from main.rs, NOT handlers/mod.rs, because docs_truth skips
> files included by `#[cfg(test)] mod X;` in their declaring parent and a
> handlers-side decl would have exempted the tables from the comment
> guard. Tests consume the consts; verdicts identical; row counts floored
> in the ledger. Scope 3: ten pure-unit pins relocated verbatim to their
> subjects — 6 into `handlers/mod.rs` (authorize ×3, audit_scope ×2,
> typed-edge), then auth_tokens→config, temporal→temporal,
> trace_caps→trace, eval_metrics→eval. Every router/DB/main.rs-subject
> suite stays (screen family waits for its subject's M2 promotion; the
> `test_db()` mass moves at the family/lib-flip milestones). The floor
> discipline bit once in development: relocating a family without the
> same-commit ledger edit failed exactly as designed. Landed truth:
> 19,906 → 19,282 lines; region 13,342 → 12,712; route ceiling frozen at
> 234 (routes move in Vaulting, not Scaffold). Full suite 1,314 passed /
> 7 ignored per commit; clippy -D warnings clean; lipstyk diff-strict
> green; wire artifacts diff-empty (openapi.yaml, route-coverage,
> route-authz, x-api-version); /health smoke green. Ceilings (honest): no
> chunker/capacity pure pins existed in main.rs to relocate; no behavior
> change of any kind — the whole release is proving that with gates.
> See CHANGELOG.md §[1.28.54]. Predecessor: v1.28.53 "Triage" — the
> review queue is domain-scoped for real (full note in CHANGELOG
> §[1.28.53]; no AGENTS.md row was cut for it).

> **Version note (retained predecessor):** **v1.28.52 "Cornerstone" — THE
> FIN.** The Foundation Line is complete and machine-enforced. AMENDMENT
> first: v1.28.51
> shipped with gate.rs (78) still holding SQL, so the milestone opened
> with the AGENTS.md-prescribed Masonry-class extraction of the final
> vein — a new `service::gate` core, six surfaces, six commits, full
> gate + baseline-row-lowered per commit (78 → 68 → 66 → 59 → 57 → 21 →
> 0: the review-queue read, the creation insert + conflict pre-check,
> the expire/reject family, the edit path, the approve family with the
> decision CAS one-defined across six branches and the article state
> CAS typed so `public_slug_taken` keeps its 409, and the export
> bundle). Then scope 1: the enforcing flip — `SQL_BASELINE`, the floor
> pin, and the allowlist machinery DELETED; `no_sql_in_handlers_enforced`
> walks `src/handlers/` recursively and fails on ANY statement match
> (production, test, or comment residue), with a ≥30-file sanity against
> the vacuous pass and a counter self-pin (`sql_statement_counter_still_fires`).
> Scope 2: the transport-free grep takes its line-plan name
> `service_layer_free_of_http_types` (born a hard error at Plumb — there
> was never a warning phase); both guards ride CI via the test jobs.
> Scope 3: `docs/architecture.md` gains the two layer rules, the
> request-flow diagram, and the seam table; this file's Architecture Law
> points at it. Scope 4: the Foundation Line report appended to
> `docs/AUDIT.md` (pin counts, eval floor, smoke matrix, wire + schema
> identity). Quirk preserved verbatim + filed: the translation CAS's
> `decided_at = datetime('now')` (SQL-side clock) needs a pin or fix.
> Full suite 1306 → 1308 passed / 7 ignored; clippy green (bench); the
> compliance-pack test run from the Confluence note still owed before
> push. See CHANGELOG.md §[1.28.52]. Predecessor: v1.28.51

> **Version note:** **v1.28.51 "Confluence" shipped 2026-09-02** — the
> long tail: every handler file EXCEPT gate.rs drains to ZERO embedded
> SQL; the inventory floor moves 241 → 78 with the single remaining row
> `("gate.rs", 78)`. Sixteen files across 15 extraction commits, one
> commit per file in roadmap order, full gate per commit, baseline row
> lowered in the same commit as each move. New/extended cores:
> service::procedure (store tx: root → per-chunk quarantine flags →
> steps → next_step edges skipped on a quarantined root; step-chain/
> meta/decision reads; best-effort vec-shadow writes), service::ump_ops
> (urn lookup, supersession read, raw relations, soft-forget block
> flag+hash-only-tombstone+in-tx-audit, consent-denial audit helper
> moved WITH its pin), service::forget (single-chunk erasure; tombstone
> only when a row actually deleted), service::suggest (last-wins
> feedback upsert, fail-open existence fence, retyped off HandlerError;
> grouped outcome counts), service::compliance (best-effort oversight
> write, evidence counts unwrap_or(-1), legacy-JSON RoPA read, RoPA
> upsert with in-tx audit), service::art30 (register reads: fail-the-
> request categories, best-effort connector/DSAR, fail-open lifecycle),
> service::webhook_ingest (kb-feedback flood/finding/hot-count, Signal
> flood bound, draft-approve read + digest-gated UPDATE), workflow side:
> state run-row reads + open_run, outbox steering inbox + lineage
> reads, scoreboard.rs NEW (runs page, fail-closed hash linkage,
> aftersales cohort, score_units_now + its test module), kcs article
> lifecycle, valet brief projections, relay handover reads, crew
> touch + skills proposal, channels user-map proposal + shared
> seen-window flood count; plus role::defined_count,
> capacity::knowledge_docs (fail-open),
> legal_hold::first_missing_id, service::recall::chunk_for_verify.
> The twelve borrowed fence pins in clients.rs moved onto
> service::register's test module (fixtures already there from
> Terrace); the valet brief tests moved onto workflow::valet's test
> module; the scoreboard tests moved with their fns. CAS discipline,
> verify-before-serve UMP orchestration, digest gates, probe-blind 404
> families: byte-identical, pinned through every move. Body-scan +
> authz + read-seam guards passed with no additions. Well_known.rs
> verified 0-SQL (the drained-file template). Dup-guard + transport-
> free greps green. Full suite 1306 → 1316 passed / 7 ignored; clippy
> green on bench/default/otel/compliance-pack (clippy only); CI
> dry-run green (crates, steward-harness, default-features); lipstyk
> diff-strict clean (one string-params finding fixed);
> openapi.yaml diff-empty; schema untouched at 1.28.45.
> CEILINGS (honest): **the allowlist did not reach empty — gate.rs
> (78: 50 prod + 28 test, the HITL proposal engine) was the one
> straggler and blocked the v1.28.52 enforcing flip; resolved by the
> Cornerstone amendment (the final-vein extraction ran inside
> v1.28.52, before the flip)**.
> The compliance-pack TEST RUN is deferred (clippy green; three
> interrupted attempts — one-time full rebuild); run before push. The
> compliance-pack's own pins were made compilable (full 9-column
> evidence fixture; the decision test seam is
> cfg(any(test, feature = "compliance-pack"))). Known-flaky backup
> tests raced twice (console-seam test sets BRAIN_CONNECTOR_CONFIG_DIR
> without the env-lock; pre-existing test-infra, untouched). Filed
> follow-ups: forget writes no audit_events row (tombstone-only
> evidence); the Signal digest-mismatch Denied audit still rolls back
> with its tx; put_run_state's 200-body revision quirk (run-id-as-
> revision) preserved verbatim, needs a pin or fix. See
> CHANGELOG.md §[1.28.51].

> **Version note:** **v1.28.46 "Plumb" shipped 2026-08-28** — the Foundation
> Line opens: the release that ships the LINE'S MACHINERY and the first
> vein, zero features/endpoints/schema/wire changes by design. The debt lock
> (`sql_inventory_baseline_freezes_the_debt` in `src/service/mod.rs`)
> freezes the per-file SQL-statement inventory of `src/handlers/*.rs` at the
> measured numbers (445 across 29 files; case-insensitive non-overlapping
> occurrences of `SELECT `/`INSERT `/`UPDATE `/`DELETE FROM`): any file
> above its frozen count, or SQL in an unlisted file, fails CI; below-
> baseline progress prints deltas — the lock stops regrowth, it does NOT
> force pace (the enforcing flip is the line's LAST milestone). The service
> layer (`src/service/`) opens as the convergence target: the layer contract
> as docs + greps-as-tests (services take connections, never pools/state/
> transport types; own SQL + bounds + FK-children + audit-per-write inside
> the caller's tx; typed errors, handler-side HTTP mapping). First
> extraction exemplar: the govern retention family → `src/service/
> retention.rs` (override upsert + report queries + evidence audit inside
> ONE WorkflowTx — closing the unevidenced-write window where the audit
> rode a second pooled connection after the commit; the upsert set is now
> atomic). `govern.rs` 18 → 6 embedded statements (−12 incl. moved tests);
> pins 993 → 1000 (+7: the lock, its floor pin, the transport-free grep,
> the byte-for-byte legacy fixture captured pre-move, the in-tx audit law
> + rollback twin, the fence pin, and the report pin moved verbatim).
> Wire artifacts byte-identical; schema untouched at 1.28.45. Ceilings:
> report rows stay legacy JSON maps (byte-for-byte pin outranks the
> domain-type aspiration); the baseline counts comments + test seeds
> (substring lock, not a precision instrument); kind charset validation
> stays handler-side (handler-typed) with the core fence covering
> bounds + emptiness only. See CHANGELOG.md §[1.28.46].
> Predecessor: v1.28.45 "Herald" — Slack + Teams as the console's annexes

> **Version note:** **v1.28.45 "Herald" shipped 2026-08-27** — Slack +
> Teams as the console's OPERATOR ANNEXES: the bridge (one binary, two new
> adapters, config-off default) dials Slack Socket Mode with NO inbound
> listener (source-grep pinned) and serves Teams via Bot Framework +
> Adaptive Cards with BF-JWT verify before parse; mapped-channel messages
> become screened threaded notes; pending proposals render as Blocks/Cards
> with the DIGEST in the block and approve/reject actions that MUST carry
> it (bridge-side refuse-and-log, then the kernel re-verifies inside the
> byte-identical approve verb — two independent enforcement points); ONE
> new kernel seam `POST /webhooks/channel/{kind}/console` (pending/decide/
> due/crank, HMAC, closed vocabulary) relays the console in; the Slack user
> map is a proposal-maintained table (`channel_user_map`, schema 1.28.45;
> `/workflow/channel/user-map` files the proposal, approval is the ONLY
> writer; opaque platform ids, roles resolved at file+apply, audited per
> change); Relay handover offers enqueue `channel/ping` rows the drain
> delivers in-channel with the I-PASS completeness state (refs only,
> unmapped = audited loud + consumed); mapped operator activity feeds Crew
> presence as the new `channel` activity KIND only, DPO-switch governed at
> the write. Server-diff verification: the plan expected zero server diff;
> the no-brain-token law + bearer-mode 401s made an additive seam NECESSARY
> (ledger row in CHANGELOG §[1.28.45]); approve/reject handler machinery is
> REUSED unchanged. Routes additive: `/webhooks/channel/{kind}/console`,
> `/workflow/channel/user-map`. Ceilings: replayed decides return the
> console's 404 (the `{moved:false}` receipt stays channel/template-
> specific); slash approve acts only on proposals the bridge rendered this
> session; `/brain due` lists, never fires; Teams drain delivery uses the
> standard regional BF host (no per-activity serviceUrl echo on the drain);
> no channel-side handover accept/decline. See CHANGELOG.md §[1.28.45].
> Predecessor: v1.28.44 "Caravel" — WhatsApp for Business as a governed edge

> **Version note:** **v1.28.44 "Caravel" shipped 2026-08-27** — WhatsApp for
> Business as a GOVERNED EDGE: `tools/channel-bridge` (standalone Rust crate,
> config-off default) owns the public webhook surface — answers the
> `hub.challenge` handshake ITSELF (never the kernel), verifies every POST's
> `X-Hub-Signature-256` raw-body constant-time + length-checked BEFORE any
> parse, then forwards verified envelopes over the Switchboard HMAC seam.
> The 24-hour window binds `reply_window_allows` exactly: outside it ONLY
> approved `channel/template` acts WITH standing consent pass (free-form
> refused even when approved+consented); template sends are digest-bound
> HITL proposals dispatched in the approval tx (replay-safe `{moved:false}`);
> business-initiated = template + consent + approved proposal ALL THREE,
> cold contacts auto-open their care case with the window CLOSED; status
> receipts land as `case/channel_status` lineage events (refs never bodies);
> quality tiers throttle deterministically with FRESH=most-restrictive and
> metadata-only downgrade alerts; attachment SHA-256s ride ON the note while
> bytes stay quarantined edge-side. Schema UNCHANGED at 1.28.44; no routes.
> Ceilings: TLS terminates at the operator proxy; tier taxonomy pinned per
> graph_api_version at deploy; parameterless templates only; kernel does not
> pace by tier. See CHANGELOG.md §[1.28.44].
> Predecessor: v1.28.43 "Switchboard" — the channel seams, Signal first-class

> **Version note:** **v1.28.43 "Switchboard" shipped 2026-08-27** — the
> channel bridge framework: `/webhooks/channel/{kind}` + `/drain`
> (Standard-Webhooks HMAC, replay-capped on bridge+external_id),
> `channel_threads` tenant-scoped thread map (schema 1.28.43), `channel/out`
> outbox topic gated by `enqueue_out` type-fence + reply-window + consent,
> tokenless mount registration with server-recomputed config digests, and
> `tools/signal-gateway` promoted to the first-class Signal edge.
> See CHANGELOG.md §[1.28.43].
> Predecessor: v1.28.42 "Valet" — the personal AI assistant, dogfooded
> (full note retired to `docs/AGENTS_HISTORY.md`)

> **Version note:** **v1.28.41 "Terrain" shipped 2026-08-26** — G8 + the
> series-exit gate closed, ending the Conformance Line. Tiers are tested
> config: checked-in profiles (`deploy/tiers/t{1..4}.env`), the expanded
> T1–T4 guide in `docs/deployment.md` (per-tier env matrix, sizing, cron
> cadences, upgrade path), a CI tier-smoke matrix job that boots each
> profile end-to-end, and two meta-tests (`guide_and_profiles_never_drift`,
> `tier_profiles_boot_and_pass_smoke`). The CONTACT_CENTER_STANDARDS matrix
> is re-audited green-or-ceiling-marked, pinned by
> `series_exit_gate_checklist_green_or_ceiling_marked`; AUDIT.md carries the
> close-out. Schema UNCHANGED at 1.28.41; no route changes; CI-only matrix
> is independently disableable. Ceilings: no installer wizard; tier smoke
> proves config boots, not sizing; G10 watch item stands.
> See CHANGELOG.md §[1.28.41].
> Predecessor: v1.28.40 "Handshake" — G5+G7, the WFM seam + workload views
> **Version note:** **v1.28.40 "Handshake" shipped 2026-08-26** — G5+G7 of
> the Conformance Line closed: the WFM seam is first-party and versioned
> (`wfm/1`, additive-only, two-way pinned against `docs/wfm-seam.md` via
> `wfm_schema_is_versioned_and_additive_only`; generic `brain wfm-import`
> CSV/JSON adapters; skills import lands as HITL proposals), and workload
> visibility completes the people picture (`GET /ops/workload` lineage-only
> per-principal burden + fatigue signals that alert and never reassign;
> `GET /ops/coverage` joins skills to worktype demand). Schema UNCHANGED at
> 1.28.40; routes additive: `/ops/workload`, `/ops/coverage` (both Read).
> Ceilings: gate-backlog attribution is domain-lineage-only (proposals have
> no domain column); fatigue alerting is view-only (no push channel); no
> forecasting/adherence/reassignment; vendor-specific WFM connectors later.
> See CHANGELOG.md §[1.28.40].
> Predecessor: v1.28.39 "Access" — G3+G4, WCAG 2.2 AA as hard gates
> **Version note:** **v1.28.39 "Access" shipped 2026-08-26** — G3+G4 of the
> Conformance Line closed: the six WCAG 2.2 AA criteria new in 2.2 are
> release-blocking automated gates over the console (`focus_never_obscured_
> by_docks`, `drag_alternatives_exist_for_every_drag`, `target_size_floor_
> 24px_enforced_by_classes`, `help_entry_consistent_across_panels`,
> `no_redundant_entry_in_approval_flow`, plus ACR-honesty and RTL/
> pseudolocale pins), the shell ships ONE consistent help entry (3.2.6),
> the layout uses logical CSS properties so `ar` mirrors fully under
> `dir="rtl"`, and `en-XA` elongation is budgeted at test time via the
> `fluent-pseudo` dev-dependency. Checklist/ACR rows now cite their pinning
> tests. Schema UNCHANGED at 1.28.39; no route changes.
> Ceilings: axe-browser CI gate stays operator-run; exact-match locale
> negotiation (no BCP-47 subtags); manual SR matrix rows still unchecked.
> See CHANGELOG.md §[1.28.39].
> Predecessor: v1.28.38 "Lexicon" — G2, the normative metric dictionary

> **v1.28.37 "Advocate" (2026-08-26)** — G1 closed: the whole ISO 10002
> complaint lifecycle on the shipped machinery (the register IS the audit
> chain — no parallel complaint database). The published complaints policy
> (`knowledge.source='complaint_policy'`) renders as the public
> `how-to-complain.html`, linked from EVERY status-page footer;
> `kb build --with-case-status` refuses loudly without a published policy.
> Acknowledgment is its own audited step (`/workflow/runs/{id}/complaint/ack`)
> with an idempotent overdue sweep on the alert bus
> (`/workflow/complaints/ack-sweep`). The closure confirm-gate is wired into
> the lifecycle itself; safety-relevant complaints hit the GPSR path before
> the complaint keyword; the monthly register extract (dispositions, ack-SLA
> attainment, ADR referrals) rides the SAME audited `calibration/sign` row.
> Schema UNCHANGED at 1.28.37 (additive code only).
> Ceilings: no certification claim; ack sweep is on-demand/cron (no internal
> scheduler); register covers the trailing window at sign time only.
> See CHANGELOG.md §[1.28.37].
> Predecessor: v1.28.36 "Keystone" — the last three Order-of-Care gaps

## Architecture Law (the steering)

- **Two layers, one pattern.** Handlers are protocol adapters ONLY: parse →
  OptPrincipal → `authorize`/`authorize_role` → one `spawn_blocking` →
  service-core call → read-seam shaping → response. ALL SQL, bounds/caps,
  FK-children ordering, and invariants live in domain modules
  (`src/workflow/*`; `src/service/*` from v1.28.46) taking `&Connection` /
  `WorkflowTx` — never Pool, AppState, or axum types.
- **Audit-per-write.** Every mutation emits its hash-chained audit row INSIDE
  the caller's transaction (`record_tenant`, SAVEPOINT-nested): a transition
  and its evidence commit or roll back together.
- **Fail-closed everywhere.** Authz gates, poisoned locks, unreadable config,
  quarantine, legal holds, DPO switches: error paths deny loudly; silence is
  never certified (`let _ =` on writes is forbidden).
- **Read seam unconditional.** Every emitted text field passes
  `sanitize_read`; untrusted content is screened + stripped at WRITE time
  (`screen_content` pattern); LLM-facing payloads are fenced.
- **Bounds law.** Every list surface capped, every input bounded, every cap
  pinned by a test; SQL never decides a row's fate (Rust-side pure arbiter).
- **Wire-contract discipline.** Any route change ships openapi.yaml +
  route-coverage + route-authz guard-table entries in the same commit; the
  `x-api-version` stamp moves only when the wire contract moves.
- **Convergence.** New code is ALWAYS a service core; legacy extraction is
  trigger-based (piggyback / pre-storage-adapter deadline); handler-side SQL
  is ENFORCED ZERO by the `no_sql_in_handlers_enforced` guard (Cornerstone —
  any match in `src/handlers/**` fails CI; there is no allowlist). The law's
  public statement lives in `docs/architecture.md` (the two layer rules, the
  request-flow diagram, the seam table); this file is the operational
  summary.
- **The thin binary (Capstone).** main.rs is WIRING ONLY (bootstrap →
  compose → serve), pinned ≤ 300 lines with no cfg(test) region (the mass
  lives in `tests/`); route registrations live ONLY under
  `src/server/router/**`; `server::bootstrap` stays protocol-free (no axum
  types). Each clause is machine-checked by the spire gates in
  `src/spire_inventory.rs` (`route_registrations_live_only_under_router`,
  `bootstrap_stays_protocol_free`,
  `spire_inventory_freezes_the_thin_binary`); the one fenced exception is
  `src/bin/mcp.rs` — a separate binary's single-endpoint /mcp protocol
  edge, pinned at exactly one site. The line's measured record:
  `docs/AUDIT.md` (the Spire Line close-out).

## Service-layer convergence (Foundation Line, v1.28.46+)

- New code ALWAYS builds as a service core (`src/service/<domain>.rs` owning
  SQL + invariants + audit_write inside the caller's tx); handlers are
  protocol adapters only. Templates: `src/workflow/channel.rs` +
  `src/handlers/channel.rs`, and since Plumb `src/service/retention.rs` +
  `src/handlers/govern.rs` (the extraction exemplar).
- Legacy extraction is TRIGGER-BASED, never cosmetic: (1) piggyback — when a
  feature substantively touches `recall/gate/observe/domains/clients`, extract
  that surface's core in the same release (pins first, move second);
  (2) deadline — immediately before any v2.x storage-adapter work.
  (The historical freeze-and-burn machinery — the `sql_inventory_baseline`
  per-file table shipped in Plumb at 445 across 29 files, lowered line-wide
  by Quarry/Masonry/Terrace/Aqueduct/Confluence to 78 — is RETIRED: the
  Cornerstone extraction took gate.rs 78 → 0 and the enforcing flip deleted
  the table. `no_sql_in_handlers_enforced` in `src/service/mod.rs` now fails
  on ANY statement under `src/handlers/**`, recursively. Line plan:
  `IMPLEMENTATION_ROADMAP_v1.28.46_to_v1.28.52_FOUNDATION_LINE.md`;
  executor: `EXECUTION_PROMPT_v1.28.46_to_v1.28.52_FOUNDATION_LINE.md`.)

## Operational Context (read first)

**Paths**
| What | Where |
|---|---|
| Repo | `/Users/mark/Sites/brain-server` |
| **Live DB** | `~/.openclaw/workspace/brain.db` (override: `BRAIN_DB_PATH` env) |
| Binaries | `~/.local/bin/{brain-server, brain, mcp, bench, brain-connector-stub, brain-migrate-rehearse}` (+ `brain-connector-gh`/`brain-connector-crm` when their features are built) |
| Auth token | `~/.config/brain-server/auth-token` (0600; written by `install-service.sh`) |
| Launchd plist | `~/Library/LaunchAgents/com.brain.server.plist` |
| Logs | `~/Library/Logs/brain-server.{log,err.log}` |
| Config module | `src/config.rs` (all constants + env var resolution) |

**Commands**
```sh
# Build all 4 binaries
 cargo build --release --features bench --bin brain-server --bin brain --bin mcp --bin bench

# Tests + quality gates (always run with --features bench — the bench binary is feature-gated)
# Do NOT hand-type the count. `scripts/badges.sh` derives it and writes the README
# badge; `scripts/badges.sh --verify-count` re-derives and REFUSES on drift. A
# number pasted here is a number nobody diffed against a measurement — the exact
# failure this file's own header documents, repeated. (It happened: the old
# comment here carried a count pinned to a commit twelve releases back.)
cargo test --features bench                                  # count: scripts/badges.sh --verify-count
cargo clippy --all-targets --features bench -- -D warnings   # zero warnings enforced
cargo fmt --check

# CI DRY-RUN — REQUIRED before every push/release (v1.28.29 lesson):
# the local gate alone does not cover the default-feature build or the
# side workspaces CI runs on Ubuntu. Run these before pushing:
export RUSTFLAGS="-D warnings"
cargo clippy --all-targets -- -D warnings                    # job lint-test (default features)
cargo test --all-targets                                     # job lint-test (default features)
cargo test  --manifest-path crates/Cargo.toml --all-targets          # engine-crates
cargo clippy --manifest-path crates/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path tools/steward-harness/Cargo.toml         # steward-harness-gate
cargo clippy --all-targets --features otel -- -D warnings    # otel-gate
cargo test  --all-targets --features otel
# client-gate only when client/ touched (slow: wasm + desktop headers).

# FEATURE LANES — every `[features]` entry has a job in .github/workflows/ci.yml.
# Six did not, and two of those did not compile (`injection-classifier`,
# 9 errors; `neural-embed`, 2 errors), so the breakage accumulated unnoticed.
# A lane that does not exist looks exactly like a lane that passes.
# `tests/feature_lane_pins.rs` derives the feature list from Cargo.toml and
# fails on any feature no lane builds — do NOT delete that pin; it is the only
# thing making the six lanes below durable.
for f in compliance-pack multivec injection-classifier neural-embed loom rerank-tier; do
  cargo clippy --all-targets --features "$f" -- -D warnings
done
# A lane proves a feature COMPILES. It does not prove the feature WORKS — the
# ONNX/model artifacts are not fetched in every lane, and no lane exercises
# layer-2 scoring or a live rerank.

# Repo briefing — ONE-SHOT agent overview before any work (runs <1s):
scripts/repo-brief.sh
# (versions, HEAD, dirty paths, main.rs structure, env/route/CLI counts,
#  which guards exist vs ABSENT, stale-marker probe of living docs)

# v1.28.31 lesson — these two CI jobs are NOT covered by the pre-push hook
# (it fmts only the server tree) and burned the first Charter push. Run them
# locally as part of the gate, ALWAYS before push:
scripts/lipstyk-gate.sh
 cargo fmt --manifest-path client/Cargo.toml -- --check
# lipstyk-gate: the CI watchdog's trap-proof wrapper — the raw
# `lipstyk --diff "$(git rev-parse origin/main)"` form goes VACUOUS once you
# push (origin/main == HEAD → empty diff → exit 0 having scanned nothing) and
# it never sees UNTRACKED new modules (both teeth bit on the Terrace push;
# CI caught what the local run waved through). The wrapper pins the base
# (pre-push merge-base, else HEAD~1, else pass the old tip / release tag),
# marks new files intent-to-add, and REFUSES to pass vacuously. lipstyk:
# changed lines must add no diagnostics (Box<dyn Error> returns in
# tests included — helpers inside #[cfg(test)] mods are still scanned; use
# .expect() like sibling tests instead of Result-returning test bodies).

# CI and releases (2026-10-06 billing law): PRIVATE-repo Actions are
# disabled (the free 2,000 min/month died in six days; CI + CodeQL were the
# burn). PUBLIC-repo Actions are free, so the RELEASE TAG is the public CI
# trigger: a v* tag push runs the FULL ci.yml matrix on public (main is
# still never pushed there — release.sh's law), and release.yml's
# publication step fail-closes unless that matrix is green for the tagged
# SHA: red or absent ⇒ binaries build but nothing publishes. The pre-tag
# discipline is the LOCAL gate suite; release.sh watches the public runs
# after the push and exits non-zero on a not-green verdict. The
# release builds are 4 parallel per-target jobs (linux x86_64/aarch64,
# macOS arm64/x86_64); the verify-required-assets gate refuses to publish if
# any primary brain-server binary is missing.

# README badges — NEVER hand-type them; regenerate from the real build:
scripts/badges.sh                         # prints the version/test/UMP/SBOM badge block
scripts/badges.sh --verify-count          # the REAL count compare (one full cargo test, ~4 min)
scripts/badges.sh --selfcheck             # CHEAP gate: version↔README, UMP, checklist, SBOM
# (derives version from Cargo.toml, test count from cargo test --features bench,migrate)

# Install + restart launchd service (also installs CLI binaries, strips macOS provenance xattr)
scripts/install-service.sh

# Health + stats against running server
brain doctor
brain status
```

**Runtime**
- Server: launchd-managed, `127.0.0.1:8765`, `KeepAlive=true`, `RunAtLoad=true`.
- Auth: bearer token via `AUTH_TOKEN_FILE` → `AUTH_TOKEN` (server) / `BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` → default file (CLI). Off by default if no token resolves.
- macOS Sonoma+: newly-copied executables get `com.apple.provenance` xattr → Gatekeeper SIGKILLs on first exec (exit 137). `install-service.sh` strips it; manual `cp` does not.

**OpenClaw integration**
- brain-server is the **memory backend** for openclaw; the plugin (`brain-server/plugin/`, TypeScript) calls `/recall` each turn via openclaw's `before_prompt_build` hook.
- openclaw config: `~/.openclaw/openclaw.json` (plugin block at key `brain-server`).
- The `brain` CLI is built from this repo's `Cargo.toml` (`[[bin]] name = "brain"`), **not** from openclaw.

**Customer-domain assets live in a private repo (2026-08-19)**
- The case-classification work is **not** part of brain-server: plan, customer
  taxonomy, routing matrix, playbooks, spine builder + outputs moved to a
  **private** repo (not named here — this file is published to the public
  portfolio, and naming the customer is a confidentiality problem
  independent of the assets themselves). Never commit customer-domain content
  here — `.gitignore` covers `spine/` + the two spine scripts defensively.
- This repo stays the vendor-agnostic product: graph-default-on recall, the
  feature-gated `src/classify` engine (when built), proposals/suggest/audit.
  The taxonomy is *data the engine consumes*, owned by the private repo.

**Steward engine IP lives in a private repo (2026-08-21)**
- The advanced architecture docs (Steward architecture, harness audit, state
  mapping, port specs, rubric pin, diagnostics loop, compliance mapping) are
  **MemorySteward LLC IP** and live in the **private** repo `brain-steward-ip`
  (local checkout `~/Sites/brain-steward-ip`). Never commit them here —
  `.gitignore` defends the names defensively (same pattern as the
  customer-domain repo).
  The public repo ships the substrate only; the engine crates (`brain-engine-sdk`
  5,887 top-level + `host/`/`pure/` lines, 8,352 total / 127 tests at v1.28.23,
  plus the five `*-core` engines) are PUBLIC, REAL code — measured 2026-09-15.
  (The old "scaffolds stay public while empty" claim was stale — do not repeat
  it; the SDK is built, kernel-consumed, and dependency-free by design.)
- **The Steward-line implementation plans (1.27.32 → 1.27.42) live in
  `brain-steward-ip/plans/`** with the master `ROADMAP_1.27.x.md`. This repo
  keeps **no** kernel-root plan files: every `IMPLEMENTATION_PLAN_*`,
  `IMPLEMENTATION_ROADMAP_*`, `ROADMAP.md`, `DESIGN_*`, `CLIENT_ROADMAP.md`,
  `REALITY_CHECK.md`, `MARKETING_PLAN.md`, `TODO.md` and `USE_CASES.md` that
  used to sit here was **moved to `brain-steward-ip/plans/archive/` on
  2026-10-04** — see `plans/archive/KERNEL_ROOT_PLANS.md` and
  `KERNEL_ROOT_NONPLAN.md` there for provenance and the list. None was ever
  tracked (`.gitignore` 55–70), so the public tree carried 222 root `.md` files
  while git held 16; the root is now 19, all of them real project docs.
  **Do not re-add one "for context"** — the private archive is the home, and
  `tests/external_claim_pins.rs` discovers the claim-sensitive ones by CONTENT
  rather than by path, so a copy here is not what any gate reads.

---

## Known issues (open)

- ~~**CI gap — tests run on x86_64 runners only.**~~ **CLOSED 2026-09-15
  as NOT-APPLICABLE:** no Jetson/fleet deployment exists (brain-server is
  not installed on any aarch64 host), so the advisory's precondition —
  "before fleet deploys" — is absent. The release matrix still
  cross-builds aarch64 and CI still executes tests on x86_64 only;
  REOPEN at the first aarch64 fleet deployment, and then as an ARM-hosted
  CI test lane, not a manual smoke.
- ~~**Historical token leak (openclaw-side).**~~ **CLOSED 2026-09-09:** the
  final `transcript_events` copy of the agent token was redacted + VACUUMed
  (backup: `~/Downloads/audit/openclaw-agent-pre-purge-*.sqlite`, 0600), the
  agent token was ROTATED (old value dead → 401; watcher reload audited), and
  the discovery of the night: the openclaw gateway's env carried the OPERATOR
  token — the plugin had been authenticating as full superuser. It now runs
  on the AGENT token (`service-env/ai.openclaw.gateway.env`), so the Twokeys
  agent boundary is ENFORCED live (purge → 403 proven). The operator token is
  unchanged. The `${BRAIN_SERVER_AUTH_TOKEN}` placeholder in
  `openclaw.json` resolves from the same env file. Hygiene:
  `~/.config/brain-server/auth-agent-token` (the installer's designated
  `AGENT_TOKEN_FILE` target, unreferenced by the running service) was
  re-synced to the live agent token so a future installer re-run or
  `AGENT_TOKEN_FILE` adoption cannot resurrect the dead value.

History: per-release detail lives in `CHANGELOG.md` §[version] and
`docs/AGENTS_HISTORY.md` (load on demand). Nothing older than the note above
is kept here.
