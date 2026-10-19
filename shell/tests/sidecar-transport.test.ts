import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, expect, test, vi } from 'vitest';
import { DEFAULT_BASE, apiBase, resetApiBase, setApiBase } from '../src/lib/api/client';

/**
 * Phase III-0a — the sidecar transport is DECIDED, and the decline is a fact
 * about the tree rather than a stance anybody can quietly reverse.
 *
 * Plan 05 §6 specifies the sidecar as `JSONL over UDS` with
 * `BRAIN_LOCAL_SIDECAR_SOCKET`, and the sidecar plan repeats it. **None of it
 * is built.** The kernel has no `UnixListener` anywhere; `config.rs` gates
 * `BIND_HOST` and the server binds loopback HTTP; and the whole Phase I typed
 * wire — `openapi-fetch`, the generated `schema.d.ts`, the drift gate — is
 * HTTP. So the v1 contract is DECIDED to be loopback HTTP and UDS is DECLINED,
 * with the reason in `docs/gate-law-register.md`.
 *
 * **Why the decline needs a pin and not a sentence.** The prompt named the
 * failure this exists to prevent: "a UDS rewrite will arrive dressed as a
 * refactor." It reads as tidying — replace `fetch(baseUrl)` with a socket
 * client, keep the shapes — and every file it touches is individually
 * reasonable. Nothing in the tree fails when it lands. So the pin asserts the
 * decline three ways, and each can fail on its own:
 *
 *   ARM 1-4 (behavioral). The loopback law, DRIVEN. A non-loopback origin is
 *     refused on both paths a reader can change: the settings panel's
 *     `setApiBase`, and the `VITE_BRAIN_API_BASE` env override read at module
 *     load. Both FALL BACK rather than being honoured.
 *   ARM 5 (exhaustive). No UDS symbol exists in ANY of the three roots the
 *     contract spans — the webview, the Rust core, and the kernel. Walking the
 *     tree rather than naming files is the whole point: a hardcoded list goes
 *     stale the day a new nested binary or module appears, and a stale list is
 *     a green light for exactly the file it stopped watching.
 *   ARM 6-7 (structural). `tauri.conf.json` declares no socket transport and
 *     no Tauri-mediated http plugin, and `capabilities/main.json` grants no
 *     http permission.
 *
 * **Why ARM 7 is the load-bearing half of the sibling decision.** Plan 05 §4
 * says requests should go through the Rust core so the webview never receives
 * the token. That is NOT ADOPTED, and the reason is a fact rather than a
 * preference: `capabilities/main.json` grants `core:default` only and no http
 * capability exists in Tauri at all, so a Rust-mediated HTTP path cannot be
 * built without first ADDING a capability — which is a new egress surface, not
 * a refactor. So the webview keeps the wire and what moves is the token's
 * STORAGE (III-2, to the login keychain, behind the existing `kernel_token()`
 * seam). ARM 7 is what stops "move the wire to the core" from being a quiet
 * two-line change later.
 *
 * **The anti-vacuity discipline, applied to every scan.** A walk that finds
 * nothing because the directory moved is a green light for nothing — the
 * register records that failure twice already (the census pointed at the wrong
 * artifact; the II-7d walk was deleted and every parity arm went green behind
 * it). So every scan here asserts its own population first, and the loopback
 * refusal arms are paired with an arm that a loopback override is ACCEPTED,
 * because a `setApiBase` that refused everything would pass a refusal-only
 * suite.
 */

const here = dirname(fileURLToPath(import.meta.url));
const shellRoot = join(here, '..');
const kernelRoot = join(here, '..', '..');

/**
 * The UDS vocabulary, spelled exactly. `socket` on its own is NOT a needle —
 * `app.html` says "the HMR websocket" in a comment, and a needle loose enough
 * to match prose is a needle that will be widened by the next reader into
 * uselessness. Each of these names a UDS-specific construct.
 *
 * **Comments are NOT stripped, and that is a decision.** A doc comment
 * mentioning `UnixListener` reds this scan, which reads as harsh until it is
 * the alternative: an exemption keyed on "this line says we declined it" is a
 * rule whose exceptions live in prose, which the gate-law register has already
 * refused twice (R123 rule 3, F17c). Failing closed also catches the case that
 * actually matters — a comment CLAIMING a socket transport the code does not
 * use. The cost is that the decline has to be worded around the literal token.
 */
