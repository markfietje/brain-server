# Plan — The Six Loops: Final Architecture

**Owner:** MemorySteward LLC · private · 2026-09-27
**Scope:** The definitive 6-loop architecture — what each loop does, why it exists, and how they fit together
**Status:** AGREED — this is the architecture

---

## 1 · The Core Insight

Knowledge has a lifecycle. Software has a lifecycle. They are not the same lifecycle, but they share the same shape:

```
Create → Use → Improve → Deliver
```

The 6 loops cover both lifecycles:

| Loop | Lifecycle | What it does |
|---|---|---|
| **Create** | Knowledge | Discover new knowledge, test it, add it to the base |
| **Solve** | Knowledge | Find answers with sources, ask experts when unsure |
| **Evolve** | Knowledge | Review, approve, and publish knowledge |
| **Deflect** | Knowledge | Reuse what works, measure what sticks |
| **Operate** | Knowledge | Watch outcomes, learn from results, improve |
| **Deliver** | Software | Build, test, release, and track software |

---

## 2 · The Six Loops in Order

```
┌─────────────────────────────────────────────────────────────────────────┐
│                                                                         │
│  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌────────┐│
│  │ Create  │───▶│  Solve  │───▶│ Evolve  │───▶│ Deflect │───▶│Operate ││
│  │         │    │         │    │         │    │         │    │        ││
│  │ Discover│    │ Find    │    │ Review  │    │ Reuse   │    │ Watch  ││
│  │ Test    │    │ Ask     │    │ Publish │    │ Measure │    │ Learn  ││
│  │ Add     │    │         │    │         │    │         │    │Improve ││
│  └─────────┘    └─────────┘    └─────────┘    └─────────┘    └────────┘│
│       │              │              │              │              │     │
│       │              │              │              │              │     │
│       ▼              ▼              ▼              ▼              ▼     │
│  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌────────┐│
│  │ SECI    │    │ KCS v6  │    │ KCS v6  │    │ KCS v6  │    │ KCS v6 ││
│  │ GRAI    │    │ ISO     │    │ ISO     │    │ COPC    │    │ ISO    ││
│  │ AKI     │    │ 30401   │    │ 30401   │    │ 8.0     │    │ 30401  ││
│  │ CKLT    │    │         │    │         │    │ ISO     │    │ DORA   ││
│  │ AutoKD  │    │         │    │         │    │ 10002   │    │        ││
│  │Xcientist    │         │    │         │    │         │    │        ││
│  │ResearchLoop │         │    │         │    │         │    │        ││
│  └─────────┘    └─────────┘    └─────────┘    └─────────┘    └────────┘│
│                                                                         │
│                              ┌─────────┐                                │
│                              │ Deliver │                                │
│                              │         │                                │
│                              │ Scope   │                                │
│                              │ Design  │                                │
│                              │ Verify  │                                │
│                              │ Release │                                │
│                              │ Operate │                                │
│                              └─────────┘                                │
│                                   │                                     │
│                                   ▼                                     │
│                              ┌─────────┐                                │
│                              │ SLSA    │                                │
│                              │ in-toto │                                │
│                              │ ISO     │                                │
│                              │ 29110   │                                │
│                              │ DORA    │                                │
│                              │ ITIL 4  │                                │
│                              └─────────┘                                │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 3 · The Knowledge Loops (5)

These 5 loops manage knowledge throughout its lifecycle.

### 3.1 Create — Knowledge Creation

**Why it exists:** Knowledge doesn't just appear. Someone has to discover it, test it, and add it to the base. Without Create, the knowledge base is static — it only contains what was manually entered.

**What it does:**

| Phase | What it does | Example |
|---|---|---|
| **Discover** | Identify knowledge gaps, research questions, innovation opportunities | "We don't have a procedure for damaged items over $500" |
| **Hypothesize** | Generate hypotheses, proposals, candidate solutions | "Maybe we can extend the refund window to 30 days" |
| **Validate** | Test hypotheses through experimentation, simulation, or prototyping | "Let's try it with 10 customers and see what happens" |
| **Integrate** | Merge validated findings into the knowledge base | "It worked. Let's add it to the procedure." |
| **Disseminate** | Share new knowledge across the organization | "Everyone, we now have a new procedure for damaged items" |

**Scientific basis:** SECI (Nonaka & Takeuchi, 1995), GRAI (Böhm & Durst, 2025), AKI (Kirchner & Scarso, 2026), CKLT (Zhang, 2026), AutoKD (2026), Xcientist (2026), ResearchLoop (Xia & Wang, 2026)

**Standard:** SECI, GRAI, AKI, CKLT, NIST AI RMF

---

### 3.2 Solve — Knowledge Retrieval

**Why it exists:** Knowledge is only useful if you can find it. Solve is the loop that finds answers with evidence.

**What it does:**

| Phase | What it does | Example |
|---|---|---|
| **Recall** | Find answers with sources | "What's the refund policy for damaged items?" |
| **AskHuman** | Ask experts when unsure | "This is ambiguous. Let me ask Maria." |
| **Evidence** | Show where the answer came from | "Source: Refund Policy v3.2, Section 4.1" |
| **Confidence** | Show how confident the system is | "Confidence: High (2 sources, 45ms)" |

**Scientific basis:** SECI (Internalization), KCS Solve Loop

**Standard:** KCS v6 / KCS 2027, ISO 30401

---

### 3.3 Evolve — Knowledge Publication

**Why it exists:** Knowledge needs to be reviewed before it's published. Evolve is the loop that reviews, approves, and publishes knowledge.

**What it does:**

| Phase | What it does | Example |
|---|---|---|
| **Propose** | Submit knowledge for review | "John proposed a new procedure for password resets" |
| **Gate** | Human review and approval | "Maria reviewed and approved it" |
| **Publish** | Make knowledge available to everyone | "The new procedure is now live" |
| **Supersede** | Replace old knowledge with new | "The old procedure is now superseded" |

**Scientific basis:** SECI (Externalization + Combination), KCS Evolve Loop

**Standard:** KCS v6 / KCS 2027, ISO 30401

---

### 3.4 Deflect — Knowledge Reuse

**Why it exists:** Knowledge is only valuable if it's reused. Deflect measures how often knowledge is reused and what sticks.

**What it does:**

| Phase | What it does | Example |
|---|---|---|
| **Reuse** | Use existing knowledge instead of creating new | "The agent found the answer in the knowledge base" |
| **Measure** | Track reuse rate, deflection rate, repeat rate | "Deflection rate: 78% (up from 65% last month)" |
| **Flag** | Flag knowledge that isn't being used | "This article hasn't been used in 90 days" |

**Scientific basis:** SECI (Combination), KCS Deflect Loop, COPC 8.0

**Standard:** KCS v6 / KCS 2027, COPC 8.0, ISO 10002

---

### 3.5 Operate — Knowledge Feedback

**Why it exists:** Knowledge needs feedback to improve. Operate watches outcomes, learns from results, and feeds improvements back into Evolve.

**What it does:**

| Phase | What it does | Example |
|---|---|---|
| **Observe** | Monitor knowledge base usage, recall quality, deflection rate | "Recall quality dropped 5% this month" |
| **Attribute** | Attribute outcomes to specific articles, decisions, models | "The drop is caused by the new refund policy article" |
| **Improve** | Feed outcomes back into Evolve (retrain, update, retire) | "Let's update the refund policy article" |

**Scientific basis:** Double-loop learning (Argyris & Schön, 1978), NIST AI 800-4 (2026)

**Standard:** KCS v6 / KCS 2027, ISO 30401, DORA

---

## 4 · The Software Loop (1)

This loop manages software throughout its lifecycle.

### 3.6 Deliver — Software Delivery

**Why it exists:** Software needs to be built, tested, released, and tracked. Deliver is the loop that manages the software lifecycle.

**What it does:**

| Phase | What it does | Example |
|---|---|---|
| **Scope** | Define what to build | "We need a new module for music education" |
| **Design** | Plan how to build it | "Here's the architecture and data model" |
| **Verify** | Build, test, and QA | "All tests pass, ready for release" |
| **Release** | Build, attest, approve, promote | "Version 1.0.0 is now live" |
| **Operate** | Observe, attribute, improve | "99.9% uptime, no critical bugs" |

**Scientific basis:** SLSA v1.2 (2025), in-toto v1.0, ISO 29110, DORA, ITIL 4

**Standard:** SLSA v1.2, in-toto, ISO 29110, DORA, ITIL 4

---

## 5 · How the Loops Connect

### 5.1 Knowledge Flow

```
Create ──▶ Solve ──▶ Evolve ──▶ Deflect ──▶ Operate
  │          │          │          │          │
  │          │          │          │          │
  ▼          ▼          ▼          ▼          ▼
