import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { shellRoot } from './fixtures';

/**
 * II-7e — `prewarm`: the OBLIGATION, pinned so the omission cannot be forgotten
 * silently.
 *
 * ## WHAT PLAN 02 §7 ASKS FOR, AND WHY IT CANNOT BE DONE
 *
 * Plan 02 §7's exit criterion: *"First delta paints without a blocking spinner
 * (prewarm wired)"*, with a measured ~2.0 s cold against ~0.7 s warm. It is a
 * real criterion about a real latency, and it is written against a **model
 * session**.
 *
 * **THERE IS NO MODEL SESSION IN THE SHIPPED APP.** II-1 decided the seam stays
 * a seam: no Swift host fills `KernelRecallTransport`, the webview holds the only
 * wire, and the functional consequence — the on-device model's tool has no live
 * transport — is recorded in the register's transport-decision row. So a turn
 * today is RETRIEVAL plus its evidence. There are no model deltas to stream and
 * no session to prewarm.
 *
 * That is also why Plan 02 §4's "model unavailable degrades to working
 * retrieval" is not a fallback here: it is the ONLY mode.
 *
 * ## WHY THIS IS A PIN AND NOT A COMMENT
 *
 * The prompt's instruction is "decide, do not wire it": wiring a prewarm
 * against a session that does not exist is a no-op that reads as done. The
 * alternative to deciding is worse than either — it is leaving the criterion
 * looking pending forever, which is the unnamed omission this roadmap has
 * carried since P1.
 *
 * So the obligation is made VISIBLE AND NAMED, and the pin is written so it
 * FAILS THE MOMENT THE PREMISE CHANGES: if a wired transport ever exists, this
 * pin goes red and says prewarm is then owed. A criterion recorded as
 * unreachable is a decision with an expiry condition. A criterion silently
 * absent is a debt nobody is tracking.
 *
 * ## THE ARITHMETIC IS DERIVED FROM THE TREE, not asserted
 *
 * "There is no model session" is a claim about code, so it is measured: every
 * production conformer of `KernelRecallTransport` is read out of
 * `shell-macos/Sources/`, and the count of WIRED ones is derived. The stub in
 * `Tests/` is excluded and the exclusion is stated, because a conformer in a
 * test file is evidence about tests and not about the app.
 */

const kernelRepo = join(shellRoot, '..');
const swiftSources = join(kernelRepo, 'shell-macos', 'Sources');
const swiftTests = join(kernelRepo, 'shell-macos', 'Tests');

function swiftFilesIn(dir: string): string[] {
	const found: string[] = [];
	const walk = (current: string): void => {
		for (const entry of readdirSync(current, { withFileTypes: true })) {
			const path = join(current, entry.name);
			if (entry.isDirectory()) walk(path);
			else if (entry.name.endsWith('.swift')) found.push(path);
		}
	};
	walk(dir);
	return found;
}

/**
 * The production conformers of the transport protocol, derived.
 *
 * `Sources/` only. The stub in `Tests/AppleFMProbeTests` is a TEST FIXTURE and
 * counting it would be the wrong measurement in the dangerous direction: it
 * would make the shipped app look wired.
 */
