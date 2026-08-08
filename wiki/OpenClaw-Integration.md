# OpenClaw Integration

Brain Server is the **memory backend for [OpenClaw](https://github.com/openclaw)**, the open-source personal AI assistant gateway. The integration is a TypeScript plugin that calls `/recall` each turn via OpenClaw's `before_prompt_build` hook — so the agent gets the right memory injected, with zero token cost and zero data egress.

## How it works

The plugin lives in `plugin/` in the repository. On every turn:

1. OpenClaw's `before_prompt_build` hook fires.
2. The plugin calls `POST /recall` on Brain Server with the conversation context.
3. Brain Server runs the deterministic retrieval pipeline and returns the top evidence.
4. The plugin injects that evidence into the prompt.

Because recall is **deterministic and local**, it's effectively free — a drop-in for the `active-memory` sub-agent, in the same memory slot.

## Configuration

OpenClaw reads its config from `~/.openclaw/openclaw.json`. The plugin is registered at the `brain-server` key:

```json
{
  "brain-server": {
    "baseUrl": "http://127.0.0.1:8765",
    "tokenFile": "~/.config/brain-server/auth-token",
    "enabled": true
  }
}
```

The plugin supports **per-agent opt-in** and **group/channel exclusions**, so you can prevent data leakage across contexts.

## The memory slot

Brain Server occupies OpenClaw's memory slot as a `kind: "memory"` plugin. This means:

- It replaces/augments the `active-memory` sub-agent.
- It is **deterministic** — the same query returns the same evidence (no LLM in the loop).
- It is **domain-graphed** — health, business, and code memories are kept separate and auto-routed.

## Why use it with OpenClaw

- **Zero per-query cost** — no embedding API on every read/write.
- **Zero data egress** — the agent's memory stays on the host.
- **Zero recall latency** — no round-trip to the cloud.
- **Human-gated writes** — nothing enters memory without approval if you enable the proposal queue.
- **Auditable** — recall traces replay exactly what was injected each turn.

## Next steps

- **[Use Cases](Use-Cases)** — worked examples.
- **[Quickstart](Quickstart)** — run the server first.
- **[Architecture](Architecture)** — how recall works under the hood.