Discover  Find     Review    Reuse     Watch
Test      Ask      Publish   Measure   Learn
Add       Evidence Supersede Flag      Improve
```

### 5.2 Feedback Loops

```
Operate ──▶ Evolve: Outcomes feed back into knowledge publication
Operate ──▶ Create: Knowledge gaps trigger new creation cycles
Deflect ──▶ Create: High repeat rates trigger new article proposals
Deliver ──▶ All: New software versions improve all loops
```

### 5.3 The Complete Cycle

```
┌─────────────────────────────────────────────────────────────────────────┐
│                                                                         │
│  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌────────┐│
│  │ Create  │───▶│  Solve  │───▶│ Evolve  │───▶│ Deflect │───▶│Operate ││
│  │         │    │         │    │         │    │         │    │        ││
│  │ Discover│    │ Find    │    │ Review  │    │ Reuse   │    │ Watch  ││
│  │ Test    │    │ Ask     │    │ Publish │    │ Measure │    │ Learn  ││
│  │ Add     │    │         │    │         │    │         │    │Improve ││
│  └─────────┘    └─────────┘    └─────────┘    └─────────┘    └────────┘│
│       ▲              │              │              │              │     │
│       │              │              │              │              │     │
│       └──────────────┴──────────────┴──────────────┴──────────────┘     │
│                                                                         │
│                              ┌─────────┐                                │
│                              │ Deliver │                                │
│                              │         │                                │
│                              │ Scope   │                                │
│                              │ Design  │                                │
│                              │ Verify  │                                │
│                              │ Release │                                │
│                              │ Operate │                                │
│                              └─────────┘                                │
│                                   │                                     │
│                                   ▼                                     │
│                              ┌─────────┐                                │
│                              │ All 5   │                                │
│                              │ loops   │                                │
│                              │ improve │                                │
│                              └─────────┘                                │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 6 · Why These 6 Loops

