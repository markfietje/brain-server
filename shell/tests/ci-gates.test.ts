import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { shellRoot } from './fixtures';

type PackageManifest = {
	scripts: Record<string, string>;
	engines?: Record<string, string>;
};

const packageText = readFileSync(join(shellRoot, 'package.json'), 'utf8');
const manifest = JSON.parse(packageText) as PackageManifest;
const workflow = readFileSync(
	join(shellRoot, '..', '.github', 'workflows', 'shell.yml'),
	'utf8'
);
const readme = readFileSync(join(shellRoot, 'README.md'), 'utf8');
const playwrightConfig = readFileSync(join(shellRoot, 'playwright.config.ts'), 'utf8');

test('shell_ci_contract_is_fail_closed_clean_runner_and_e2e_gated', () => {
	expect(manifest.engines?.['node']).toBe('>=24 <27');
	expect(manifest.scripts['gen:api']).toBe(
		'openapi-typescript ../openapi.yaml -o src/lib/api/schema.d.ts'
	);
	expect(manifest.scripts['test']).toBe('vitest run');
	expect(manifest.scripts).not.toHaveProperty('pretest');

	for (const path of [
		'shell/**',
		'openapi.yaml',
		'plugin/fixtures/invisible-classes.json',
		'.github/workflows/shell.yml'
	]) {
		expect(workflow).toContain(`- '${path}'`);
	}
	expect(workflow).toContain('permissions:\n  contents: read');
	expect(workflow).toMatch(/timeout-minutes:\s*60/);
	expect(workflow).toContain('pnpm install --frozen-lockfile');
	expect(workflow).toContain('pnpm lint');
	expect(workflow).toContain('pnpm check');
	expect(workflow).toContain('pnpm test');
	expect(workflow).toContain('pnpm build');
	expect(workflow).toContain('pnpm audit --prod --audit-level high');
	expect(workflow).toContain('cargo build --offline --locked --bin brain-server');
	// The Rust core's pins must RUN in this lane. `src-tauri` had clippy and a
	// build but no `cargo test`, so behavioral pins in the core compiled in CI
	// and never executed — the exact shape of a guard that does not exist.
	// Red-proofed 2026-10-18 by removing this step: every core pin still passed
	// locally and the lane stayed green, because nothing in it ran them.
	expect(workflow).toContain(
		'cargo test --manifest-path src-tauri/Cargo.toml --locked --offline'
	);
	expect(workflow).toContain('pnpm exec openapi-typescript ../openapi.yaml -o "$generated"');
	expect(workflow).toContain('cmp src/lib/api/schema.d.ts "$generated"');
	expect(workflow).toContain('pnpm exec playwright install --with-deps chromium webkit');
	// 1.99, matching the toolchain the server binaries build with. The pin
	// itself is still required — the lane must not float to `stable`.
	expect(workflow).toContain('toolchain: 1.99');
	expect(workflow).not.toContain('toolchain: stable');
	expect(workflow).toContain('E2E_KERNEL_PORT: 8799');
	expect(workflow).toContain('pnpm test:e2e --workers=1');
	expect(workflow).toContain('pnpm tauri build --no-bundle');
	expect(workflow).toContain('cargo install cargo-audit --version 0.22.2 --locked');
	expect(workflow).not.toContain('|| true');

	for (const action of [
		'actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1',
		'pnpm/action-setup@ea17c68df8912ef543352723c149a84f56e3d413',
		'actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1',
		'Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6',
		'dtolnay/rust-toolchain@6bed0761d98439e5a578e2877258200ad565ba87'
	]) {
		expect(workflow).toContain(action);
	}
	for (const line of workflow.split('\n')) {
		const match = line.match(/uses:\s*(\S+)/);
		const reference = match?.[1];
		if (reference === undefined || reference.startsWith('./')) continue;
		expect(reference).toMatch(/@[0-9a-f]{40}$/);
	}
	expect(workflow).toContain('libwebkit2gtk-4.1-dev');
	expect(workflow).toContain('uses: ./.github/actions/huggingface-prefetch');
	expect(workflow).toContain('shell/src-tauri');
	expect(workflow).toContain('workspaces: shell/src-tauri');

	expect(playwrightConfig).toContain("workers: process.env['CI'] ? 1 : undefined");
	expect(playwrightConfig).toContain("name: 'webkit'");
	expect(playwrightConfig).toContain('bypassCSP: false');
	expect(readme).toContain('>=24 <27');
	expect(readme).toContain('Node 24');
});

