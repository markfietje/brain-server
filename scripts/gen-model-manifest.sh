#!/usr/bin/env bash
# gen-model-manifest.sh — emit a BRAIN_MODEL_MANIFEST file for the local model
# artifacts (.fastembed_cache/ + models/). The manifest is a flat JSON object
# mapping file path (relative to the manifest's own directory) → SHA-256 hex;
# the server verifies every entry at boot and refuses on any mismatch.
#
# Usage: gen-model-manifest.sh [OUT] [ROOT...]
#   OUT   defaults to ~/.config/brain-server/model-manifest.json
#   ROOT  dirs to walk; defaults to "$HOME/.fastembed_cache" and ./models
set -euo pipefail

OUT="${1:-$HOME/.config/brain-server/model-manifest.json}"
shift || true
ROOTS=("$@")
[[ ${#ROOTS[@]} -eq 0 ]] && ROOTS=("$HOME/.fastembed_cache" "models")

mkdir -p "$(dirname "$OUT")"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
printf '{\n' > "$OUT.tmp"
first=1
for root in "${ROOTS[@]}"; do
	[[ -d "$root" ]] || continue
	while IFS= read -r -d '' f; do
		sha=$(shasum -a 256 "$f" | awk '{print $1}')
		if [[ -n "$(dirname "$f")" && ! -f "$(cd "$(dirname "$f")" && pwd)/$(basename "$f")" ]]; then
			continue
		fi
		rel=$(python3 -c "import json,sys;print(json.dumps(sys.argv[1]))" "$f")
		[[ $first = 1 ]] || printf ',\n' >> "$OUT.tmp"
		first=0
		printf '  %s: "%s"' "$rel" "$sha" >> "$OUT.tmp"
	done < <(find "$root" -type f -print0 | sort -z)
done
printf '\n}\n' >> "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
chmod 600 "$OUT"
echo "model manifest written: $OUT"
