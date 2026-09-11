# Signed parcels (memory that travels)

A parcel moves reviewed memory between domains or machines without
re-typing it. Export signs a manifest over the rows; import verifies the
signature before writing anything, and whatever arrives still lands as
pending proposals for a human. A parcel never promotes by itself.

## HTTP

- `POST /parcels/export` (Admin on domain) ships only promoted,
  non-quarantined rows. Body `{domain, since?}`. Returns the parcel
  (`manifest`, `signature`, `signed_by`), its hash, source domain, region,
  and row count. Refuses with `parcel_too_large` or `operator_key_missing`
  (no key means no signature, and unsigned export is not offered).
- `POST /parcels/import` verifies before writing: the parcel signature,
  the named counterparty, and the size. Body always includes
  `expected_signer`; missing means `400 signer_required`, wrong means
  `400 signer_mismatch`, and naming your own key for someone else's parcel
  means `409 signer_alias`. Accepted rows land pending, deduplicated by
  content hash and injection-screened (the screened count is reported).
- `GET /parcels` (Read) pages the ledger: direction in or out, parcel
  hash, signer, row count, reviewer, timestamp.

## CLI

- `brain parcel export --domain <d> [--since <ts>] --out <file>`
- `brain parcel import --file <file> --domain <d>
  --expected-signer <did>`
- `brain parcel ledger [--domain <d>]`

## Governance

Every import and export writes a ledger row plus a hash-chained audit row
in the same transaction. The ledger answers "who sent what to whom" long
after the fact; combined with the approval digest on the receiving side,
it closes the loop between transport trust (the signature) and content
trust (the human).
