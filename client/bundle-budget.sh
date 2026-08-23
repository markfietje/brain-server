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
wasm=""
for f in target/wasm32-unknown-unknown/release/*.wasm; do
  [ -e "$f" ] || continue
  if [ -z "$wasm" ] || [ "$f" -nt "$wasm" ]; then wasm="$f"; fi
done
if [ -z "$wasm" ]; then echo "no release wasm found under target/wasm32-unknown-unknown"; exit 1; fi
size=$(stat -f%z "$wasm" 2>/dev/null || stat -c%s "$wasm")
# v1.28.21: the build now keeps the name section + relocation records
# (client/.cargo/config.toml) so `dx build --wasm-split` can partition the
# binary — the RAW artifact therefore overstates the shipped posture. dx's
# shipped wasm has custom sections (name/linking/reloc.*) stripped by
# wasm-opt; mirror that here by measuring with those sections removed.
# Pure section-frame walk: id byte + LEB128 size per section; drop custom
# sections whose name starts with a known toolchain marker.
shipped=$(python3 - "$wasm" <<'PY'
import sys

def leb(buf, i):
    r = s = 0
    while True:
        b = buf[i]; i += 1
        r |= (b & 0x7F) << s
        if not b & 0x80:
            return r, i
        s += 7

data = bytearray(open(sys.argv[1], "rb").read())
i = 8  # magic + version
out = bytearray(data[:i])
drop = ("name", "linking", "reloc.", "target_features", "producers")
while i < len(data):
    start = i
    sec_id = data[i]; i += 1
    n, i = leb(data, i)
    body_start = i
    end = i + n
    if sec_id == 0:  # custom
        ln, j = leb(data, i)
        nm = bytes(data[j:j + ln]).decode("utf-8", "replace")
        if any(nm == d or nm.startswith(d) for d in drop):
            i = end
            continue
    out += data[start:end]
    i = end
print(len(out))
PY
)
echo "wasm raw: $size bytes ($((size / 1024)) KB)"
echo "wasm shipped-posture (custom sections stripped): $shipped bytes ($((shipped / 1024)) KB) — budget $budget bytes"
if [ "$shipped" -gt "$budget" ]; then
  echo "BUDGET BREACH — the release wasm outgrew its $budget-byte budget."
  exit 1
fi

# v1.28.20 Cockpit M4: the wasm dependency graph must stay tokio-runtime-free.
# tokio is declared sync-only; a feature creep that pulls rt into the wasm
# graph would silently grow the bundle and the concurrency surface. Normal
# edges only — dev-deps legitimately use the runtime for tests.
if cargo tree --target wasm32-unknown-unknown -e normal -p brain-client 2>/dev/null | grep -q "tokio"; then
  if cargo tree --target wasm32-unknown-unknown -e normal,features -p brain-client 2>/dev/null | grep -qE 'tokio feature "(rt|macros|full|default)"'; then
    echo "TOKIO CREEP — the wasm graph pulled in the tokio runtime; keep tokio sync-only on web."
    exit 1
  fi
fi

# Desktop binary budget (generous but finite — a full webview shell). Checked
# only when a desktop target dir exists (the operator's build-desktop.sh run);
# CI compiles `--features desktop` for correctness, not size.
desktop_budget=41943040 # 40 MiB
for f in target/release/bundle/macos/*.app/Contents/MacOS/* target/release/brain-client target/x86_64-unknown-linux-gnu/release/brain-client; do
  [ -e "$f" ] || continue
  dsize=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f")
  echo "desktop: $dsize bytes ($((dsize / 1024)) KB) — budget $desktop_budget bytes ($f)"
  if [ "$dsize" -gt "$desktop_budget" ]; then
    echo "DESKTOP BUDGET BREACH — $f outgrew $desktop_budget bytes."
    exit 1
  fi
done