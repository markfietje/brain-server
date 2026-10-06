# Lesson 3: Domains and tenants

**Level:** L2 · **Time:** about 20 minutes · **Commands against a throwaway instance**

## What you will do

Learn the isolation model. One server can hold many knowledge worlds, and
the walls between them are real. Get this right early, because renaming and
re-slicing later is work, and getting it wrong is worse.

## Domains: the walls

A domain is a labeled knowledge world with its own memory, its own
retention, its own screening posture. Recall in one domain does not serve
rows from another. The default domain is called global, and a single-team
deployment can live there forever, but the moment two audiences must not
see each other's facts (two clients, or "internal" versus "customer-facing"),
they get two domains. That is the whole rule, and it is enforced in storage
and retrieval, not by convention.

```bash
brain domains-recompute          # refresh domain membership stats
brain domain-move 42 43 --to acme --confirm global   # move specific chunks
```

## Clients: the BPO register

When the "two audiences" are external customers, the system has a first
class shape for it, the client register. A client gets one isolation domain,
jurisdictional metadata, DPA terms, legal holds, a QA queue, and a
termination path. The verbs:

```bash
brain client add acme --jurisdiction EU          # one domain per client
brain client dpa get acme
brain client dpa set acme --retention 90 --deletion 30 --audit 365 \
  --breach 72 --onward none --sub-sub prohibited
brain client hold add acme 512 --reason "litigation hold, case 2291"
brain client hold list acme
brain client qa list acme
brain client qa coach acme 512 --note "verified source before approving"
brain client end acme --purge --yes              # terminate: purge or return
```

Details that matter in practice:

- **Jurisdiction is required** at registration. It is not decoration, it
  drives the rights machinery in lesson 6 of Level 1 and the checks in
  Level 3.
- **Holds**: adding and listing live on the CLI. Releasing a hold lives on
  the HTTP API only (`POST /legal-hold/{id}/release`). There is no CLI
  release verb, deliberately: lifting a litigation hold should be a
  deliberate, logged, API-level act.
- **QA and coaching**: supervisors get a per-client review queue and can
  attach coaching notes to specific review decisions. This is where "Maria
  approved a mood" becomes a teaching moment instead of a silent drift.
- **Termination** (`client end`) is the whole end-of-engagement ceremony:
  purge or return the data, archive the record, produce the certificate.
  Not a DELETE statement with hope.

## When to use what

| Situation | Shape |
|---|---|
| One team, one audience | The global domain, no clients |
| Internal vs external knowledge | Two domains |
| An external customer's data | A registered client, its own domain |
| One customer, several projects | Still one client domain, unless isolation is contractual |

The last row is the one people get wrong. Domains are an isolation tool,
not a folder system. If isolation is not required, do not fragment.

## Moving house, safely

`brain domain-move` moves specific chunks across the wall. It asks for
`--confirm global` when the source is the global domain, because moving
rows OUT of the shared world is the direction that surprises people later.
Read the confirmation, mean it, then confirm.

## Exercise

On a throwaway instance:

1. Register a client: `brain client add acme --jurisdiction EU`.
2. Set its DPA terms to something plausible, then read them back.
3. Place a hold on a chunk id belonging to "acme" (ingest one first if the
   domain is empty).
4. Attempt a purge for a subject whose data sits under the hold. Observe
   the hold doing its job.
5. Look at the audit trail for this lesson so far. Count the distinct
   event kinds the client operations produced.

## What you learned

- Domains are enforced walls, not labels. Clients are domains plus duty
  machinery.
- Jurisdiction is required, holds release only via the API, termination is
  a ceremony.
- Domains are for isolation, not for folders.

## Next

[Lesson 4: Getting memory in, from everywhere](04-getting-memory-in.md)
