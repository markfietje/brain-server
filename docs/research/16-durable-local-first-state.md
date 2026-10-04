# Durable Local-First State: Argon2id key derivation, WAL durability posture, and physical erasure

**File:** `src/backup.rs` (v3 writer, Argon2id + AES-256-GCM) ·
`src/standby.rs` (warm standby, encrypted chunks, RTO/RPO) ·
`src/shred.rs` + `src/service/dsar.rs` (physical residue drop) ·
`src/capacity.rs` (`SynchronousMode`, WAL autocheckpoint) ·
`src/bin/brain.rs` (`brain shred`, `brain anchor --verify`) ·
`src/anchor.rs` (off-host state fingerprint)

## The problem

A governed memory store has three separate durability stories that are usually
conflated into the word "backup". Confusing them produces systems that are
either slow, fragile, or quietly lying about what they protect.

**Confidentiality at rest.** A backup that is encrypted with something weaker
than its own passphrase is a liability sitting on a different disk. The
parameters chosen for a KDF are the entire security margin, and they are
recorded in the file, which means the choice has to be defensible years later
rather than merely convenient at authoring time.

**Durability of the primary.** SQLite's write-ahead log and its `synchronous`
pragma determine what survives power loss. A deployment can be perfectly
encrypted and still lose a committed transaction, which for an audit-chained
store is a correctness failure rather than an operational inconvenience. The
tension is real: `FULL` fsyncs on every commit and `NORMAL` does not, and the
tuned setting is much faster.

**Erasure of what was already deleted.** Logical deletion is not physical
deletion. SQLite's `secure_delete` is **off by default**, so freed page images,
the write-ahead log, and any standby chunks on a follower may still hold the
bytes of a record someone was legally required to erase. A DSAR response that
says "purged" while the plaintext survives in a WAL frame is a compliance
failure that no amount of correct application code prevents.

The open question these three share: how do you make each property measurable,
and how do you avoid claiming a stronger version of it than you built?

## The references

- **Argon2id for key derivation.** Argon2 won the Password Hashing Competition
  and is the current standard recommendation for password hashing and for
  stretching weaker secrets into keys. Its defining property is memory-hardness:
  the cost of a guess scales with memory the attacker must provision, which is
  what makes commodity GPU and ASIC attacks expensive. The `argon2` crate's
  documented defaults are `m_cost = 19456` KiB, `t_cost = 2`, `p_cost = 1`
  (verified against the crate documentation via Context7), and its `Params::new`
  constrains `m_cost` to at least `8 * p_cost` blocks.
- **RFC 9106** specifies Argon2d, Argon2i, and Argon2id and the parameter
  selection guidance. Argon2id is the hybrid variant: data-independent
  addressing like Argon2i, which resists side-channel and GPU attacks, with the
  time-memory tradeoff of Argon2d against massive precomputation. For a KDF
  stretching a passphrase, Argon2id is the default recommendation.
- **AES-256-GCM** for authenticated encryption. GCM is counter-mode encryption
  with a Galois-field authentication tag, so it provides confidentiality *and*
  integrity in one pass, and a tampered ciphertext fails to open rather than
  decrypting to plausible garbage. The 96-bit nonce is the sharp edge: reusing a
  nonce under the same key destroys the authentication guarantee entirely, which
  is why the nonce must be freshly random per artifact rather than derived.
- **SQLite WAL and `synchronous`.** `PRAGMA journal_mode=WAL` lets readers and a
  writer proceed concurrently by appending to a separate log, with
  `PRAGMA wal_autocheckpoint` controlling when that log is folded back into the
  main database and `PRAGMA wal_checkpoint(TRUNCATE)` forcing it. In WAL mode,
  `synchronous=NORMAL` is SQLite's own recommended tuning posture and
  `synchronous=FULL` is the conservative one; `PRAGMA synchronous` is
  **per-connection**, not per-database, which is the detail that makes a default
  easy to get wrong (verified against the SQLite documentation via Context7).
- **`secure_delete` and `VACUUM`.** `PRAGMA secure_delete=ON` zeroes freed
  content when SQLite reuses a page. `VACUUM` rebuilds the database into a fresh
  file, discarding the freelist and therefore discarding whatever the freed but
  not-yet-reused pages still held. Neither reaches a write-ahead log frame that
  has already been written, and neither reaches copies on other storage. The
  ordering matters: checkpoint the WAL first, or the log still holds the bytes
  the rebuild was meant to remove.
- **A deliberate non-claim: no secure-erase primitive.** On SSDs, logical
  overwriting does not reliably destroy the previous physical state, because the
  flash translation layer remaps blocks and wear-levelling means the old cells
  may never be addressed again. We therefore do not claim physical destruction on
  flash media, and the CLI prints its own ceilings per run rather than implying
  the operation was total.

## The deterministic way brain-server implements it

**Key derivation, with the parameters written into the artifact.** The backup v3
format records its own KDF parameters in a plaintext header: `{"kdf":
"argon2id", "m": 65536, "t": 3, "p": 1, "salt": ..., "nonce": ...}`, with both
salt and nonce freshly random per backup. `ARGON2_M_COST` is 65536 KiB, which is
64 MiB, roughly **3.4x the crate's own recommended default** of 19456 KiB. That
is a deliberate margin for a secret whose exposure is a shipping accident rather
than a credential-stuffing table. The derived key is 32 bytes, and the
ciphertext is `AES-256-GCM(bundle_bytes)`.

