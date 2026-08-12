# Support

> v1.20.10 "Proof" — the support statement the CRA/regulatory bar expects.
> Honest and explicit about the self-hosted, best-effort, no-SLA posture.

## Supported versions

See `SECURITY.md` → **Supported Versions** for the matrix and the support
window. In short: the current minor and the previous minor receive fixes;
older LTS lines receive back-compat/security fixes only.

## Reporting problems

- **Bugs / behaviour questions** — open a GitHub issue with the version
  (`GET /health` → `version`) and a repro.
- **Security vulnerabilities** — do **not** file a public issue. Use the
  GitHub Security Advisories "Report a vulnerability" flow, per
  `SECURITY.md` (48h ack, 5-business-day fix target, coordinated disclosure).

## Updates

- Follow `CHANGELOG.md`; upgrade per `docs/deployment.md` (server restart via
  `scripts/install-service.sh`).
- Each release ships a CycloneDX SBOM in `dist/` (see `docs/cra.md`) so you can
  scan dependencies independently.

## The honest part

brain-server is a **self-hosted, best-effort project**. There is **no SLA and
no paid support tier**. It is published for operators who run their own memory
store and take responsibility for it. If you need contractual support, engage a
vendor; nothing here should be read as an enterprise support commitment.
