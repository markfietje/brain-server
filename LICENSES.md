# LICENSES — what ships under which licence

**Purpose.** Every distribution surface — the container image, the signed
desktop shell, the npm plugin — carries this disclosure so a redistributor
can honour each component's terms from one file. It ships in the image at
`/usr/share/doc/brain-server/` next to `LICENSE` and `NOTICE`, and its
presence is machine-checked by the Spire `license_header_present` pin.

| Component | Ships in | Licence | Source |
|---|---|---|---|
| `brain-server`, `brain`, `mcp`, `brain-connector-stub` | image · shell sidecar | Apache-2.0 (`LICENSE` + `NOTICE`) | this repo |
| desktop shell (Tauri) | macOS/Windows updater | Apache-2.0 | this repo |
| `plugin` | npm / ClawHub | Apache-2.0 | this repo |
| `signal-gateway` | optional, added at deployment | AGPL-3.0-only | this repo |
| ↳ `presage`, `presage-store-sqlite` (rev `6f630791`) | linked into signal-gateway | AGPL-3.0-only | whisperfish/presage |
| ↳ `libsignal-service` (rev `fae0679d`) | linked into signal-gateway | AGPL-3.0-only | whisperfish/libsignal-service-rs |
| ↳ `libsignal` (tag v0.99.0, workspace of 36 crates) | linked into signal-gateway | AGPL-3.0-only | signalapp/libsignal |

The pinned revs are the licence ground truth, verified in the vendored
checkouts on 2026-10-27 (the 2019 Signal blog's GPLv3 + App-Store
permission covers `libsignal-protocol-c` only, not this Rust stack). A rev
bump re-opens the verification: reading the stack's `license` fields is
part of the deliberate bump ritual the `signal-gateway` manifest describes.

## The three standing rules this table encodes

1. **The process boundary is the licence boundary.** The AGPL stack lives
   only inside the `signal-gateway` process: no AGPL crate links into the
   Apache binaries, no AGPL source is copied into Apache files, and the
   gateway never imports private-repo code. The programs talk over
   loopback HTTP at arm's length — separate works, not one combined work —
   pinned by `bridge_holds_no_brain_credentials`.
2. **Gateway changes stay public.** AGPL §13 gives every user of a
   distributed or network-served binary a source right. The public repo
   satisfies it, and keeps satisfying it only while every gateway
   modification lands here — a private gateway fork forfeits the shield.
3. **No App Store for a Signal-bearing shell.** Apple's MAS terms conflict
   with the GPL family and the AGPL Rust stack carries no App-Store
   permission. Developer ID plus the direct updater is the sanctioned
   channel.