### 6.1 The Original 3 (KCS v6)

KCS v6 defined the double-loop model:

- **Solve Loop:** Find answers, reuse knowledge, capture new knowledge
- **Evolve Loop:** Review, approve, publish, improve knowledge

KCS v6 added a third loop:

- **Deflect Loop:** Measure reuse, track deflection rate

### 6.2 The 4th Loop (Deliver)

The Deliver loop was added because:

- Knowledge needs software to be useful
- Software needs to be built, tested, released, and tracked
- Software delivery needs the same governance as knowledge management

### 6.3 The 5th Loop (Operate)

The Operate loop was added because:

- Knowledge needs feedback to improve
- Outcomes need to be attributed to specific knowledge
- Improvements need to feed back into Evolve

### 6.4 The 6th Loop (Create)

The Create loop was added because:

- Knowledge doesn't just appear — it needs to be created
- Knowledge gaps need to be identified
- New knowledge needs to be tested and validated
- Knowledge creation is distinct from knowledge management

---

## 7 · The One-Page Summary

> **MemorySteward** is the guided knowledge lifecycle for teams that can't afford wrong answers.
>
> **5 knowledge loops:**
> - **Create** — Discover new knowledge, test it, add it to the base
> - **Solve** — Find answers with sources, ask experts when unsure
> - **Evolve** — Review, approve, and publish knowledge
> - **Deflect** — Reuse what works, measure what sticks
> - **Operate** — Watch outcomes, learn from results, improve
>
> **1 software loop:**
> - **Deliver** — Build, test, release, and track software
>
> Every answer has evidence. Every write passes human review. Every action leaves a clear history.
>
> **Setup takes 5 minutes. Trust takes forever.**