const UDS_NEEDLES = [
	'UnixListener',
	'UnixStream',
	'BRAIN_LOCAL_SIDECAR_SOCKET',
	'uds://',
	'unix://'
] as const;

/** The three roots the sidecar contract spans, each with an anti-vacuity floor. */
const SCAN_ROOTS = [
	{ label: 'shell/src', root: join(shellRoot, 'src'), exts: ['.ts', '.svelte'], floor: 40 },
	{ label: 'shell/src-tauri/src', root: join(shellRoot, 'src-tauri', 'src'), exts: ['.rs'], floor: 5 },
	{ label: 'kernel src', root: join(kernelRoot, 'src'), exts: ['.rs'], floor: 200 }
] as const;

function sourceFiles(root: string, extensions: readonly string[]): string[] {
	const out: string[] = [];
	const walk = (dir: string): void => {
		let entries: import('node:fs').Dirent[];
		try {
			entries = readdirSync(dir, { withFileTypes: true });
		} catch {
			return;
		}
		for (const entry of entries) {
			// `node_modules` is not scanned (third-party code is not this tree's
			// contract); `gen/` is generated and gitignored, so a scan that read
			// it would measure an artifact nobody reviews.
			if (entry.name === 'node_modules' || entry.name === 'gen' || entry.name === 'target') {
				continue;
			}
			const path = join(dir, entry.name);
			if (entry.isDirectory()) {
				walk(path);
			} else if (extensions.some((ext) => entry.name.endsWith(ext))) {
				out.push(path);
			}
		}
	};
	walk(root);
	return out;
}

/** Loopback, spelled the way `client.ts` spells it — deliberately a re-derivation. */
function isLoopback(raw: string): boolean {
	try {
		const host = new URL(raw).hostname;
		return host === '127.0.0.1' || host === 'localhost' || host === '::1' || host === '[::1]';
	} catch {
		return false;
	}
}

afterEach(() => {
	resetApiBase();
	vi.unstubAllEnvs();
});

