#!/usr/bin/env bash
# Build the Dioxus web bundle and deploy it to client/dist (what brain-server
# serves at /app). Run from this directory:  ./deploy-web.sh
#
# Two non-obvious pieces this script bakes in:
#  * --base-path /app so dx emits /app/assets/* URLs in index.html.
#  * A <link rel="stylesheet"> for the hashed tailwind CSS is injected into
#    index.html. dx would normally inject this at runtime, but the runtime
#    resolves it via the compile-time DIOXUS_ASSET_ROOT env var, which is baked
#    when the cached dioxus-cli-config crate was built — so it comes out empty
#    and the CSS link would point at /assets/... (auth 401). Injecting the link
#    with the concrete /app/assets/... URL sidesteps that. Re-run after every
#    rebuild; the hashed CSS filename changes on content change.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR"

dx bundle --platform web --release --base-path /app >/dev/null 2>&1 || true

SRC="target/dx/brain-client/release/web/public"
CSS="$(basename "$(ls "$SRC"/assets/tailwind-*.css | head -1)")"
[[ -n "$CSS" ]] || { echo "no tailwind css in bundle"; exit 1; }

rm -rf dist && mkdir -p dist/assets
cp "$SRC/index.html" dist/
# Resolve concrete filenames (not globs) — dx rebuilds target/ async-ish and a
# bare glob can race the copy, matching nothing mid-rebuild.
JS="$(basename "$(ls "$SRC"/assets/brain-client-*.js | head -1)")"
WASM="$(basename "$(ls "$SRC"/assets/brain-client_bg-*.wasm | head -1)")"
[[ -n "$JS" && -n "$WASM" ]] || { echo "no js/wasm in bundle"; exit 1; }
cp "$SRC/assets/$JS" "$SRC/assets/$WASM" dist/assets/
cp "$SRC/assets/$CSS" dist/assets/

python3 - dist/index.html "$CSS" <<'EOF'
import sys
idx, css = sys.argv[1], sys.argv[2]
html = open(idx).read()
link = f'    <link rel="stylesheet" href="/app/assets/{css}" type="text/css">'
if 'rel="stylesheet"' not in html:
    html = html.replace('</head>', link + '\n    </head>')
open(idx, 'w').write(html)
EOF

echo "deployed to dist/ (tailwind: $CSS)"
