# Dioxus WASM Split — Research Findings (2026-08-09)

**Question:** Can the latest Dioxus (specifically the asked-about "0.8.1") do a
**split bundle** (wasm-split / code-splitting the wasm binary into lazily-loaded
chunks)?

**Short answer:** The premise is wrong — **there is no stable Dioxus 0.8.1.**
The latest stable is **0.7.10** (which this project already pins). The `wasm-split`
feature **does exist in 0.7.10** (both `dioxus` and `dioxus-router` ship a
`wasm-split` cargo feature), but it is **experimental** and gated behind an
experimental CLI flag. The 0.8 line exists only as `0.8.0-alpha.0` / `0.8.0-alpha.1`
— not production-stable.

## Version reality (verified against crates.io, 2026-08-09)

| Crate | Max stable | 0.8 line |
|---|---|---|
| `dioxus` | **0.7.10** | only `0.8.0-alpha.0`, `0.8.0-alpha.1` |
| `dioxus-router` | **0.7.10** | only `0.8.0-alpha.x` |
| `dioxus-cli` (local `dx`) | 0.7.10 | — |

So "Dioxus 0.8.1" does not exist as a stable release. There is nothing to
upgrade to that resolves the bundle-size ceiling today.

## wasm-split in the current stable line (0.7.10)

- **Both** `dioxus` and `dioxus-router` expose a `wasm-split` cargo feature
  (verified on crates.io). Enabling it is done via:
  ```toml
  dioxus = { version = "0.7", features = ["router", "wasm-split"] }
  dioxus-router = { version = "0.7", features = ["wasm-split"] }
  ```
- The installed `dx` (0.7.10) exposes an **experimental** flag:
  `dx bundle --experimental-wasm-split` (a.k.a. `--wasm-split`), documented as
  "Bundle split the wasm binary into multiple chunks based on `#[wasm_split]`".
- The splitter is **route-variant-driven**: it slices the router's route
  components into separate chunks loaded on navigation, using a
  `#[wasm_split(...)]` macro (or `dioxus-router?/wasm-split` at bundle time).

### Why we have NOT enabled it (the honest ceilings, re-verified)

1. **It is experimental.** The Dioxus docs/CLI consistently mark it
   `--experimental-wasm-split`. The wasm-split tooling lives in a sub-workspace
   (`packages/wasm-split`) and ships only pre-1.0 alpha versions
   (`wasm-split-cli` 0.7.0-alpha.x on lib.rs). No stable/SemVer-guaranteed release.
2. **It disconnects the call graph.** From the official docs: "Enabling splitting
   disconnects the call graph, meaning if you try to run your app with a normal
   `dx serve`, it won't work." It becomes a build-only mode that a plain `dx serve`
   can't run. Our workflow relies on `dx bundle` + plain serving; adopting it
   forks dev vs. build behavior.
3. **It requires router-wide refactoring.** Route variants must be split with the
   `#[wasm_split]` macro + a `SuspenseBoundary` above the `<Outlet>`. Our client
   has 12 panels under one `AppShell` layout; slicing them out cleanly (and
   keeping the shared `ApiClient`/`UiState` contexts, the command palette, and the
   connect-first flow working across chunk boundaries) is real, error-prone work.
4. **Suspense/async across split chunks** interacts with our `use_resource`-driven
   panels and the keyring/localStorage seams — a regression surface we don't
   currently have coverage for (73 tests, none exercise cross-chunk navigation).
5. **No measured win on this codebase.** The current single wasm is **3.7 MB**
   (`brain-client_bg-*.wasm`). Splitting routes could cut initial parse/compile,
   but our heaviest dependency (the static embedding-independent client) is shared
   shell code; the actual per-panel delta is small. Until we measure it, enabling
   splitting is speculative optimization.

## Recommendation

- **Do NOT adopt wasm-split now.** The stable version (0.7.10) is what we already
  run; "0.8.1" doesn't exist. The feature is experimental, build-only, and
  router-refactor-heavy for no measured payoff.
- **Track it** for when (a) Dioxus ships a stable 0.8.0+ with wasm-split
  non-experimental, and (b) we measure that initial-load parse time is actually a
  bottleneck (the bundle is served from `/app` on a local edge device).
- **Keep the bundle single-file** for now; if initial-load latency becomes a
  problem, revisit after Dioxus 0.8 stable.

## Sources
- crates.io API (max_stable_version for `dioxus`, `dioxus-router`; feature lists for 0.7.10).
- Dioxus docs (learn site + `packages/router/README.md` + `packages/wasm-split/README.md` + DeepWiki WASM Code Splitting).
- Local `dx --version` + `dx bundle --help`.