test('the_sidecar_transport_is_loopback_http_and_uds_is_declined', () => {
	// ---- ARM 1 (behavioral): the default origin IS loopback ----------------
	expect(isLoopback(DEFAULT_BASE), `the pinned default ${DEFAULT_BASE} must be loopback`).toBe(
		true
	);
	expect(isLoopback(apiBase()), 'the live origin must be loopback too').toBe(true);

	// ---- ARM 2 (behavioral): a non-loopback override is REFUSED -----------
	//
	// Four shapes, because a guard that only catches a public domain name
	// catches the demo and misses the LAN: the other machine on the network is
	// the egress surface UDS was going to answer.
	//
	// The offenders are COLLECTED rather than asserted one at a time. A bare
	// `expect` per origin aborts the test at the first failure, so a plant that
	// broke all four would report one — and "the pin catches this" would be a
	// claim about a run nobody could read. Collecting reports the whole set in
	// one run, which is what makes the red-proof record below honest.
	const nonLoopback = [
		'http://evil.example.com',
		'http://10.0.0.5:8765',
		'http://192.168.1.20',
		'http://[2001:db8::1]:8765',
		'file:///etc/passwd'
	];
	const honoured: string[] = [];
	for (const origin of nonLoopback) {
		// The delta is measured PER ORIGIN against the base this iteration
		// entered with. Asserting "the base is loopback" instead would be a
		// CUMULATIVE check, and under a plant the first offender poisons the
		// base so every later iteration reports a move it did not make — a
		// diagnostic that names the wrong origin is worse than none, because
		// the fix it suggests is the wrong one.
		const before = apiBase();
		if (setApiBase(origin) !== false) honoured.push(`${origin} was ACCEPTED`);
		if (apiBase() !== before) honoured.push(`${origin} MOVED the active origin to ${apiBase()}`);
	}
	expect(
		honoured,
		'every non-loopback origin must be REFUSED, and the active origin must stay loopback (fail-safe, not fail-open)'
	).toEqual([]);

	// ---- ARM 3 (behavioral, anti-vacuity): loopback IS honoured -----------
	//
	// Without this arm ARM 2 is satisfiable by a `setApiBase` that refuses
	// every input — a shell that cannot reach its own kernel on another port,
	// which the e2e lane depends on. A refusal law with no accepting arm
	// measures nothing.
	const accepted: string[] = [];
	if (setApiBase('http://127.0.0.1:9911') !== true) accepted.push('a loopback override was REFUSED');
	if (apiBase() !== 'http://127.0.0.1:9911') accepted.push(`it did not take effect (${apiBase()})`);
	resetApiBase();
	if (apiBase() !== DEFAULT_BASE) accepted.push(`resetApiBase did not restore the default (${apiBase()})`);
	expect(accepted, 'loopback overrides must be honoured and resettable').toEqual([]);

	// ---- ARM 5 (exhaustive): no UDS symbol in any of the three roots -------
	//
	// The decline is a fact about the tree, so it is asserted as one: every
	// root's population is asserted FIRST (a moved or renamed directory lowers
	// the count and must FAIL on the floor rather than turning the scan into a
	// green light for nothing), and every offending file is collected so one run
	// reports the whole set.
	const scanFaults: string[] = [];
	for (const { label, root, exts, floor } of SCAN_ROOTS) {
		const files = sourceFiles(root, exts);
		expect(
			files.length,
			`the UDS scan read ${files.length} files under ${label}, below its floor of ${floor}. A moved or renamed directory must FAIL here, not pass empty.`
		).toBeGreaterThanOrEqual(floor);
		for (const path of files) {
			const text = readFileSync(path, 'utf8');
			for (const needle of UDS_NEEDLES) {
				if (text.includes(needle)) {
					scanFaults.push(`${relative(kernelRoot, path)} carries "${needle}"`);
				}
			}
		}
	}
	expect(
		scanFaults,
		`no source file may carry a UDS symbol. The v1 sidecar contract is loopback HTTP and UDS is DECLINED with the reason recorded in docs/gate-law-register.md — a socket transport invalidates the generated client, the drift gate and the loopback law, and buys nothing when both processes are the same user's. Reopening it is a decision to make in the open, not a refactor to arrive dressed as one. Offenders: ${JSON.stringify(scanFaults)}`
	).toEqual([]);

	// ---- ARM 6 (structural): the bundle declares no socket transport -------
	//
	// `tauri.conf.json` is read rather than named as a path so the walk covers
	// nested values: a socket configured three levels under `bundle.macOS` is
	// the shape a top-level key check would miss.
	const tauriConf = JSON.parse(
		readFileSync(join(shellRoot, 'src-tauri', 'tauri.conf.json'), 'utf8')
	);
	const confStrings: string[] = [];
	(function walk(node: unknown): void {
		if (typeof node === 'string') confStrings.push(node);
		else if (Array.isArray(node)) node.forEach(walk);
		else if (node !== null && typeof node === 'object') Object.values(node).forEach(walk);
	})(tauriConf);
	expect(
		confStrings.filter((value) => UDS_NEEDLES.some((needle) => value.includes(needle))),
		'tauri.conf.json must declare no socket transport: the sidecar is a bundled binary reached over loopback HTTP, not a socket'
	).toEqual([]);
	// The wire is not Tauri-mediated: an `http` plugin in the bundle config
	// would be the first half of Plan 05 §4's proxy-core wording becoming real.
	expect(
		Object.keys(tauriConf.plugins ?? {}),
		'tauri.conf.json declares no Tauri http plugin — the webview holds the only wire, by ARM 7'
	).not.toContain('http');

	// ---- ARM 7 (structural): the webview keeps the wire -------------------
	//
	// This is why "move the requests through the Rust core" is NOT ADOPTED. A
	// capability grant is how a Tauri-mediated HTTP path would be born, and
	// today none exists. If this arm ever fires, the decision has been reversed
	// in the tree and the register row must change with it.
	const capabilities = JSON.parse(
		readFileSync(join(shellRoot, 'src-tauri', 'capabilities', 'main.json'), 'utf8')
	);
	const permissions: string[] = capabilities.permissions ?? [];
	expect(
		permissions.filter((p) => p.includes('http')),
		`capabilities/main.json grants an http permission (${permissions.join(', ')}). That is the capability Plan 05 §4's proxy-core wording needs and II-1 refused: the webview holds the only wire, and the token's STORAGE moves to the keychain instead (III-2).`
	).toEqual([]);
	expect(permissions).toContain('core:default');
});

