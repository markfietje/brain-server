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

# 5b. Fork-field patch table. Fields the FORK workspace owns by declaration —
# the canonical tree's values are wrong for the deployed extension and the
# rsync must not get to keep them. (K7-03: the mirror-sync overwrote the
# fork's typebox truth repair with the canonical 1.3.26 manifest while the
# workspace catalog/lock said 1.3.27 — a misstating manifest plus a
# `--frozen-lockfile` mismatch, in one silent copy.) Each row: file, field,
# source of truth. The patch runs AFTER rsync; the byte-identity check below
# then verifies the file's delta is EXACTLY these fields, and the post-check
# (5c) pins manifest == lock forever.
FORK_WS="$OC_DIR/pnpm-workspace.yaml"
FORK_LOCK="$OC_DIR/pnpm-lock.yaml"

patch_fork_fields() {
	local manifest="$TARGET/package.json"
	local want
	# Fork catalog truth: the workspace-level typebox pin.
	want="$(awk '/^[[:space:]]+typebox:/ {print $2; exit}' "$FORK_WS")"
	if [[ -z "$want" ]]; then
		echo "fork-field patch: no typebox entry found in $FORK_WS — refusing (fail-closed)" >&2
		exit 1
	fi
	# Line-targeted rewrite only — the rest of the manifest must stay
	# byte-identical to the canonical tree (formatting included).
	if ! LC_ALL=C sed -i '' "s/^\([[:space:]]*\"typebox\": \)\"[^\"]*\"$/\1\"$want\"/" "$manifest"; then
		echo "fork-field patch: sed rewrite of $manifest failed" >&2
		exit 1
	fi
	if ! grep -q "\"typebox\": \"$want\"" "$manifest"; then
		echo "fork-field patch: typebox line not found/rewritten in $manifest — refusing" >&2
		exit 1
	fi
	echo ">> fork-field patch: package.json dependencies.typebox <- $want (fork workspace catalog truth)"
}

# 5c. The fork-pair pin: the extension manifest's declared specifier must
# equal the lockfile's recorded specifier. This is the mechanical check the
# 0.6.8 sync regression would have failed (K7-03 — red-first demonstrated
# 2026-09-14: manifest 1.3.26 vs lock 1.3.27); it runs in THIS repo and
# enforces the pair forever.
check_manifest_matches_lock() {
	local manifest_typebox lock_spec
	manifest_typebox="$(sed -n 's/.*"typebox": "\([^"]*\)".*/\1/p' "$TARGET/package.json" | head -1)"
	lock_spec="$(awk '
		/^  extensions\/brain-server:$/ { inblk = 1; next }
		inblk && /^  [^ ]/ { exit }
		inblk && /^      typebox:/ { getline; sub(/^[[:space:]]*specifier:[[:space:]]*/, ""); print; exit }
	' "$FORK_LOCK")"
	if [[ -z "$manifest_typebox" || -z "$lock_spec" || "$manifest_typebox" != "$lock_spec" ]]; then
		echo "SYNC UNVERIFIED: extension manifest typebox ('${manifest_typebox:-absent}') != lockfile specifier ('${lock_spec:-absent}')" >&2
		echo "  the manifest misstates what runs and --frozen-lockfile will refuse — fix the pair, then re-run" >&2
		exit 1
	fi
	echo ">> extension manifest matches lock specifier ($lock_spec)"
}

if [[ -f "$FORK_WS" && -f "$FORK_LOCK" ]]; then
	patch_fork_fields
	check_manifest_matches_lock
else
	echo "SYNC UNVERIFIED: fork workspace files missing ($FORK_WS / $FORK_LOCK)" >&2
	exit 1
fi

# 6. Post-sync byte-identity check, fail-closed, modulo the DECLARED
# exception list below. Excludes mirror the rsync set (plus macOS
# metadata); --delete above means anything else must match. An exception
# is not a pass: each entry names its file + reason, and the check then
# verifies the declared delta is EXACTLY the reason (anything more fails).
#   - format.test.ts: the fork workspace's import-order formatter reorders
#     the header imports; the delta must be import-lines-only (verified by
#     diffing with import lines stripped) — the test bodies stay
#     byte-identical, so test-count parity is structural.
#   - package.json: the fork-field patch table (5b) owns the typebox
#     specifier; the delta must be typebox-lines-only.
DIFF_OUT="$(diff -rq "$SRC" "$TARGET" -x node_modules -x package-lock.json -x .DS_Store || true)"
DECLARED_FILES=()
if [[ -n "$DIFF_OUT" ]]; then
	DECLARED_FILES=("format.test.ts" "package.json")
	UNDECLARED="$(printf '%s\n' "$DIFF_OUT" | grep -v -e 'format.test.ts' -e 'package.json' || true)"
	if [[ -n "$UNDECLARED" ]]; then
		echo "SYNC UNVERIFIED: drift outside the declared exception list (format.test.ts, package.json):" >&2
		printf '%s\n' "$UNDECLARED" >&2
		exit 1
	fi
	if ! diff <(grep -v '^import' "$SRC/src/format.test.ts") \
			<(grep -v '^import' "$TARGET/src/format.test.ts") >/dev/null; then
		echo "SYNC UNVERIFIED: format.test.ts differs beyond import order —" >&2
		echo "  merge the change into the canonical tree and re-sync" >&2
		exit 1
	fi
	if ! diff <(grep -v '"typebox"' "$SRC/package.json") \
			<(grep -v '"typebox"' "$TARGET/package.json") >/dev/null; then
		echo "SYNC UNVERIFIED: package.json differs beyond the fork-field patch table —" >&2
		echo "  only dependencies.typebox may differ; merge the rest into the canonical tree" >&2
		exit 1
	fi
	echo ">> declared deltas verified: format.test.ts (import order), package.json (typebox specifier only)"
fi

git -C "$REPO" rev-parse HEAD > "$BASELINE_FILE"
echo "synced $SRC -> $TARGET (byte-identical modulo declared deltas; baseline updated)"
echo "next: cd $OC_DIR && pnpm install --frozen-lockfile \\"
echo "      && node_modules/.bin/vitest run extensions/brain-server/test \\"
echo "      && node_modules/.bin/tsc --noEmit -p extensions/brain-server/tsconfig.json, then commit the synced tree."
