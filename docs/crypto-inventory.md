# Cryptographic inventory & algorithm-agility seams

NCCoE SP 1800-38B shape: every algorithm this product deploys, what it
protects, its harvest-now-decrypt-later (HNDL) exposure verdict, and the
SWAP PATH — the named code seam a replacement lands through. This file is
the deliverable the Enterprise Line's PQC milestone (v1.28.62) pins: the
`reg_watch::pqc_inventory_seam_deliverable` test asserts the inventory AND
the two agility seams below stay present and truthful.

Posture: *documented measurement*, not certification. No PQC primitive is
deployed anywhere in this codebase — this document is the seam map that
makes the landing a config+key exercise, not a protocol rewrite. The
horizon we plan against: NIST IR 8547 / OMB M-26-15 / CNSA 2.0
(key-establishment migration complete by 2030-12-31; signatures 2031).
Sources: <https://csrc.nist.gov/pubs/ir/8547/final> ·
<https://nvlpubs.nist.gov/nistpubs/specialpublications/NIST.SP.1800-38B.pdf>

## Algorithm inventory

| Algorithm | Where (the real call sites) | What it protects | HNDL verdict | Swap path |
|---|---|---|---|---|
| Ed25519 | `ump_integrity` (record sigs §6.1), agent cards (`workflow/mesh.rs` provision/verify), parcels + standby manifests (`sign_manifest_bytes`), provenance marks (`provenance.rs`), capability tokens (`mint_capability_token`), `brain key sign` | Identity + integrity of UMP records, cards, parcels, standby manifests, boundary artifacts, tokens | NOT HNDL-exposed (integrity/authenticity, not confidentiality). The exposure is harvest-now-FORGE-later: a recorded signature must stay unforgeable for the artifact's whole evidentiary life (audit/DSAR evidence = years). | `did:key` multicodec prefix (below) + the dual-sign transition in `### UMP signatures` |
| HMAC-SHA256 | audit hash-chain links + hmac256 epoch (`audit/mod.rs`), Standard-Webhooks verify (`webhook.rs`), GitHub webhook verify, case-status tokens (`workflow/case_status.rs`), channels (`workflow/channels.rs`), observe series | Tamper-evidence of the audit chain; webhook authenticity; unguessable public status refs | NOT HNDL-exposed (verdicts, not secrets to decrypt). Grover halves effective strength to ~128 bits — comfortably above any near-term quantum margin. | New HMAC type alias in `webhook.rs` + `audit` epoch flip (the `--re-audit` re-anchor machinery already versioned the chain format) |
| SHA-256 | manifest digests (`kb.rs::manifest_json`, parcels), card signature message (`mesh.rs::sha256_hex`), provenance wrapper (`provenance::signed_message`), subject hashing (`outreach::hash_subject`) | Content-addressing, signatures' digest messages, one-way subject pseudonyms | NOT HNDL for pseudonyms (one-way by construction — no decrypt-later risk at any quantum speedup). Collision margin halves (~128 bits) — fine for digests of this size/life. | Digest-string conventions are isolated in the two `sha256_hex` helpers; a SHA-384/SHA3 bump is a typed swap per site |
| BLAKE3 | UMP record content hashes (`ump_integrity::record_hash` — the spec §2.8 mandated algorithm) | UMP content-addressed ids (`urn:ump:…`) | NOT HNDL (ids, not secrets). | The UMP spec owns this choice — a change is a spec revision + `content_hash_string` re-version, not a site-by-site migration |
| RS256/RS384/RS512, ES256/ES384, EdDSA | JWT/JWS verify (`auth/jwt.rs::ALLOWED_ALGS`, keys from `auth/jwks.rs` `BRAIN_JWT_KEY_DIR`) | Bearer identity (SSO) | Weakly HNDL-exposed: tokens are short-lived (15-min access ceiling) so recorded tokens age out; the LONG-lived exposure is the IdP's signing keys, not ours. | `### JWT: the ML-DSA landing procedure` below |
| AES-256-GCM | backup v3 blobs (`backup.rs`, header bytes as AAD), standby base + WAL chunks (`standby.rs` via `encrypt_v3_blob`) | Memory at rest (backups, follower copies) | THE HNDL surface of this product: a stolen archive stays decryptable-forever only while its passphrase holds — 256-bit keys carry ~128-bit post-quantum security (Grover), which is why the family was chosen. Verdict: KEEP; manage the passphrase, not the cipher. | `backup::encrypt_v3_blob` is the single writer; a cipher swap is a v4 header (the v2→v3 AAD fix is the precedent for a format bump) |
| Argon2id | backup/standby passphrase KDF (`backup.rs`: m=64 MiB, t=3, p=1, 32-byte out) | Turns the operator passphrase into the AES key | NOT HNDL (a KDF, not stored material). No practical quantum break known; parameters get a documented review at the 2030 horizon. | Parameters ride the v3 header and are bounds-checked on read — widening them is header-compatible; a KDF swap is a v4 format bump |

