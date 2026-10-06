# The secrets ladder — how key material resolves, and why it refuses

> Pinned to crate **v1.29.2**. Read with [Configuration](./configuration.md)
> (every `BRAIN_*` knob and its source) and [Security](./security.md) (the
> transport, authz and erasure posture). This page owns one narrow thing: the
> **resolution order and the refusals** — what happens when a secret is
> missing, wide-mode, or malformed.

There are two distinct mechanisms in the tree, and they are deliberately not
the same thing:

| | Owner | Used for | Posture |
|---|---|---|---|
| **The secret broker** | `src/secrets.rs` | engine-facing key material, resolved by name | `resolve(name)` — file first, inline last |
| **The confined provider reader** | `src/secret_file.rs` | one server-configured provider bearer | `read_provider_secret(root, file)` — confined, shape-validating |

Both share one owner for the reader-side mode check:
`check_secret_permissions` (`src/secret_file.rs`), re-exported through
`src/auth/mod.rs`. The writer-side contract stays in
`scripts/install-service.sh`'s `chmod`. On non-Unix platforms the mode check is
**unchecked** — there are no POSIX modes to read, and this is a disclosed
ceiling, not a silent skip.

## 1. The broker ladder: file, then inline, never a downgrade

`resolve(name)` (`src/secrets.rs:41`) has exactly two rungs:

1. `BRAIN_<NAME>_KEY_FILE` — a path. Read **only after** `check_secret_permissions`
   passes. The file's contents are trimmed and returned.
2. `BRAIN_<NAME>_KEY` — the inline value, trimmed, non-empty only. A last
   resort.

If neither is configured, resolution fails with
`SecretError::NotConfigured(name)`. Callers surface
`AuthStoreUnavailable` / `Internal` — **never an empty secret**.

The names are **derived at runtime**, not hardcoded: the broker builds
`BRAIN_{NAME}_KEY_FILE` and `BRAIN_{NAME}_KEY` by uppercasing the caller's
`name` (`src/secrets.rs:34`). That is why the env-truth gate cannot see these
names by grep — they are `format!`-built at the call site, which is why they
appear in `scripts/env-truth.sh`'s `PINNED_CALLSITES` inventory
(`BRAIN_CASE_STATUS_KEY` / `BRAIN_CASE_STATUS_KEY_FILE`, derive ×2).

**The fail-closed invariant is the point.** A `*_KEY_FILE` that exists but is
group/world-readable refuses resolution outright. It does **not** fall through
to the inline variable and it does **not** fall back to any other source — a
wide mode is treated as an incident, never as a reason to look somewhere
weaker (`src/secrets.rs:4-8`).

### Live callers

Only two consumers resolve through the broker at this version, and both are
deliberate:

* `src/workflow/case_status.rs:97` — `resolve("case_status")`, the HMAC salt
  behind public case-status refs. Without any salt configured, ref minting
  refuses rather than minting from a default.
* `src/workflow/hostcalls.rs:355` — a `resolve(target).is_ok()` **configuredness
  probe**: it reports whether a named secret is available. It does not read,
  return, or log the material.

A missing salt, or an unreadable/wide-mode salt file, surfaces through
`CaseStatusError`'s `From<SecretError>` conversion
(`src/workflow/case_status.rs:58`) — the failure is typed, never swallowed into
an unsigned ref.

## 2. The confined provider reader: shape-validating, root-confined

`read_provider_secret(root, configured_file)` (`src/secret_file.rs:51`) is the
stricter of the two, because it reads a bearer that the server itself points
at. It canonicalizes `root` first, then rejects, in a **closed vocabulary**
(`ProviderSecretError`) that deliberately carries no path or OS error text:

`RootUnavailable`, `OutsideRoot`, `Symlink`, `NotRegular`, `Permission`,
`Unreadable`, `TooLarge`, `Empty`, `Multiline`, `InvalidEncoding`.

Concretely, the target is refused when it is a symlink, resolves outside the
canonical root, is not a regular file, is group/world-readable, is empty,
contains line breaks or control/whitespace characters, or exceeds
`MAX_PROVIDER_SECRET_BYTES` (**16 KiB**). Exactly one trailing `LF` or `CRLF`
is accepted as file framing and is **not** part of the returned value.