/**
 * III-1 — the cert-free macOS bundle lane: stage both kernel arches, build
 * the universal app, WALK every Mach-O, and plant an unsigned helper that
 * must make the walk FAIL. No secrets, so it may run on the public mirror.
 * Its first real execution is OWED until the next mirror push; the register
 * row says so rather than claiming a run nobody has observed.
 */
test('the_macos_bundle_lane_stages_builds_and_walks', () => {
	expect(workflow).toContain('bundle-macos:');
	expect(workflow).toContain('runs-on: macos-latest');
	expect(workflow).toContain('node shell/scripts/sidecar.mjs stage');
	expect(workflow).toContain(
		'pnpm tauri build --bundles app --target universal-apple-darwin'
	);
	expect(workflow).toContain('node shell/scripts/sidecar.mjs verify');
	expect(workflow).toContain('targets: aarch64-apple-darwin,x86_64-apple-darwin');
	expect(workflow).toContain('codesign --remove-signature');
	expect(workflow).toContain('planted-helper');
	// The lane is deliberately secret-free: ad-hoc signing needs no keychain.
	const bundleJob = workflow.slice(workflow.indexOf('bundle-macos:'));
	expect(bundleJob).not.toContain('secrets.');
	expect(bundleJob).toContain('Contents/MacOS/brain-server" --version');
});

/**
 * III-2 — the keychain dependency is target-gated, and the mock seam stays
 * portable. `keyring-core` is pure Rust and must build on the Ubuntu lane so
 * the mock-store pins compile there; only the Apple store is macOS-gated. The
 * v1 `keyring` wrapper is declined by name (no injection point).
 */
test('the_keychain_dependencies_are_target_gated_and_use_keyring_core', () => {
	const cargoToml = readFileSync(join(shellRoot, 'src-tauri', 'Cargo.toml'), 'utf8');
	const cargoLock = readFileSync(join(shellRoot, 'src-tauri', 'Cargo.lock'), 'utf8');

	const table = (header: string): string => {
		const at = cargoToml.indexOf(header);
		expect(at, `Cargo.toml lost ${header}`).toBeGreaterThanOrEqual(0);
		const rest = cargoToml.slice(at + header.length);
		const end = rest.search(/^\[/m);
		return end === -1 ? rest : rest.slice(0, end);
	};

	const dependencies = table('[dependencies]');
	const macDependencies = table(`[target.'cfg(target_os = "macos")'.dependencies]`);

	// Portable: the mock-store tests run on Linux.
	expect(dependencies).toMatch(/^keyring-core\s*=\s*"1\.0\.1"$/m);
	// Apple-only: Linux must never build the platform store.
	expect(macDependencies).toMatch(
		/^apple-native-keyring-store\s*=\s*\{[^}]*version\s*=\s*"1\.0\.2"[^}]*\}$/m
	);
	expect(macDependencies).toContain('features = ["keychain"]');
	expect(dependencies).not.toContain('apple-native-keyring-store');

	// The v1 meta-crate is declined: it hard-wires the platform store in a
	// process-global default, which makes the mock seam impossible.
	expect(cargoToml).not.toMatch(/^keyring\s*=/m);

	// The lock resolves the two real crates and no `keyring` package (the
	// exact line, not the `keyring-core` substring).
	expect(cargoLock).toContain('\nname = "keyring-core"\n');
	expect(cargoLock).toContain('\nname = "apple-native-keyring-store"\n');
	expect(cargoLock).not.toContain('\nname = "keyring"\n');

	// The offline test step is unchanged: the online clippy step is what
	// populates the registry first (C10 — no fetch step was added).
	expect(workflow).toContain(
		'cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings'
	);
	expect(workflow).toContain(
		'cargo test --manifest-path src-tauri/Cargo.toml --locked --offline'
	);
});
