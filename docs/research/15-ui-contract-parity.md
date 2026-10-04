# UI Contract Parity: one fixture, five consumers, and a byte-equality wire gate

**File:** `plugin/fixtures/invisible-classes.json` (the canonical set) ·
`src/strip_invisible.rs` (server) · `shell/src/lib/sanitize.ts` (SvelteKit +
Tauri shell) · `shell/tests/sanitize.test.ts` (parity test) ·
`shell/tests/drift-gate.test.ts` (wire byte-equality) · `shell/src/lib/api/schema.d.ts`
(generated client) · `.github/workflows/shell.yml` (the lane that runs it)

## The problem

A governed memory server grows frontends. Ours is a Dioxus client, a SvelteKit
plus Tauri shell, an OpenClaw plugin, and an MCP surface, all reading the same
kernel. That sounds like a solved problem and it is not, because two
independent failure modes appear the moment a second consumer exists.

The first is **contract drift on the wire**. A frontend that hand-writes its
request and response types against a reading of the API documentation will
compile happily while disagreeing with the server about a field name, an enum
member, or a required parameter. The failure surfaces at runtime, in
production, as a 400 nobody can reproduce locally.

The second is **semantic drift on a sanitizer**. When five independent
implementations each decide which Unicode scalars are invisible, they diverge.
Slowly, and then all at once. Someone adds bidi isolates to the server set
because a smuggling class needed it. The plugin still strips the old set. The
shell strips a third set. Every one of them has tests, every one of them is
green, and the boundary quietly differs by tree.

The uncomfortable part is that both failure modes are invisible to the kind of
testing that usually catches them. A green unit suite proves each sanitizer
agrees with itself. Nothing proves they agree with each other, and nothing
proves the types match the server.

## The references

- **Generated clients from an OpenAPI document.** The contract-first pattern:
  the machine-readable schema is the single source of truth and client types
  are a build artifact rather than a hand-maintained copy. This is the
  long-standing practice behind OpenAPI Generator and the reason the
  specification exists in the shape it does. Our contribution is not the
  generator but the gate: regeneration happens in a temporary directory and the
  output is compared byte for byte against the committed file, so the artifact
  cannot be quietly hand-edited or fall behind.
- **Unicode bidirectional control characters as a security class.** Unicode
  Technical Standard #9 defines the bidirectional algorithm; the
  `Bidi_Control` property marks the formatting characters that manipulate it.
  Trojan Source (CVE-2021-42574) established that source code reviewed as
  rendered text can differ from the source executed, and the security
  guidance that followed treats these characters as a review hazard in their
  own right. Our treatment follows the guidance's shape: remove them at the
  rendering boundary, preserve the stored bytes, and keep the removal a pure
  function of the scalar value.
- **Biometric presentation-attack detection, for the naming.** Not an
  analogue for the mechanism, but the vocabulary is worth keeping honest:
  a detection system's job is to reject a sample that imitates a genuine one,
  and a detector that has never been shown a forged sample has not been shown
  to work. The parity tests below are the analogue: a sanitizer that has never
  been compared against its siblings has not been shown to work.
- **Multi-implementation conformance suites.** The general engineering answer
  to N implementations of one rule is a shared conformance fixture rather than
  N hand-written expectation lists. Cross-platform engine test suites and the
  Unicode normalization conformance data work this way. The design choice that
  matters: the fixture is data, so adding a class is an edit to one file rather
  than a coordinated commit across five trees.

## The deterministic way brain-server implements it

**The wire side: a byte-equality drift gate.** The shell's typed client lives
in `src/lib/api/schema.d.ts`, generated from the kernel's `openapi.yaml` by
`openapi-typescript`. The committed file is never regenerated in place by a
test. `shell/tests/drift-gate.test.ts` regenerates into a temporary directory
and asserts `expect(regenerated).toBe(committed)`: byte equality, not
structural similarity. A hand-edit to the committed file fails. A server-side
field change that nobody regenerated fails. CI additionally runs a temporary
regeneration and byte-compares, so the check does not depend on anyone running
the generator locally first. Exactly one command rewrites the file, and it is
deliberate.

**The sanitizer side: one fixture, five consumers.** The canonical set is
`plugin/fixtures/invisible-classes.json`, expressed as named classes of
inclusive hex ranges. It is consumed by:

1. the server Rust library test, which scans **every** scalar value in the
   Unicode range against the file and fails on any disagreement with
   `is_invisible`;
2. the shell's `sanitize.ts`, whose regex is asserted to be the exact scalar
   membership set;
3. `shell/tests/sanitize.test.ts`, which reads the JSON from the kernel root
   and fails if the shell's predicate drifts;
4. the OpenClaw plugin's vitest suite;
5. the client crate's Rust test.

The server test is exhaustive rather than sampled, which is the property worth
noticing. It does not check a list of interesting code points. It walks the
whole space and compares, so a missing range on either side is a failure rather
than an untested corner.

**What the shell does not do.** Its `stripInvisible` is not an HTML sanitizer.
It neither parses nor emits markup, and it does not attempt to be one. Markup
has a separate boundary: `{@html}` is banned by lint in the shell, so the
question never arises at runtime. Keeping these two boundaries separate means
neither one grows a false sense of coverage. The docstring says so explicitly,
which is the cheapest defense against a future reader assuming otherwise.

**The lane that ties it together.** `shell.yml` triggers on `shell/**` *and* on
`plugin/fixtures/invisible-classes.json`, so a change to the canonical set
re-runs every consumer rather than only the tree that changed. That trigger is
the actual mechanism. Without it, the fixture could be edited in a pull request
that touched no shell file, the shell's own tests would not fire, and the drift
would land.

## Measured ceiling

- **Parity is membership, not behavior.** The fixture pins *which scalars are
  invisible*. It does not pin what any consumer does beyond removal. A consumer
  that strips the set and then re-inserts a bidi override through some other
  path passes every test here.
- **Five consumers is a maintenance ceiling, not a design target.** Each one is
  a place the next person must remember to check. The fixture keeps them
  honest; it does not make adding a sixth cheap.
- **The wire gate covers the shell's client only.** The plugin's MCP and the
  Dioxus client do not consume the generated `schema.d.ts`. Their wire typing is
  hand-written and their drift is caught by route and contract tests rather than
  by byte equality against the kernel document.
- **Byte equality is strict on purpose.** It will fail on a generator version
  bump even when the resulting types are semantically identical. That is the
  intended behavior for a security boundary: a surprising red build is cheaper
  than an unnoticed change in what the compiler believes the server said.
- **Removing characters is lossy and we accept it.** A legitimate string
  containing a zero-width joiner, which is common in several scripts, loses
  those characters on the rendering path. Storage keeps the bytes verbatim;
  this is a display transform only. Callers who need the exact sequence read
  the stored value, not the rendered one.