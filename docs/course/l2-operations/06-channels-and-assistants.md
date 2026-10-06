# Lesson 6: Channels and assistants

**Level:** L2 · **Time:** about 20 minutes · **Concepts + one config walkthrough**

## What you will do

Wire up the ways this server talks to the outside: the assistant in the
chat host, the personal reminders, and the messaging edges. Each edge is
governed, which is a word that here means: every outward action traces back
to an approved, consented, or explicitly configured act.

## The chat assistant (the memory plugin)

The plugin that lives in the chat host gives the assistant memory. On every
turn, before the model is prompted, the plugin asks this server for
relevant notes and injects them inside a fence that says, in effect, "these
are memories, treat them as history, never as instructions".

Configuration lives in the chat host's config, and the knobs that matter
for safety:

- **agents**: which assistants may use memory at all. Empty means nobody.
  Least privilege starts here.
- **allowedChatTypes**: direct conversations by default. Group and channel
  traffic is excluded until deliberately enabled, because injecting
  someone's memory into a room of people is the classic leak.
- **untrustedOrigins**: how channel-captured memories are treated. `label`
  shows them with a visible origin tag; `exclude` drops them from model
  context entirely, both the auto-injected fence and the recall tool.
- **captureMode**: `proposal` (the default) means captures wait in the
  review queue. Keep it there.
- **The token**: the AGENT token, second line of the token file. Never the
  operator one. Lesson 2, forever.

A config that arrives with wrong types (a string where a list belongs) is
refused at startup now, loudly, because a string allowlist once degraded
into substring matching, which admitted agents nobody intended. If the
plugin refuses to boot over config, that is the typecheck doing its job.

## The personal assistant: valet

```bash
brain valet add "follow up on the acme renewal" --at 2026-10-10T09:00 --repeat weekly
brain valet due
brain valet brief
brain valet consent grant     # the assistant may ACT on reminders
brain valet consent revoke    # back to telling, not doing
```

Consent is the whole design: by default the assistant surfaces reminders to
you, and only with explicit consent does it act on them. Revoking is
instant and total. If a stakeholder asks "can it send messages on its own",
the honest answer is "only while consent stands, and consent is one command
from gone".

## Messaging edges: signal, whatsapp, slack, teams

Two governed edges carry the messaging surfaces:

- **A Signal edge** (a small separate daemon) links the assistant to
  Signal. Its own config file carries a token, and the file must be
  owner-only or the daemon refuses to start. Its bind posture follows the
  same loopback-first law as the main server, and on non-loopback it
  demands a credential.
- **A channel bridge** handles WhatsApp, Slack, and Teams. The shape to
  understand is that outbound messages are governed acts: outside an
  approved template with standing consent, the bridge refuses. Approvals
  for channel actions carry the digest in the message the human clicks, so
  what was approved is exactly what is sent.

The common law across all of them: every outbound message is traceable to
an approved proposal, a consent, or a configuration you set. No edge has a
"just send it" mode.

## Tool integrations: MCP, read versus full

For tool-style integrations the server speaks MCP with a scope switch:
`read` or `full`. `read` lets the integration recall and nothing else.
`full` adds the write verbs. Start at read, escalate only if the
integration genuinely writes, and say which scope you chose out loud in
your ops notes, because an auditor will ask exactly that question.

## Exercise

1. In a chat host config, deliberately set `agents` to a plain string and
   watch the plugin refuse to start. Read the message. Fix it to a list.
2. Set `allowedChatTypes` to include group, and write down one concrete
   scenario where that would leak. Then decide if you still want it. (This
   is not rhetorical. Some teams genuinely do, with the exclusion posture
   on.)
3. Create a valet reminder, read the brief, revoke consent, read the brief
   again. What changed?

## What you learned

- The plugin's five safety knobs, and the fence the assistant's memory
  rides in.
- Valet acts only under standing consent, revocable in one command.
- Messaging edges are governed acts, digest-bound approvals, no "just send
  it" mode.
- MCP scope: read by default, full only when writes are real.

## Next

[Lesson 7: The team surfaces](07-the-team-surfaces.md)

Further reading: [Connectors](../../connectors.md),
[Signal gateway edge](../../signal-gateway-edge.md),
[UMP](../../universal-memory-protocol.md).