## HNDL exposure verdicts (the honest summary)

- **Nothing in brain-server is long-lived confidential ciphertext under a
  quantum-vulnerable primitive.** The only ciphertext at rest is
  AES-256-GCM (backups + standby chunks), whose 256-bit keys retain a
  ~128-bit post-quantum security margin under Grover — the classical
  symmetric recommendation CNSA 2.0 lands on. The verdict is KEEP.
- **The quantum-exposed class here is signatures, and the exposure is
  forgery-later, not decrypt-later.** Recorded Ed25519 signatures
  (audit-linked UMP records, signed cards, parcels, standby manifests,
  provenance marks) must remain unforgeable for the evidence's retention
  life. The mitigation is algorithm agility (below), deployed BEFORE any
  PQC-forgery capability exists — exactly what this seam map is for.
- **The classical-crypto ceiling is stated, not hidden:** until a PQC stack
  lands (JWT ML-DSA needs the IdP first — see the procedure), every
  signature in this system is classical. That is the Enterprise audit
  report's printed ceiling; this file is how it gets closed.

## Algorithm-agility seams

### JWT: the ML-DSA landing procedure (FIPS 204)

`src/auth/jwt.rs` is the single JWT verification entry point, and its
algorithm dispatch is already enum-isolated: `decode_header` reads the JOSE
`alg` BEFORE any key material is touched, the whitelist
(`ALLOWED_ALGS`) rejects everything unlisted, and `Validation::new` is
pinned per-token to the header's alg (no library-default HMAC confusion).
Landing ML-DSA is therefore:

1. **Key material:** ML-DSA public keys arrive as JWK (`kty` per the
   IETF JOSE PQC drafts) in the IdP's JWKS, or as files in
   `BRAIN_JWT_KEY_DIR` (`auth/jwks.rs` — add the ML-DSA loader beside the
   RSA/EC/Ed25519 ones). Key-dir + config work; no schema, no wire change.
2. **Whitelist:** add the algorithm to `ALLOWED_ALGS` in `auth/jwt.rs` —
   the ONE gate every token passes. The OWASP posture is unchanged: only
   the documented IdP's algorithm joins; `HS*`/`none` stay forbidden.
3. **Verifier:** if `jsonwebtoken` v10 gains the algorithm, this is a
   one-line `Algorithm` variant. If not, the two-phase design already
   gives the seam: `decode_header`'s alg field routes ML-DSA tokens to a
   dedicated verify fn (the same whitelist-then-key order, ML-DSA
   verification library beside the crate). The OWASP cheat-sheet contract
   (whitelist before key lookup, per-token `Validation`, `jti` required)
   is re-asserted by the existing test matrix, which is written against
   the seam, not the library.
4. **Rotation:** JWKS `kid` rotation is live machinery (1–3 keys during
   rotation). Dual-algorithm transition = serve RS256 + ML-DSA kids in
   parallel, retire RS256 kids after the IdP flips — no downtime, no
   token invalidation beyond the normal expiry.

The honest dependency: an IdP must issue ML-DSA tokens first. This
procedure is the receiver-side readiness, recorded before it is needed.

### UMP signatures: the algorithm version-prefix rule

Every UMP-family signature names its signer as a `did:key:z…` string whose
bytes are `multicodec varint || raw public key` (`ump_integrity::
did_key_from_ed25519` prefixes `0xed 0x01`, the registered Ed25519-pubkey
code; `verifying_key_from_did` REFUSES any other codec). The multicodec
table IS the algorithm version prefix:

- A future ML-DSA/SLH-DSA UMP signer lands as a NEW registered multicodec
  code with its own prefix bytes. `did:key` strings then self-describe
  their algorithm — old verifiers reject the unknown prefix (fail closed,
  the exact behavior `verifying_key_from_did` ships today), new verifiers
  dispatch on the prefix. No out-of-band algorithm negotiation, no
  ambiguity about which key made which signature.
- Records and manifests stay byte-compatible: the `integrity`/`signed_by`
  fields already carry the did string verbatim; the signature algorithm is
  a property of the DID, not a new field.
- The dual-sign transition for long-lived evidence: re-sign under BOTH the
  Ed25519 did and the PQC did during the migration window (parcels and
  standby manifests carry the manifest JSON, so a second signature block
  is additive); verify accepts either during the window, only the PQC did
  after cutover.
- Parcel manifests additionally carry the integer `version` field
  (`PARCEL_VERSION`, enforced on import) — a format-level escape hatch
  that stays reserved for changes the DID prefix cannot express.

### What this file does NOT claim

No PQC algorithm is deployed. No hybrid signature is deployed. The 2030
horizon is a planning input, not a deadline this codebase currently
meets for signatures; the inventory above is the measured starting point
it will be measured against.
