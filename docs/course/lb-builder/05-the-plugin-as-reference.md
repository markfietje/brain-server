# Lesson 5: The chat plugin, read as a reference integration

**Level:** B · **Time:** about 20 minutes · **Reading code-shaped ideas, no build**

## Questions people ask

**Is there a working integration I can copy?** Yes, the chat-host plugin
(`plugin/` in the repo). It is a thin TypeScript shim: on every turn,
before the model is prompted, it queries `/recall`, wraps the hits in the
fence, and hands the block to the host as context to prepend. No model of
its own, no tokens, milliseconds per turn.

**What are the safety knobs I should steal?** The plugin's config surface
is the checklist:

- `agents`: allowlist of assistant identities. Empty disables everything.
- `allowedChatTypes`: direct conversations by default. Group and channel
  are opt-in, because injecting memory into a room is the classic leak.
- `untrustedOrigins`: `label` (visible origin prefix on captured hits) or
  `exclude` (captured hits dropped from model context entirely, both the
  injected fence and the recall tool result).
- `captureMode`: `proposal` keeps captures in the human queue.
- Plus a startup typecheck: a config with a string where a list belongs
  refuses to boot, because that exact type error once degraded an
  allowlist into substring matching.

**What is the turn-time budget?** The plugin's recall call is bounded
(2s default timeout, fail-open on error: a memory hiccup must never stall
the chat). Copy that posture: memory is an enhancement, not a dependency
your uptime depends on.

## The tool surface the plugin registers

Beyond auto-inject, the plugin exposes tools to the assistant
(`brain_memory_recall`, `brain_memory_store`, graph tools, proposal
review tools behind an opt-in). Two design notes worth stealing: tool
names are namespaced (`brain_*`) so they never collide with host built-ins
(confused-deputy hygiene), and the write tools end in proposals, never
direct writes.

## Exercise

Read `plugin/src/tools.ts` and `plugin/src/format.ts` in the repo
fifteen minutes each, with two questions: where exactly does the fence get
generated, and where does the origin label get decided? Then sketch, in
one page, the same two answers for YOUR host. If you cannot answer for
your host, your host's memory integration is not designed yet, it is
assembled.

## Next

[Lesson 6: The architecture and the loops, a builder's tour](06-architecture-and-loops.md)