Recording the parameters is not incidental bookkeeping. It is what makes an
artifact decryptable by a future version that wants to raise the cost, and what
lets a reader *refuse* an artifact whose KDF it does not implement: `restore`
and `verify` sniff a magic value, v2 parses the header and hard-errors on an
unknown version or unknown KDF, and v1 falls back to a legacy derivation with a
loud warning. An unrecognized artifact is refused rather than guessed at.

**Durability as a declared envelope, defaulting to the conservative end.** The
capacity envelope carries a `SynchronousMode` of `Full` or `Normal`, with `Full`
as the `#[default]`. The comment on the enum records why: only the one-shot
migration connection ever set `NORMAL`, and because `PRAGMA synchronous` is
per-connection, the compile default of `FULL` is what a pooled connection
actually gets. `Normal` is the posture an operator can opt into via
`BRAIN_SYNCHRONOUS`, alongside `BRAIN_WAL_AUTOCHECKPOINT`.

Two properties make this worth trusting. First, the default is asserted equal to
the measured pre-existing behavior by a test named `envelope_defaults_equal_current_behavior`,
so the envelope cannot silently drift into being slower than what it replaced.
Second, an unrecognized value **refuses** rather than falling back, following the
project's write-posture pattern. A typo in a durability knob must not quietly
degrade the guarantee. The pragmas are applied at every pooled connection's
initialization through a named function, so the boot file does not grow a second
copy of the same logic.

**Warm standby, as shipped mechanisms rather than a new subsystem.** The standby
cycle reuses the existing v3 backup writer rather than introducing a second
encryption path, and the ordering is load-bearing and commented as such: passive
checkpoint, then base via `VACUUM INTO`, then WAL frame chunks copied *after* the
base, because the writer truncates the log and an earlier chunk copy would
replay pre-base frames and roll the restore back. Chunks ride the same
`encrypt_v3_blob` path, so **no unencrypted byte exists at rest on the follower**.
The manifest is signed last, Ed25519 over its exact bytes, so a manifest cannot
describe a set of chunks that were not all present when it was signed. Promotion
reuses the shipped restore path, registers the vector extension before touching
vec0 tables, and verifies with `integrity_check`.

**Physical erasure as an ordered, audited operation.** `brain shred` is the
counterpart to logical purge, and its order is the whole point:
`secure_delete=ON` (with the setting read back and asserted) ->
`wal_checkpoint(TRUNCATE)` -> `VACUUM` -> a second `TRUNCATE` ->
`integrity_check` -> exactly one hash-chained `forget` row. The WAL truncation
comes before the VACUUM because the rebuild cannot remove bytes the log still
holds. The receipt prints pages before and after, freelist pages after asserted
as zero, the `secure_delete` readback, and the audit row id, so the operator gets
evidence rather than a word like "done". It refuses without `--yes`, and it runs
per domain database after a purge.

**The off-host anchor, and why it is read-only.** `brain anchor` prints a
deterministic fingerprint of current state: the audit chain head, a knowledge
content census, and row counts. The operator records it *off-host*.
`--verify` recomputes and diffs. The command is read-only by design, and the
reason is neat: writing an anchor's own audit row would move the chain head the
fingerprint just recorded, so the off-host copy is the actual evidence. This
catches a class no in-tree check can, namely a knowledge table modified while the
audit chain still verifies clean.

## Measured ceiling

- **No physical destruction on flash.** `secure_delete` plus `VACUUM` removes the
  logical copy. On SSDs, wear-levelling and block remapping mean the previous
  physical state is not reliably overwritten. The CLI states this per run, and
  physical media sanitization remains an operator-level action.
- **Filesystem copies, `.bak` files, and standby chunks on the follower are out
  of scope for shred.** Shredding addresses the live database. Anything that was
  copied elsewhere must be shredded or destroyed where it lives.
- **Argon2id at 64 MiB is a cost, and the cost is paid at restore time.** The
  margin is real and it is not free. On a constrained device this is measured
  seconds, not milliseconds, and an operator restoring under time pressure will
  feel it.
- **`Full` synchronous is the default for a reason and is not free either.** The
  measured WAL trajectory is flat at zero pages in both postures under normal
  load, with a transient visible only in a 6000-document burst under the
  conservative setting. We default to the conservative end and let an operator
  opt down knowingly.
- **The anchor detects SQL-level tampering, not host compromise.** The chain key
  and the pin share the host, so an attacker with host access can forge both. It
  is a tripwire against an accidental or application-level change, and it is not
  a defense against a root adversary. The off-host copy is what makes it useful
  at all, which is also why an on-host-only anchor would be close to worthless.
- **RTO and RPO are measured on our hardware, and the standby drill is an
  operator-run procedure.** A shipper living inside the server it protects is a
  correlated failure, so the whole cycle is a CLI an operator runs, not a daemon.
  Rehearsed numbers do not transfer to different storage.