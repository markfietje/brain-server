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
# Pick the FRESHEST tailwind build. dx leaves stale hashed CSS in target/ between
# rebuilds; a bare `ls | head -1` glob races it and can copy an old stylesheet
# (matching the stale-JS bug this script already fixes for the JS name). The
# newest mtime IS the just-completed `dx bundle` output.
CSS="$(basename "$(ls -t "$SRC"/assets/tailwind-*.css | head -1)")"
[[ -n "$CSS" ]] || { echo "no tailwind css in bundle"; exit 1; }

rm -rf dist && mkdir -p dist/assets
cp "$SRC/index.html" dist/
cp pwa/manifest.webmanifest dist/manifest.webmanifest
cp pwa/sw.js dist/sw.js
# Resolve the JS name from the freshly-written index.html itself (NOT a glob —
# dx leaves stale hashed assets in target/ between rebuilds and a bare glob
# races it, matching an old build while index.html references the new one).
# The WASM name is inside the JS (loaded via import), so derive it from there.
JS="$(grep -oE 'assets/brain-client-[^"]+\.js' "$SRC/index.html" | head -1 | xargs basename)"
[[ -n "$JS" ]] || { echo "no js in index.html"; exit 1; }
cp "$SRC/assets/$JS" dist/assets/
WASM="$(grep -oE 'brain-client_bg-[^"]+\.wasm' "$SRC/assets/$JS" | head -1)"
[[ -n "$WASM" ]] || { echo "no wasm in $JS"; exit 1; }
cp "$SRC/assets/$WASM" dist/assets/
cp "$SRC/assets/$CSS" dist/assets/

python3 - dist/index.html "$CSS" <<'EOF'
import sys
idx, css = sys.argv[1], sys.argv[2]
html = open(idx).read()
css_link = f'    <link rel="stylesheet" href="/app/assets/{css}" type="text/css">'
manifest = '    <link rel="manifest" href="/app/manifest.webmanifest">'
theme = '    <meta name="theme-color" content="#0b0d10">'
sw_reg = ('    <script>\n'
          '        if ("serviceWorker" in navigator) {\n'
          '            window.addEventListener("load", () =>\n'
          '                navigator.serviceWorker.register("/app/sw.js"));\n'
          '        }\n'
          '    </script>')
# M7.6 RTL: dir="auto" lets the browser choose LTR/RTL per element content,
# so memory text (e.g. Arabic/Hebrew notes) flows correctly while the shell
# stays LTR — no JS, no i18n extraction (that's a v2.x concern).
if 'dir="auto"' not in html and '<html' in html:
    html = html.replace('<html', '<html dir="auto"', 1)
if 'rel="stylesheet"' not in html:
    html = html.replace('</head>', css_link + '\n    </head>')
if 'manifest.webmanifest' not in html:
    html = html.replace('</head>', manifest + '\n    </head>')
if 'theme-color' not in html:
    html = html.replace('</head>', theme + '\n    </head>')
if 'sw.js' not in html:
    html = html.replace('</body>', sw_reg + '\n    </body>')
open(idx, 'w').write(html)
EOF

echo "deployed to dist/ (tailwind: $CSS)"
