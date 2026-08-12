# CRA Evidentiary Kit

> v1.20.10 "Proof" — an assembly of already-shipped evidence for the EU Cyber
> Resilience Act (CRA, in force 2026) "reporting + support + SBOM" bar. This is
> **not** a claim of formal conformity assessment; it is the evidentiary bundle
> an auditor/reviewer needs to evaluate that claim.

## What the CRA evidentiary kit is

The CRA makes a manufacturer responsible for the *security of the digital
elements* of a product across its life — including producing a **software bill
of materials (SBOM)**, a **vulnerability reporting channel**, and a **security
support window**. brain-server already ships each of these; `scripts/cra-kit.sh`
assembles them into one hashed bundle:

```
scripts/cra-kit.sh
```

writes `dist/cra-kit/`:

| Artifact | Source | What it evidences |
|----------|--------|-------------------|
| `brain-server-<ver>.cdx.json` | `scripts/sbom.sh` (CycloneDX from `Cargo.lock`) | SBOM — full dependency tree for component/supply-chain scan |
| `SECURITY.md` | repo | reporting path + supported-versions window |
| `SUPPORT.md` | repo | support statement + update guidance + no-SLA honesty |
| `deployment.md` | `docs/deployment.md` | how the product is deployed/updated |
| `CRA_MANIFEST.json` | generated | SHA-256 index of every artifact (integrity pin) |

Idempotent: re-running rebuilds from the same sources, so hashes are stable for
unchanged content. The only external tool is `shasum`/`sha256sum` (present on
macOS and Linux).

## Relationship to the SBOM (pre-existing)

The per-release CycloneDX SBOM predates this kit (v1.17.5 ships it into `dist/`
on every tag release; `SECURITY.md` §SBOM documents it). The kit merely wraps
it with the reporting + support docs the CRA pairs with it, so the whole
evidentiary story is answerable in one command.

## Honest ceiling

This kit assembles **evidence**, not **certification**. Conformity assessment,
an EU-type designation, or a formal declaration of conformity are legal steps
performed by the responsible manufacturer against the regulation's security
requirements (including Annex I security requirements and any applicable
harmonised standard) — none of which this repository performs or claims. Where
the regulation's requirements exceed what a self-hosted, operator-run store can
truthfully assert (e.g. organizational "responsible manufacturer" obligations
or 24/7 coordinated-vulnerability-disclosure staffing), this kit is the record
that surfaces the gap rather than hiding it.
