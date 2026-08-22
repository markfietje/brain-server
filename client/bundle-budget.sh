#!/usr/bin/env bash
# v1.20.0 M2.1 — bundle regression budget for the web client.
# CI cannot run `dx bundle` (the Dioxus CLI is not on the runners), so this
# guards the dominant term — the release WASM binary — against unbounded
# growth. Measured release wasm at v1.20.0: 4.34 MB (the dx-bundle artifact
# with wasm-opt is 3.7 MB, v1.18.1). v1.28.4 tightened the cap to 5.5 MiB
# (5,734,400 bytes) — the plan's ceiling for the shell + conversation engine;
# the script fails CI on regression.
set -euo pipefail
cd "$(dirname "$0")"
budget=5734400
cargo build --release --target wasm32-unknown-unknown --quiet
wasm=$(ls -t target/wasm32-unknown-unknown/release/*.wasm | head -1)
size=$(stat -f%z "$wasm" 2>/dev/null || stat -c%s "$wasm")
echo "wasm: $size bytes ($((size / 1024)) KB) — budget $budget bytes"
if [ "$size" -gt "$budget" ]; then
  echo "BUDGET BREACH — the release wasm outgrew its $budget-byte budget; wasm-split (Dioxus 0.8) is the planned release valve."
  exit 1
fi