test('the_env_api_base_override_falls_back_to_loopback_rather_than_being_honoured', async () => {
	// Its OWN test, not an arm of the one above, and the split is the finding
	// rather than tidiness. `VITE_BRAIN_API_BASE` and the settings panel are two
	// different readers of the same law — one baked in at build time, one typed
	// at runtime — and a plant in either must be able to fail on its own. Inside
	// one test the first failed assertion ends the test, so the env arm sat
	// BEHIND the settings-panel arm and could never be observed red in the same
	// run: the row would have claimed both go red when only one ever had.
	//
	// The env is read ONCE at module load into `activeBase`, so the module is
	// re-imported under a stubbed env rather than poked afterwards. Poking after
	// the fact would assert nothing — `apiBase()` would report the base the
	// module had already resolved.
	const envFaults: string[] = [];
	vi.stubEnv('VITE_BRAIN_API_BASE', 'http://evil.example.com');
	vi.resetModules();
	const underBadEnv = await import('../src/lib/api/client');
	if (!isLoopback(underBadEnv.apiBase())) {
		envFaults.push(`VITE_BRAIN_API_BASE was HONOURED at ${underBadEnv.apiBase()}`);
	} else if (underBadEnv.apiBase() !== DEFAULT_BASE) {
		envFaults.push(`it fell back to ${underBadEnv.apiBase()}, not to the pinned default`);
	}

	// And the positive half again, on the same path: a loopback env IS taken.
	// Without it this test is satisfiable by a `resolveBase` that ignored the
	// env entirely — which is how the e2e lane picks its port, so it is a
	// behaviour that must survive, not just an escape hatch.
	vi.stubEnv('VITE_BRAIN_API_BASE', 'http://127.0.0.1:9922');
	vi.resetModules();
	const underGoodEnv = await import('../src/lib/api/client');
	if (underGoodEnv.apiBase() !== 'http://127.0.0.1:9922') {
		envFaults.push(`a loopback VITE_BRAIN_API_BASE was not taken (${underGoodEnv.apiBase()})`);
	}
	vi.unstubAllEnvs();
	vi.resetModules();

	expect(
		envFaults,
		'the env override must fall back to the loopback default, and a loopback env must be taken'
	).toEqual([]);
});

test('the_register_records_the_sidecar_transport_decision_and_what_it_ruled_out', () => {
	// A decision the next reader cannot find will be re-proposed. The register
	// row is where it lives, and "what it ruled out" is the half that matters:
	// a row stating only what it chose reads as inevitable rather than chosen.
	const register = readFileSync(join(kernelRoot, 'docs', 'gate-law-register.md'), 'utf8');
	const row = register
		.split('\n')
		.find(
			(line) =>
				line.includes('the_sidecar_transport_is_loopback_http_and_uds_is_declined') &&
				line.includes('UDS')
		);

	expect(row, 'the register carries no sidecar-transport row').toBeDefined();
	expect(row).toContain('What it ruled out');
	// Both halves of the sibling decision are named in the row, because they are
	// two answers to one question: what crosses the webview boundary.
	expect(row).toContain('kernel_token');
	expect(row).toContain('loopback');
});