# Dioxus WASM Split — Research Findings (2026-08-09, **updated 2026-08-25**)

**Question:** Can Dioxus do a **split bundle** (wasm-split / code-splitting the
wasm binary into lazily-loaded chunks)?

**Short answer (then):** No stable path — 0.8 didn't exist, the feature was
experimental, and there was no measured win. Recommendation was *do not adopt*.

**Short answer (now):** The situation inverted. The bundle outgrew its budget
posture, so we moved onto the `0.8.0-alpha.1` line deliberately and
**`dx build --wasm-split` is enabled and green** (since v1.28.21). The
remaining work is annotating real lazy boundaries — the splitter runs today but
nothing earns a second chunk yet.

## Version reality (re-verified against crates.io, 2026-08-25)

| Crate | Max stable | Alpha line | We pin |
|---|---|---|---|
| `dioxus` | **0.7.10** | `0.8.0-alpha.1` | **`=0.8.0-alpha.1`** |
| `dioxus-router` | 0.7.x | 0.8.0-alpha.x | (via `dioxus/router`) |

The client deliberately rides the alpha: wasm-split tooling is where the 0.8
line lives, and the alternative was an over-budget single blob. This is a
conscious trade — pin exact (`=`), accept pre-1.0 churn, and let
`Cargo.lock` + CI gate every bump.

## What actually shipped (v1.28.20–.21)

1. **Split-compatible build config** (`client/.cargo/config.toml`): the
   splitter needs function names AND relocation records to partition the
   binary. The old `strip=symbols` erased the name section and wasm-split died
   with *"Failed to find `main` function"*. Now:
   `-C strip=debuginfo` (drops only DWARF — the size bulk) +
   `-C link-arg=--emit-relocs`.
2. **`dx build --platform web --release --wasm-split` is the shipped path**,
   verified green. Without annotated boundaries it emits main + one empty
   chunk — zero behavioral change, zero risk, infrastructure proven.
3. **Budget law rewritten for the split posture** (`client/bundle-budget.sh`,
   enforced in CI): the raw cargo artifact now legitimately carries splitter
   metadata (name/linking/reloc.* custom sections), so the gate measures the
   **shipped posture** — those sections stripped by a pure section-frame walk,
   mirroring dx's wasm-opt pass. Budget stays **5.5 MiB**; a breach fails CI.
   Current numbers: raw ≈ 12.2 MiB → shipped-posture ≈ 4.0 MiB (under budget).
4. **Tokio-creep guard**: the wasm dependency graph must stay runtime-free
   (`tokio` sync-only on web) — a size AND concurrency-surface guard riding the
   same script.

## Why we originally said no — and what changed

| Ceiling (2026-08-09) | Status now |
|---|---|
| Experimental, no stable release | Still true — accepted deliberately; pinned exact + locked |
| Disconnects the call graph / build-only | Solved operationally: rustflags keep the splitter fed; `dx build --wasm-split` is the documented shipped path in `Dioxus.toml` |
| Router-wide refactoring risk | **Deferred, not solved** — no `#[wasm_split]` boundaries are annotated yet, so no route slicing has happened |
| No measured win | Still unproven per-chunk; what forced the flip was the raw artifact's growth, not a parse-time benchmark |

The honest driver: this was not premature optimization. The single wasm was
pushing the ceiling, and the split toolchain was the escape hatch that lets the
shell grow without paying full price up front.

## Remaining follow-ups

1. **Annotate lazy boundaries** with `#[wasm_split(...)]` on genuinely heavy
   panels (candidates: Graph, Cockpit conversation view) + a
   `SuspenseBoundary` above the `<Outlet>`. Rule of thumb from this exercise:
   annotate only when a second module *earns its fetch*.
2. **Measure** initial parse/compile before/after each annotation — the win is
   a hypothesis until then (the app is served from `/app` on a local edge
   device, so latency pressure is mild).
3. **Track Dioxus stable**: when 0.8.0 goes stable with wasm-split
   non-experimental, drop the alpha pin.

## Sources
- crates.io API (max_stable_version / newest for `dioxus`, re-checked 2026-08-25).
- `client/Cargo.toml` (pin), `client/.cargo/config.toml` (split-compatible rustflags),
  `client/Dioxus.toml` (shipped build command), `client/bundle-budget.sh`
  (shipped-posture measurement + tokio guard).
- Commit `4f9a303` "build(client): enable wasm-split — keep names+relocs, budget reads shipped posture".
