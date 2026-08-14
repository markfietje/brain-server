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

### About the maintainer

Mark is an ex-Dell Technologies engineer with **15+ years in enterprise support
(L1–L3)** across the full server and storage stack — PowerEdge, VxRail,
PowerStore, and OpenManage, with VMware and Linux underneath. For years he was
the L3 escalation point for L2 on the VMware stack, the person who saw the
cases the first two lines couldn't solve.

That background is exactly why Brain Server exists the way it does:

- **He knows what support and contact-center teams need** — recall of past
  resolutions and policy, human-gated writes, and an audit trail you can defend
  in a review.
- **He has lived the compliance stakes** — enterprise infrastructure work is
  where "what did the system do, when, and why?" stops being theoretical.
- **He works EU hours from GMT+8**, native Dutch and fluent English, and is
  available for remote or contract roles — including senior technical support,
  infrastructure engineering, or sysadmin work where that background matters.

If you're an enterprise evaluating Brain Server, you're talking to someone who
has run support at scale, not just built the tool. CV available on request.

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

To arrange a pilot or ask a question, connect with me on
[LinkedIn](https://www.linkedin.com/in/markfietje/) — or use any channel below.

---

## Contact

Connect with me on [LinkedIn](https://www.linkedin.com/in/markfietje/) — that's
the best place to reach me. For bug reports and feature requests, prefer GitHub
Issues / Discussions:

- **LinkedIn:** [linkedin.com/in/markfietje](https://www.linkedin.com/in/markfietje/)
- **GitHub (issues & discussions):**
  [github.com/markfietje/brain-server](https://github.com/markfietje/brain-server)

Reach out any time — I'm glad to help you get Brain Server running, and happy to
talk through whether it's the right fit for your use case.
