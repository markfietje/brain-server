import { readFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { describe, expect, test } from 'vitest';

/**
 * III-8 · the plugin's own lane pins.
 *
 * Until this round the plugin's suite RAN NOWHERE: 1,768 lines across seven
 * files, no workflow executing any of them, no `test` script in
 * `package.json`, and `npm install` failing outright on the `workspace:`
 * devDependency. Five of the seven files failed to LOAD and the two that
 * loaded ran 70 tests — in a developer's terminal, and in no lane at all.
 *
 * These pins hold the round's three laws: the install works standalone (D3),
 * the entry point imports without the host present (D2), and the canonical
 * drift fixture names only consumers that actually run (D4). The lane's own
 * EXISTENCE is pinned outside this suite — by the kernel's Rust pin — because
 * a pin that lives inside the lane it pins dies with it.
 */

const pluginRoot = new URL('..', import.meta.url).pathname;
const repoRoot = new URL('../..', import.meta.url).pathname;

const read = (path: string): string => readFileSync(join(pluginRoot, path), 'utf8');
const manifest = JSON.parse(read('package.json')) as Record<string, unknown>;
const vitestConfig = read('vitest.config.ts');

// ── D3 · the plugin's dependencies install STANDALONE ───────────────────────

describe('the plugin installs standalone', () => {
	test('npm resolves the plugin manifest with no host present', () => {
		const scratch = mkdtempSync(join(tmpdir(), 'iii8-install-'));
		try {
			writeFileSync(join(scratch, 'package.json'), read('package.json'));
			// Resolution only, from the warm cache — the lane's `npm ci` has just
			// populated it. **The pin is deliberately OFFLINE**, because a pin
			// that reaches for the registry is a pin at the network's mercy: it
			// flaked once in five runs (measured 2026-10-22), and R135's lesson
			// about the keychain applies verbatim — a pin that shells out past
			// the machine must be bounded, or it is a flake. Offline keeps the
			// LAW intact: npm still parses the manifest and still refuses a
			// `workspace:` URL before it looks at any cache (measured — the
			// restored devDependency reds with npm's own EUNSUPPORTEDPROTOCOL
			// under `--offline` too). The lane's own `npm ci` remains the proof
			// that the full install works on a clean checkout, network and all.
			const install = spawnSync(
				'npm',
				[
					'install',
					'--package-lock-only',
					'--offline',
					'--no-audit',
					'--no-fund',
					'--ignore-scripts'
				],
				{ cwd: scratch, encoding: 'utf8', timeout: 120_000 }
			);
			const output = `${install.stdout}\n${install.stderr}`;
			expect(
				install.status,
				`npm install --package-lock-only failed in a clean tree ` +
					`(if this names missing packages, the cache was cold — run npm ci first):\n${output}`
			).toBe(0);
			// npm's own failure for the blocking shape is EUNSUPPORTEDPROTOCOL —
			// the red-proof restores it and expects exactly this string.
			expect(output).not.toContain('EUNSUPPORTEDPROTOCOL');
		} finally {
			rmSync(scratch, { recursive: true, force: true });
		}
	});

	test('no install-time dependency carries the workspace specifier', () => {
		// npm's EUNSUPPORTEDPROTOCOL is caused by a `workspace:` URL in a field
		// it INSTALLS from. A peer dependency is not installed by the package
		// that declares it — the HOST provides it — and an optional peer is
		// skipped entirely when the host is absent.
		for (const field of ['dependencies', 'devDependencies', 'optionalDependencies']) {
			const entries = (manifest[field] ?? {}) as Record<string, string>;
			for (const [name, specifier] of Object.entries(entries)) {
				expect(
					specifier.startsWith('workspace:'),
					`${field}.${name} carries "${specifier}" — npm refuses a workspace: URL outside the host's monorepo`
				).toBe(false);
			}
		}
		const peers = manifest.peerDependencies as Record<string, string> | undefined;
		const peerMeta = manifest.peerDependenciesMeta as
			| Record<string, { optional?: boolean }>
			| undefined;
		expect(peers?.['@openclaw/plugin-sdk'], 'the SDK must stay declared as the host\'s peer').toBe(
			'workspace:*'
		);
		expect(
			peerMeta?.['@openclaw/plugin-sdk']?.optional,
			'the SDK peer must be OPTIONAL: the host provides it, and a standalone tree must not need it'
		).toBe(true);
	});

	test('the manifest carries a test script the lane can run', () => {
		const scripts = manifest.scripts as Record<string, string>;
		expect(scripts['test'], 'the lane runs `npm test`').toBe('vitest run');
		expect(scripts['typecheck'], 'the lane type-checks before it runs the suite').toBe(
			'tsc --noEmit -p tsconfig.json'
		);
	});
});

// ── D2 · the entry point imports WITHOUT the host present ───────────────────

describe('the entry point imports without the host', () => {
	test('the host package is absent from this tree', () => {
		for (const absent of ['node_modules/@openclaw/plugin-sdk', 'node_modules/openclaw']) {
			let present = true;
			try {
				readFileSync(join(pluginRoot, absent, 'package.json'));
			} catch {
				present = false;
			}
			expect(present, `${absent} must be absent — this pin proves the host is not here`).toBe(
				false
			);
		}
	});

	test('the entry point loads, with the host absent, through the lane alias', async () => {
		// A dynamic import, so a regression reds THIS arm instead of failing
		// the whole file at collection time with an error that names nothing.
		const loaded = (await import('../index.js')) as { default?: unknown };
		const entry = loaded.default as Record<string, unknown>;
		expect(entry, 'the entry point exported nothing').toBeTruthy();
		for (const field of ['id', 'name', 'description', 'configSchema', 'register']) {
			expect(entry[field], `the entry point carries no ${field}`).toBeTruthy();
		}
		expect(entry.id).toBe('brain-server');
	});

	test('the lane alias resolves the host specifier to the stub', () => {
		expect(vitestConfig).toContain("'openclaw/plugin-sdk/plugin-entry'");
		expect(vitestConfig).toContain('test/host-sdk.stub.ts');
	});

	test('the stub refuses an entry that drifts from the declared contract', async () => {
		const { definePluginEntry } = (await import('./host-sdk.stub.js')) as {
			definePluginEntry: (entry: unknown) => unknown;
		};
		for (const broken of [
			{ name: 'Brain', description: 'd', configSchema: {}, register: () => undefined }, // no id
			{ id: 'brain-server', description: 'd', configSchema: {}, register: () => undefined }, // no name
			{ id: 'brain-server', name: 'Brain', configSchema: {}, register: () => undefined }, // no description
			{ id: 'brain-server', name: 'Brain', description: 'd', register: () => undefined }, // no schema
			{ id: 'brain-server', name: 'Brain', description: 'd', configSchema: {} } // no register
		]) {
			expect(
				() => definePluginEntry(broken),
				`the stub accepted an entry without ${Object.keys(broken).length} field(s)`
			).toThrow();
		}
		const good = {
			id: 'brain-server',
			name: 'Brain',
			description: 'd',
			configSchema: {},
			register: () => undefined
		};
		expect(definePluginEntry(good)).toBe(good);
	});
});

// ── D4 · the drift alarm has LIVE trees ─────────────────────────────────────

type FixtureConsumer = {
	/** The path the fixture's comment names, relative to the repo root. */
	path: string;
	/** Where the consumer lives — this repository, or the fork host's. */
	tree: 'this repo' | 'another repository';
	/** The lane that executes the consumer, and the command that runs it. */
	lane: { workflow: string; command: string } | null;
	/** The evidence that the consumer actually READS this fixture. */
	reads: string;
};

const invisibleFixture = JSON.parse(
	read('fixtures/invisible-classes.json')
) as { comment: string };

const consumers: FixtureConsumer[] = [
	{
		path: 'src/strip_invisible.rs',
		tree: 'this repo',
		lane: { workflow: '.github/workflows/ci.yml', command: 'cargo test' },
		reads: 'include_str!("../plugin/fixtures/invisible-classes.json")'
	},
	{
		path: 'plugin/src/format.test.ts',
		tree: 'this repo',
		lane: { workflow: '.github/workflows/plugin.yml', command: 'npm test' },
		reads: 'fixtures/invisible-classes.json'
	},
	{
		path: 'client/src/main.rs',
		tree: 'this repo',
		lane: { workflow: '.github/workflows/ci.yml', command: 'cargo test --manifest-path client/Cargo.toml' },
		reads: 'include_str!("../../plugin/fixtures/invisible-classes.json")'
	},
	{
		path: 'src/infra/unicode-visibility.fixture.test.ts',
		tree: 'another repository',
		lane: null,
		reads: 'the fork host reads it through the parity-synced extension tree'
	},
	{
		path: 'shell/src/lib/sanitize.ts',
		tree: 'this repo',
		lane: { workflow: '.github/workflows/shell.yml', command: 'pnpm test' },
		reads: 'shell/tests/sanitize.test.ts reads the fixture and holds the vendored predicate to it'
	}
];

describe('the invisible-set drift alarm has live trees', () => {
	test('the fixture names the consumers this pin holds', () => {
		for (const consumer of consumers) {
			expect(
				invisibleFixture.comment,
				`the fixture's comment no longer names ${consumer.path} — update the comment AND this table together`
			).toContain(consumer.path);
		}
	});

	test('every named consumer path exists where the comment says it lives', () => {
		for (const consumer of consumers) {
			if (consumer.tree === 'another repository') continue;
			let present = true;
			try {
				readFileSync(join(repoRoot, consumer.path));
			} catch {
				present = false;
			}
			expect(
				present,
				`the fixture's comment names ${consumer.path} and no such file exists — a consumer that is never consulted is a dark tree, and the comment is a false claim`
			).toBe(true);
		}
	});

	test('every in-repo consumer actually reads the fixture', () => {
		for (const consumer of consumers) {
			if (consumer.tree === 'another repository') continue;
			let source = '';
			try {
				source = readFileSync(join(repoRoot, consumer.path), 'utf8');
			} catch {
				source = '';
			}
			// The plugin's consumer IS the fixture's own reader; the shell's
			// consumer is its TEST (the module carries the predicate, the test
			// holds it to the fixture).
			const witness =
				consumer.path === 'shell/src/lib/sanitize.ts'
					? readFileSync(join(repoRoot, 'shell/tests/sanitize.test.ts'), 'utf8')
					: source;
			expect(
				witness.includes(consumer.reads) || witness.includes('invisible-classes.json'),
				`${consumer.path} no longer reads plugin/fixtures/invisible-classes.json`
			).toBe(true);
		}
	});

	test('every in-repo consumer runs in a lane', () => {
		for (const consumer of consumers) {
			if (consumer.tree === 'another repository' || consumer.lane === null) continue;
			const workflow = readFileSync(join(repoRoot, consumer.lane.workflow), 'utf8');
			expect(
				workflow.includes(consumer.lane.command),
				`${consumer.path} is a dark tree: ${consumer.lane.workflow} does not run "${consumer.lane.command}"`
			).toBe(true);
		}
	});
});

// ── the suite is complete, and the lane names it ────────────────────────────

describe('the suite is complete and named by the lane', () => {
	// The include list the vitest config declares — asserted verbatim, so a
	// config edit that stops collecting a suite file reds here.
	const declaredIncludes = [
		'test/**/*.test.ts',
		'src/**/*.test.ts',
		'src/**/*.spec.ts'
	];

	const suiteFiles = [
		'test/plugin.test.ts',
		'test/team-bridge.test.ts',
		'test/fence-parity.test.ts',
		'test/lane-pins.test.ts',
		'test/integration-drill-pins.test.ts',
		'src/format.test.ts',
		'src/config.test.ts',
		'src/brain-client.contract.spec.ts',
		'src/gating.contract.spec.ts'
	];

	test('the vitest config declares exactly the include list the suite needs', () => {
		for (const include of declaredIncludes) {
			expect(vitestConfig, `the lane no longer includes ${include}`).toContain(`'${include}'`);
		}
	});

	test('every suite file exists, is covered by the include list, and declares tests', () => {
		let total = 0;
		for (const file of suiteFiles) {
			const source = read(file);
			const covered = declaredIncludes.some((pattern) => {
				const [dir, suffix] = pattern.split('/**/');
				return file.startsWith(`${dir}/`) && file.endsWith(suffix.replace(/^\*\./, '.'));
			});
			expect(covered, `${file} is not covered by any include the lane declares`).toBe(true);
			const declared = (source.match(/\btest\(/g) ?? []).length;
			expect(declared, `${file} declares no test`).toBeGreaterThan(0);
			total += declared;
		}
		// The baseline measured 2026-10-22 was 70 tests in 2 of 7 files — five
		// files failed to LOAD, so 5 files' worth of pins ran NOWHERE. The floor
		// is the measured whole-suite declaration count: it only holds while
		// every file loads.
		expect(total, 'the suite lost declarations — a file stopped loading').toBeGreaterThanOrEqual(211);
	});
});
