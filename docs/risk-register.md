# Risk Register — brain-server (ISO 42001 Annex A / EU AI Act Art 9)

**Source:** `THREAT_MODEL.md` (STRIDE) + `SECURITY.md` (OWASP Top 10:2025) + `AUDIT.md` ledger (55 findings).  
**Purpose:** the table a Stage-1 auditor asks for — ID · description · likelihood × impact · treatment · owner · residual. This file is the `COMPLIANCE.md` §6.1 / Art 9 pointer.  
**Update rule:** add a row when a STRIDE entry or an AUDIT finding adds a new risk; close a row only when the treatment is pinned by a test and the `AUDIT.md` disposition is closed. Keep likelihood/impact honest (no scoring inflation).

| ID | Risk (STRIDE) | Likelihood | Impact | Treatment (shipped or ceiling) | Owner | Residual |
|---|---|---|---|---|---|---|
| R-01 | Cross-tenant read via missing AuthZ gate (A01) | Low | High | JWT `tenant` from signed claim only + per-route `authorize()` at handler entry (v1.2, test-pinned `AUTHZ_GATES`) + per-record `access_scope` deny-by-default | brain-server | Low — row-level filter, not file-level in shim mode |
| R-02 | Tampered audit log (T/R) | Low | High | Keyed HMAC-SHA256 chain (epoch + head pin, v1.27.31) + `/audit/verify` + `brain_audit_chain_ok` gauge; detects SQL/app tampering, not host compromise | brain-server + operator (LUKS) | Low (app layer), Medium (host — operator disk encryption) |
| R-03 | Memory injection / poisoning (I/LITL) | Medium | High | ASI06 posture: `origin`/`source` per row + blocklist+quarantine (`screen`) + `untrusted:true` + proposal gate (`BRAIN_WRITE_POSTURE=review`) + `content_digest` 409; ONNX classifier opt-in (`injection-classifier`) | brain-server + deployer (review queue) | Medium — heuristic screen, NFKC/homoglyph folding is ceiling (zero-dep rule) |
| R-04 | PII disclosure in recall/ingest | Medium | High | Read-time deterministic redaction (no `pii_map`), `access_scope` min-necessary filter, strict masking `[redacted:*]` at write boundary (v1.14.2) | brain-server | Low |
| R-05 | Unauthenticated access (S) | Low | High | Loopback-first defaults, fail-closed on non-loopback with no auth (v1.20.29), opaque bearer (constant-time) or JWT/JWS + OIDC discovery (v1.2), `/.well-known/*` single public-path source (Blackout) | brain-server | Low |
| R-06 | Token replay / algorithm confusion (S) | Low | High | `ALLOWED_ALGS` whitelist before key lookup (RS256/ES256/EdDSA, `none`/HS*/PS* rejected), `(jti,iss)` denylist ≤60s (bounded), refresh-chain reuse detection burns family | brain-server | Low — 60s window is accepted trade-off |
| R-07 | Channel/out forgery, steering laundering (S/T) | Low | High | `RESERVED_OUTBOX_TOPICS` at `enqueue_child` → `400 topic_reserved`, `post_steering` is `approve`-role gated + args truth (X-W1/X-L1, Wardline + Truthglass) | brain-server | Low |
| R-08 | Image/beacon exfiltration (I) | Low | High | Doc-mode images default OFF + operator allowlist, favicon proxy default OFF, `data:` ≤64 KiB (Shutter); markdown refs stripped at read seam | brain-server + operator (allowlist is trust) | Low (doc-mode), Medium (bare URLs linkified-but-inert by contract) |
| R-09 | Supply-chain / transitive dep (T) | Low | High | `cargo audit` in CI, pinned `Cargo.lock`, SBOM per release (CycloneDX), minimal optional features; `rsa 0.9.10` Marvin accepted with local-daemon model | brain-server | Low — Marvin is documented ignore |
| R-10 | Denial of service — burst / vector query (D) | Medium | Medium | Per-IP tiered rate limiting (per-tenant buckets are NOT shipped — the shared-bucket gap is X-A10's accepted residual), capacity envelopes (507 on ingest, reads never blocked), per-token/byte HTTP limits, `MAX_NOTES_PER_RUN=1000` | brain-server + reverse proxy | Low (loopback), Medium (shared loopback bucket X-A10 until per-principal) |
| R-11 | Encryption at rest (I) | Low | High | No app-level encryption at rest; operator LUKS/FileVault is the layer — standing statement: DB + `.bak` PLAINTEXT on primary, encryption law covers follower only; SQLCipher per-tenant keys v3.7 horizon | operator | Medium until v3.7 |
| R-12 | Unwarranted erasure / litigation hold miss (R) | Low | High | DSAR / `/purge` are Admin-only, explicit + tombstoned + audited; `legal_holds` freezes every erasure path (`409 legal_hold_active`, deferred `held_ids` on cert) | brain-server | Low |
| R-13 | Egress allowlist bypass (I) | Low | High | Insert-only `RwLock<HashMap>` allowlist, validate-on-first-use, IANA special-purpose tables, `BRAIN_EGRESS_ALLOW_PRIVATE=1` loud opt-out (Deadbolt) | operator (`BRAIN_EGRESS_*`) | Low — allowlisted host is trust |
| R-14 | Revocation eventual consistency (S) | Low | Medium | 60s negative-cache TTL, hot reload not shipped (PEM drop + restart X-A3b) | operator | Medium — accepted JWT trade-off |

**Scale note (ISO 42001 Clause 6.1):** Likelihood is assessed for the loopback-first, single-process deployment that this repo ships. A non-loopback, multi-tenant internet deployment moves R-01/R-10 to Medium likelihood until per-principal rate buckets + file-level tenant isolation ship — note this in the procurement response.

**Traceability:** every row above maps to a `THREAT_MODEL.md` STRIDE entry and/or an `AUDIT.md` finding. Keep this file in sync — `CONTACT_CENTER_STANDARDS.md` and `COMPLIANCE.md` §6.1 point here as the Art 9 / ISO 42001 Clause 6.1 evidence.

*Next review trigger: any STRIDE change, any AUDIT ledger add/close, or the Loop line (v1.32.x) landing.*
