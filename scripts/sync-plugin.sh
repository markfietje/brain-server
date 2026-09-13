#!/usr/bin/env bash
# sync-plugin.sh — the Parity discipline: rsync plugin/ (the canonical edit
# surface) into the openclaw workspace's deployed extension, with a real
# pre-sync drift guard and a post-sync byte-identity check. Both enforced,
# both fail-closed.
#
# Drift model: plugin/ is canonical. Between syncs, the deployed extension
# must not move on its own. A baseline file records the brain-server commit
# whose tree the target last matched; at sync time every changed file is
# three-way checked (baseline blob vs target HEAD blob). Files the target
# changed independently REFUSE the sync — a human merges, then re-runs.
#
# Usage: scripts/sync-plugin.sh [--dry-run] [OPENCLAW_DIR]   (default ~/Sites/openclaw)
set -euo pipefail

DRY_RUN=0
if [[ "${1:-}" == "--dry-run" ]]; then
	DRY_RUN=1
	shift
fi

REPO="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$REPO/plugin"
TARGET="${1:-$HOME/Sites/openclaw}/extensions/brain-server"
EXT_DIR="$(dirname "$TARGET")"
EXT_BASE="$(basename "$TARGET")"
# OC_DIR is the openclaw WORKSPACE root (parent of extensions/), where
# node_modules/.bin/oxfmt lives — not extensions/ itself.
OC_DIR="$(dirname "$EXT_DIR")"
OXFMT="$OC_DIR/node_modules/.bin/oxfmt"
BASELINE_FILE="$REPO/.plugin-sync-baseline"

[[ -d "$TARGET" ]] || { echo "target missing: $TARGET" >&2; exit 2; }

# 1. The canonical surface must be committed — never propagate worktree drafts.
if [[ -n "$(git -C "$REPO" status --porcelain -- plugin)" ]]; then
	echo "refusing: plugin/ has uncommitted changes — commit first, then sync" >&2
	exit 1
fi

# 2. Format the canonical surface with the SAME formatter the openclaw
# workspace applies on commit (oxfmt) — otherwise its pre-commit hook
# re-wraps the synced files and byte-identity drifts again (the 0.5.0
# lesson). Formatting is a source change: if it rewrites anything, STOP so
# the format pass lands as its own commit instead of riding the sync out.
if [[ -x "$OXFMT" ]]; then
	echo ">> formatting canonical plugin/ with the openclaw workspace's oxfmt…"
	(cd "$REPO" && find plugin -type f \( -name '*.ts' -o -name '*.md' -o -name '*.json' \) \
		-not -path 'plugin/node_modules/*' -print0 | xargs -0 "$OXFMT" --write)
	if [[ -n "$(git -C "$REPO" status --porcelain -- plugin)" ]]; then
		echo "refusing: oxfmt rewrote plugin/ — commit that format pass, then re-run" >&2
		exit 1
	fi
else
	echo ">> no oxfmt at $OXFMT — skipping format (byte-identity may drift on commit)" >&2
fi

# 3. Target worktree must be clean — uncommitted target edits never get
# silently absorbed.
if [[ -n "$(git -C "$EXT_DIR" status --porcelain -- "$EXT_BASE")" ]]; then
	echo "refusing: $TARGET has uncommitted changes — commit or stash first" >&2
	exit 1
fi

RSYNC_EXCLUDES=(--exclude node_modules --exclude package-lock.json)

# 4. Committed-drift guard: every file rsync would touch must match its
# baseline blob in the target's HEAD (i.e. the target did not move on its
# own since the last sync). No baseline yet (fresh clone, first run):
# initialize without guarding, loudly.
BASE_BRAIN=""
if [[ -f "$BASELINE_FILE" ]]; then
	BASE_BRAIN="$(cat "$BASELINE_FILE")"
fi
if [[ -z "$BASE_BRAIN" ]] || ! git -C "$REPO" cat-file -e "$BASE_BRAIN^{commit}" 2>/dev/null; then
	echo ">> no usable baseline ($BASELINE_FILE) — initializing without drift guard" >&2
	echo "   verify the target is faithful NOW; every future sync is guarded." >&2
	BASE_BRAIN=""
fi

CHANGED="$(rsync -rni --delete "${RSYNC_EXCLUDES[@]}" "$SRC/" "$TARGET/" \
	| sed -e 's/^[^ ]* //' -e 's/ -> .*//' | grep -v '/$' || true)"

DRIFTED=0
if [[ -n "$BASE_BRAIN" && -n "$CHANGED" ]]; then
	while IFS= read -r f; do
		old_exists=0; head_exists=0
		git -C "$REPO" cat-file -e "$BASE_BRAIN:plugin/$f" 2>/dev/null && old_exists=1 || true
		git -C "$EXT_DIR" cat-file -e "HEAD:./$EXT_BASE/$f" 2>/dev/null && head_exists=1 || true
		if [[ "$old_exists" == 1 && "$head_exists" == 1 ]]; then
			if ! cmp -s <(git -C "$REPO" show "$BASE_BRAIN:plugin/$f") \
				<(git -C "$EXT_DIR" show "HEAD:./$EXT_BASE/$f"); then
				echo "drifted (target moved independently): $f" >&2
				DRIFTED=1
			fi
		elif [[ "$old_exists" == 0 && "$head_exists" == 1 ]]; then
			# Absent at baseline, present in target HEAD: added on the
			# target side (or rsync-excluded) — not ours to touch.
			echo "drifted (target-side addition): $f" >&2
			DRIFTED=1
		fi
		# old absent + head absent: pure plugin-side addition — safe.
		# head absent + rsync deleting: file vanishes on both sides — safe.
	done <<< "$CHANGED"
	if [[ "$DRIFTED" == 1 ]]; then
		echo "refusing: target carries committed drift — merge it into plugin/ (or reset the target), then re-run" >&2
		exit 1
	fi
fi

if [[ -z "$CHANGED" ]]; then
	echo "already in sync — nothing to do"
else
	echo "$CHANGED" | while IFS= read -r line; do echo "  sync: $line"; done
fi

if [[ "$DRY_RUN" == 1 ]]; then
	echo "dry run — no changes made, baseline untouched"
	exit 0
fi

# 5. Live sync (only reached with a clean guard).
rsync -rc --delete "${RSYNC_EXCLUDES[@]}" "$SRC/" "$TARGET/"

# 6. Post-sync byte-identity check, fail-closed. Excludes mirror the rsync
# set (plus macOS metadata); --delete above means anything else must match.
if ! diff -rq "$SRC" "$TARGET" -x node_modules -x package-lock.json -x .DS_Store; then
	echo "SYNC UNVERIFIED: post-sync trees differ — investigate before committing the target" >&2
	exit 1
fi

git -C "$REPO" rev-parse HEAD > "$BASELINE_FILE"
echo "synced $SRC -> $TARGET (byte-identical; baseline updated)"
echo "next: cd $OC_DIR && node_modules/.bin/vitest run extensions/brain-server/test && \\"
echo "      node_modules/.bin/tsc --noEmit -p extensions/brain-server/tsconfig.json, then commit the synced tree."
