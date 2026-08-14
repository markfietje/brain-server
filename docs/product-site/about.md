# About & Contact

**Brain Server** is a local-first semantic memory and knowledge-graph server for
AI agents. It is built to answer the hardest question an agent memory system
faces in 2026: *"what did the agent know, when, and why — and can I delete it
on request?"*

Everything here is self-hosted, deterministic, and human-auditable. There is no
cloud, no per-query cost, and no LLM in the recall loop. The write path is
human-gated, every decision lands in a tamper-evident audit chain, and personal
data can be erased on demand with a verifiable deletion certificate.

---

## About the project

- **One self-hosted binary.** Server + CLI + MCP run from a single Rust build;
  it works on a 4 GB ARM edge device just as well as a beefy server.
- **Deterministic retrieval.** Recall never has to "think" — a static local
  embedding model plus a deterministic pipeline answer the query without an LLM
  deciding and without data leaving the machine.
- **Honest by design.** Every mechanism ships with its own documented ceiling.
  We would rather tell you what the system *doesn't* do than overstate it.
- **Open source.** The code, the research explainers, and the security claims
  are all in the [repository](https://github.com/markfietje/brain-server) —
  you can verify them live on a throwaway instance.

The project is maintained by **Mark Fietje**, an independent developer focused
on privacy-preserving, human-governable AI infrastructure.

---

## Free pilot & trial on your own hardware

If you are an enterprise or team evaluating Brain Server for a real deployment,
you don't need to take our word for it. Run it on **your own hardware** — a
laptop, a VM, or an on-prem box — and see how it behaves with your data.

A free pilot is available:

- **Self-serve first.** Install the open-source build, follow the
  [Quickstart](./quickstart.md), and you're running in minutes. No sign-up, no
  license key.
- **Hands-on support when you want it.** If you'd like guidance setting up a
  pilot, help mapping a specific requirement (compliance, tenancy, SSO), or a
  walkthrough of how the audit chain and DSAR work on your infra, just reach
  out. We're happy to help you get a trial running — at no cost and with no
  obligation.

To arrange a pilot or ask a question, use any channel below.

---

## Contact

Prefer issues for bug reports and feature requests, and the channels below for
anything else.

- **GitHub (issues & discussions):**
  [github.com/markfietje/brain-server](https://github.com/markfietje/brain-server)
- **X (Twitter):** [x.com/mark_fietje](https://x.com/mark_fietje)
- **LinkedIn:** [linkedin.com/in/markfietje](https://www.linkedin.com/in/markfietje/)
- **Signal:** `markfietje.77`
- **WhatsApp:** `markfietje`

Reach out any time — we're glad to help you get Brain Server running, and happy
to talk through whether it's the right fit for your use case.