function productionTransportConformers(): string[] {
	const conformers: string[] = [];
	for (const file of swiftFilesIn(swiftSources)) {
		const text = readFileSync(file, 'utf8');
		const pattern = /(?:public\s+)?(?:struct|class|final\s+class)\s+(\w+)[^{]*:\s*[^{]*KernelRecallTransport/g;
		let match: RegExpExecArray | null;
		while ((match = pattern.exec(text)) !== null) {
			conformers.push(match[1]!);
		}
	}
	return conformers.sort();
}

/** Does a conformer's body THROW rather than answer? The wire-vs-stub question. */
function conformerThrows(file: string, name: string): boolean {
	const text = readFileSync(file, 'utf8');
	const body = text.slice(text.indexOf(`struct ${name}`) + `struct ${name}`.length);
	// The `recall` requirement's body: a `throw` in it means the transport
	// cannot answer, which is the definition of unwired.
	return /func\s+recall[\s\S]{0,400}?\bthrow\b/.test(body);
}

test('prewarm_is_either_wired_to_a_session_that_exists_or_recorded_unreachable', () => {
	// ---- STEP 1: MEASURE how many model sessions the shipped app has ----
	//
	// Derived, not asserted. A session exists iff a production conformer of
	// `KernelRecallTransport` can actually ANSWER — and answering means not
	// throwing.
	const conformers = productionTransportConformers();
	expect(
		conformers.length,
		'the transport-conformer parse found nothing in shell-macos/Sources. An empty parse here ' +
			'would make "no session exists" vacuously true, which is the failure this pin exists to ' +
			'avoid — fail rather than pass empty.'
	).toBeGreaterThan(0);

	const wired = conformers.filter((name) => {
		const file = swiftFilesIn(swiftSources).find((f) => readFileSync(f, 'utf8').includes(`struct ${name}`));
		return file !== undefined && !conformerThrows(file, name);
	});

	// ---- STEP 2: THE ARITHMETIC, as stated values ----
	//
	// Plan 02 §7's criterion, reduced to the quantities it actually needs:
	//
	//   sessions available   = wired conformers        = 0
	//   deltas per turn      = 0 (no model session, so nothing streams)
	//   prewarm targets      = sessions available      = 0
	//   criterion reachable? = prewarm targets > 0     = FALSE
	//
	// Zero targets is not "prewarm is fast" and not "prewarm is done": it is
	// that the thing being prewarmed does not exist. Stating it as arithmetic
	// rather than prose is what keeps a reader from reading it as an oversight.
	const sessionsAvailable = wired.length;
	const deltasPerTurn = sessionsAvailable === 0 ? 0 : Number.NaN;
	const prewarmTargets = sessionsAvailable;
	const criterionReachable = prewarmTargets > 0;

	expect(sessionsAvailable, 'the derivation must actually be zero, not assumed').toBe(0);
	expect(deltasPerTurn).toBe(0);
	expect(prewarmTargets).toBe(0);
	expect(criterionReachable, 'with zero targets the criterion cannot be reached, only recorded').toBe(
		false
	);

	// ---- STEP 3: NO PREWARM IS WIRED AGAINST NOTHING ----
	//
	// The prompt's reason for not wiring: "a call to a session that does not
	// exist is a no-op that reads as done". So there must be no prewarm symbol
	// anywhere in the shell's own code. This is the arm that would catch the
	// plausible future edit — someone adding a `prewarm()` call that quietly
	// does nothing while reading as delivered.
	const shellRoots = [
		join(shellRoot, 'src'),
		join(kernelRepo, 'shell', 'src-tauri', 'src'),
		swiftSources
	];
	for (const root of shellRoots) {
		for (const file of filesUnder(root, /\.(ts|svelte|rs|swift)$/)) {
			const text = readFileSync(file, 'utf8');
			const code = text
				.split('\n')
				.filter((line) => {
					const trimmed = line.trim();
					return !trimmed.startsWith('//') && !trimmed.startsWith('*') && !trimmed.startsWith('/*');
				})
				.join('\n');
			expect(
				code.toLowerCase().includes('prewarm'),
				`${file} wires a prewarm while the derivation says there are ${prewarmTargets} sessions ` +
					'to prewarm. A prewarm against no session is a no-op that reads as done — wire a ' +
					'session first, or remove the call.'
			).toBe(false);
		}
	}

	// ---- STEP 4: THE POSITION IS RECORDED, WITH ITS REASON ----
	//
	// The obligation is only trackable if it is written down where the next
	// reader will look. This asserts the register carries the row, that the row
	// states the criterion is unreachable, and that it gives the REASON rather
	// than the word "n/a".
	const register = readFileSync(join(kernelRepo, 'docs', 'gate-law-register.md'), 'utf8');
	const row = register
		.split('\n')
		.find((line) => line.includes('prewarm') && line.includes('|'));
	expect(
		row,
		'docs/gate-law-register.md carries no prewarm row. The criterion is either met or recorded; ' +
			'"nobody mentioned it" is the third option and it is the one that loses work.'
	).toBeDefined();
	for (const fact of [
		'UNREACHABLE',
		'no model session',
		'~2.0 s',
		'~0.7 s'
	]) {
		expect(
			row,
			`the prewarm row must state \`${fact}\` — a row that records the conclusion without the ` +
				'measurement or the reason is a decision nobody can check'
		).toContain(fact);
	}
	// And it must not claim the criterion is MET: the honest word is unreachable.
	//
	// Written as four explicit forbidden PHRASES rather than a regex over the
	// line. The first draft used `/prewarm[^|]*\bWIRED\b(?!.*cannot)/i`, whose
	// pass/fail depended on which markdown cell the word landed in and on a
	// `cannot` appearing later in the row — a pin that reads green or red
	// because of table layout is a pin nobody can trust or fix.
	//
	// The second draft forbade the bare phrase `is wired`, which was blunt
	// enough to fire on the row's OWN future-tense instruction ("It is now
	// wired, re-measure cold vs warm"). A guard that cannot tell a present
	// claim from an instruction about a future state is guarding the wrong
	// thing, so every phrase below names a claim about the PRESENT.
	for (const claim of [
		'prewarm is wired',
		'prewarm has been wired',
		'criterion is met',
		'criterion has been met'
	]) {
		expect(
			row!.toLowerCase(),
			`the prewarm row must not claim the criterion is met — it contains \`${claim}\`. The ` +
				'criterion is UNREACHABLE here, and a row that says otherwise is the fabrication this ' +
				'pin exists to prevent. (Future-tense instructions — "once a session exists, wire it" ' +
				'— are correct and must stay; only a present claim is forbidden.)'
		).not.toContain(claim);
	}
	expect(
		row!.toLowerCase().includes('cannot') || row!.toLowerCase().includes('unreachable'),
		'the row must say the criterion CANNOT be met here, in those words'
	).toBe(true);

	// ---- STEP 5: THE PIN FAILS THE MOMENT THE PREMISE CHANGES ----
	//
	// This is the arm that makes the row a decision rather than a tombstone. If
	// a wired transport is ever added, `wired.length` stops being 0, this
	// assertion fails, and the message is the OBLIGATION: prewarm is now owed.
	// A pin that cannot notice its own premise expiring is a comment.
	expect(
		sessionsAvailable,
		'PREWARM IS NOW OWED. A production conformer of KernelRecallTransport can now ANSWER, so a ' +
			'model session exists and Plan 02 §7\'s criterion ("first delta paints without a blocking ' +
			'spinner, prewarm wired") is reachable. Wire it, re-measure cold vs warm, and update the ' +
			'register row — do not leave the criterion recorded as unreachable once it is not.'
	).toBe(0);

	// ---- STEP 6: THE EXCLUSION IS STATED, so it is not a hidden one ----
	//
	// There IS a transport in the test suite, and a reader checking this pin's
	// derivation would find it. Naming it here means "the stub is excluded on
	// purpose" is a recorded decision rather than a gap someone has to
	// rediscover.
	const testConformers = swiftFilesIn(swiftTests)
		.filter((file) => /:\s*[^{]*KernelRecallTransport/.test(readFileSync(file, 'utf8')))
		.map((file) => file.split('/').pop()!);
	expect(
		testConformers.length,
		'expected the test suite to carry at least one transport stub; if this is now zero the ' +
			'exclusion note below is stale'
	).toBeGreaterThan(0);
	expect(
		sessionsAvailable,
		'a TEST-FIXTURE conformer must never be counted as a shipped session — the derivation reads ' +
			`Sources/ only, and the stub in ${testConformers.join(', ')} is evidence about tests`
	).toBe(0);
});

function filesUnder(dir: string, pattern: RegExp): string[] {
	const found: string[] = [];
	const walk = (current: string): void => {
		let entries: string[];
		try {
			entries = readdirSync(current);
		} catch {
			return;
		}
		for (const entry of entries) {
			const path = join(current, entry);
			let isDirectory = false;
			try {
				isDirectory = readdirSync(path, { withFileTypes: true }).length >= 0 && !pattern.test(entry);
			} catch {
				found.push(path);
				continue;
			}
			if (isDirectory && !pattern.test(entry)) {
				walk(path);
			} else if (pattern.test(entry)) {
				found.push(path);
			}
		}
	};
	walk(dir);
	return found;
}