Consumers: the delivery connector reads a per-binding bearer
(`src/connector/delivery/mod.rs:420`, surfacing `Secret(ProviderSecretError)`)
and the GDL provider path reads it off a blocking thread
(`src/handlers/case_run.rs:294`).

The env pair is `BRAIN_GDL_PROVIDER_SECRET_FILE` + `BRAIN_GDL_PROVIDER_SECRET_ROOT`
(`src/config.rs:339-340`). This is the GDL provider profile that the
**1.29.0 breaking change** (`POST /workflow/cases/{id}/gdl` now accepts the
bounded `{ticket}` body only) moved off inline request fields — the secret is
server-owned configuration, not per-request caller input. Readiness reports
`gdl_provider: disabled|configured|invalid`, and a partial or invalid profile
refuses bootstrap.

## 3. Operator provisioning

The install path already does the right thing; the ladder's job is to keep a
plaintext value from being *read back* out of a unit or plist.

```bash
# macOS (scripts/install-service.sh) — relocates a plaintext token verbatim
# into a 0600 file and removes it from the plist; directory 0700.
# Linux (deploy/install.sh) — provisions the unit from deploy/systemd/,
# which reads its token from the same 0600 file convention.
```

* **Prefer the file rung.** `*_KEY_FILE` / `*_SECRET_FILE` over the inline
  variable for anything long-lived: the inline form puts the material in the
  process environment, where it is readable by anything that can read the
  environment.
* **Always `chmod 600`** the file, and `chmod 700` its directory. Both
  `0600` and `0400` pass; `0644` refuses.
* **Keep it below the declared root.** For provider secrets, the file must
  resolve inside `BRAIN_GDL_PROVIDER_SECRET_ROOT`.
* **One value, one line.** The confined reader refuses multiline and whitespace
  content outright.

Tier files under `deploy/tiers/` (`t1.env`–`t4.env`) are the shipped shape for
the rest of the `BRAIN_*` surface; see [Deployment](./deployment.md) and
[Deployment filesystem](./deployment-filesystem.md).

## 4. Misconfiguration: what a failure looks like

| Symptom | Cause | Posture |
|---|---|---|
| `auth store unavailable: …` at a resolve site | file exists but is wide-mode, or unreadable | **fail-closed** — no inline fallback |
| `SecretError::NotConfigured` | neither rung set | fail-closed; ref minting / configuredness probe reports false |
| `provider secret is outside the configured root` | path escapes the canonical root | fail-closed, typed |
| `provider secret contains unsafe line content` | multiline/whitespace bearer | fail-closed, typed |
| Readiness `gdl_provider: invalid` | partial GDL provider profile | bootstrap refuses |

In every row the server **denies and audits**; none of them degrades to a
weaker source or an empty value. That is the whole contract.

## 5. Honest limits

* **Non-Unix platforms get no mode check.** `check_secret_permissions` reads
  POSIX modes; where there are none, the permission rung is unenforced. The
  confinement and shape rungs still apply.
* **The inline rung still exists** and is a weaker posture by design — it is
  documented as "last resort" in the source, not deprecated. `env-truth` and
  the config table do not currently steer operators off it.
* **Names are runtime-derived**, so static analysis of `BRAIN_*` cannot see
  them; the pinned-callsite inventory in `scripts/env-truth.sh` is the
  compensating control, and it is a **short, human-maintained list** — a new
  derived secret must be added there by hand.
* **Rotation is a restart-boundary story** for file-path changes: the file is
  read at use time, but which path is in effect comes from the environment at
  process start.
* **`MAX_PROVIDER_SECRET_BYTES` (16 KiB) is a bound, not a policy.** Nothing
  here decides what a *sensible* bearer length is; it only refuses unbounded
  reads.
* This page documents the **reader-side** contract. The writer-side guarantee
  is a `chmod` in an installer script — if you provision secrets by another
  route, you own that half.

## See also

- [Configuration](./configuration.md) — the full `BRAIN_*` table with sources
- [Security](./security.md) — transport, authz, and the erasure posture
- [Deployment](./deployment.md) — install flows and tier files
- [Storage and migrations](./storage-and-migrations.md) — the data-side
  lifecycle this sits beside
- [Repo verification tooling](./repo-verification-tooling.md) — the gates that
  keep this page and the code in agreement, including `scripts/env-truth.sh`
