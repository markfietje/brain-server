# Model governance — the digest-pinned registry and the replay-gated release

**Status:** shipped (1.29.0–1.29.2, with the replay-gated promotion landing in
the unreleased delivery rounds). This page is the doc home the model-identity
line never had: what the model registry pins, what a decision run records, and
what gates a release's promotion — each stated with its refusal codes so an
operator can verify them on the wire.

Sources of truth: `src/workflow/registry.rs` (registry core + execution
resolution), `src/handlers/model_registry.rs` (protocol adapters),
`src/handlers/decision_runs.rs` + `src/handlers/decision_evals.rs`,
`src/workflow/releases.rs` (`promote_release`), and
`src/workflow/create/replay_gate.rs` (the gate itself).

## Why this exists

A decision that a machine executes on its own must be reproducible: the same
run, re-derived from its recorded inputs, must reach the same verdict. That
fails if the model behind the run silently changes, or the configuration
around it drifts, or the trace that justifies the verdict no longer re-derives.
The 1.29.x line closes each hole with a digest pin, and the delivery line
closes the last one with a gate.

## The model registry (`/workflow/model-registry*`)

Three routes (route_guards table: Admin/Write-gated, agent-refused):

| Route | What it does |
|---|---|
| `POST /workflow/model-registry/register` | Register a model identity: `{id, version, kind, …}`. Kinds are a closed vocabulary — `deterministic-rules`, `learned`, `reranker`. |
| `GET /workflow/model-registry` | Bounded listing (1..=50, default 20). |
| `GET /workflow/model-registry/{model_ref}` | One row. `model_ref_invalid` (400) for a malformed ref. |

Pinning rules, each enforced with a named refusal:

- **A learned model MUST carry its artifact digest** (`400 artifact_digest_required`)
  — 64 lowercase hex (`400 artifact_digest_invalid`). An un-pinned learned model
  is not registrable: "the same model" is a digest, not a name.
- **`config_digest`, when present, is also a sha256 pin** (`400 config_digest_invalid`).
- **A `deterministic-rules` document must NOT declare identity** (`400
  registry_identity_declared`) — its identity is derived, not asserted — and a
  declared model MUST carry it (`400 registry_identity_required`).
- **`output_vocabulary` is a non-empty subset of `choice`, `score`, `noul`**
  (`400 registry_vocabulary_invalid`) — the closed consumer set, never free-form.
- **The identity (id, version) is unique** (`409 model_already_registered`).

A registered row is what decision runs cite, by `model_ref`.

## Decision runs and evaluation records (`/workflow/decision-runs*`)

- `POST /workflow/decision-runs` executes a decision against the resolved
  registry row; `GET /workflow/decision-runs/{id}` reads it;
  `POST /workflow/decision-runs/{id}/replay-diff` re-derives the run from its
  recorded inputs and diffs; `GET /workflow/decision-runs` lists (keyset-paginated).
- **Execution resolution is host-side and single:** the run records the
  `model_ref`, `config_digest`, and the citation from what
  `resolve_for_execution` returned — never re-derived from the request, never
  the requested key. A run cannot claim a model it did not run.
- **Digest checks at execute time** (`400 model_digest_mismatch` when the
  stored artifact digest no longer matches the artifact; `400
  config_hash_mismatch` when the config pin moved). A run whose pins do not
  match does not run — it cannot quietly execute on a different artifact and
  record the old name.
- **Exploratory runs are promotion-incapable**: a proposal born from an
  exploratory decision run refuses `400 exploratory_mode_not_promotable` at the
  approval gate — an experiment's output cannot leak into durable state (the
  sanctioned path is re-running the pipeline in deterministic mode).
- **Evaluation records** are DPO/Admin-gated and demand a judgment set
  (`400 judgment_set_unavailable` when none is registered) — evaluation numbers
  always name the judgment set they were scored against.

## The release act, gated: `promote_release`

The delivery loop's release family
(`POST /workflow/delivery/releases/{id}/approve` → `.../promote`,
`POST /workflow/delivery/{kind}/due` for the crank) promotes for real — this
is the live promotion path, distinct from the claim-promote route that ships
inert (see [create-loop.md](./create-loop.md)).

At `promote_release` (`src/workflow/releases.rs`), **after** the
chain-defect precondition and **before** any state change, the
replay-determinism gate runs:

1. `delivery::replay_verify` re-derives the run's stage digests from recorded
   inputs and compares them to the recorded trace.
2. `classify_replay` returns one of three verdicts: `clean`, `divergent`
   (re-derived and recorded digests differ), or `insufficient_evidence`
   (an empty window — never read as clean).
3. A refusing verdict writes a hash-chained `Denied` audit row naming
   `replay_divergent` or `replay_insufficient_evidence`, commits ONLY that
   audit evidence, and returns a denied verdict. **The gate refuses; it never
   repairs, rewrites, or re-derives a "better" trace.**

What the gate buys — and the honest scope: a promotion cannot rest on a trace
that no longer re-derives. It does NOT claim model quality, out-of-sample
accuracy, or false-promotion rates; those remain unmeasured (the same
non-claim posture as the create loop).

An identical trace reaches `allowed` unchanged — the gate detects, it is not
the promotion itself. And the anti-vacuity property is pinned: the red-proof
that removing the gate promotes a divergent trace, and the proof that an
always-refuse gate would be caught, both live in `releases.rs`'s test battery.

## Refusal vocabulary (this page's subject, machine-named)

| Code | Where | Meaning |
|---|---|---|
| `artifact_digest_required` / `artifact_digest_invalid` | register | learned models must pin a sha256 artifact digest |
| `config_digest_invalid` | register / execute | config pin must be sha256 hex |
| `registry_identity_declared` / `registry_identity_required` | register | deterministic-rules must not assert identity; declared models must |
| `registry_vocabulary_invalid` | register | output_vocabulary outside choice/score/noul |
| `registry_kind_invalid` | register | kind outside deterministic-rules/learned/reranker |
| `model_already_registered` (409) | register | (id, version) taken |
| `model_ref_invalid` | lookup | malformed model_ref |
| `model_digest_mismatch` | execute | stored digest ≠ artifact digest |
| `config_hash_mismatch` | execute | config pin moved since registration |
| `judgment_set_unavailable` | eval records | no judgment set registered |
| `exploratory_mode_not_promotable` (400) | approve gate | exploratory-run proposals never promote |
| `replay_divergent` / `replay_insufficient_evidence` | release promote | trace no longer re-derives / nothing to compare (server-namespaced strings — `DenyReason` is a frozen crate enum) |

## What this page does NOT claim

- No out-of-sample false-promotion rate, no detection-quality figure, no owner
  named for such a measurement — the replay gate's own scope statement governs
  (`docs/create-loop.md`'s non-claims carry the reasoning).
- The gate's red-proofs are test-battery proofs, not long-run operational
  statistics.
- Claim promotion (`/workflow/claims/{id}/promote`) remains disabled and
  returns `promotion_disabled` — nothing here changes that.

## See also

- [create-loop.md](./create-loop.md) — the inert claim-promote route and its pins
- [api.md](./api.md) — the route rows for every surface named here
- [metrics.md](./metrics.md) — `brain_model_calls_total{class}` and the model-family telemetry
