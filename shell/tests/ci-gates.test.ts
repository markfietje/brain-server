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
	expect(workflow).toContain('pnpm exec openapi-typescript ../openapi.yaml -o "$generated"');
	expect(workflow).toContain('cmp src/lib/api/schema.d.ts "$generated"');
	expect(workflow).toContain('pnpm exec playwright install --with-deps chromium webkit');
	expect(workflow).toContain('toolchain: 1.98.1');
	expect(workflow).not.toContain('toolchain: stable');
	expect(workflow).toContain('E2E_KERNEL_PORT: 8799');
	expect(workflow).toContain('pnpm test:e2e --workers=1');
	expect(workflow).toContain('pnpm tauri build --no-bundle');
	expect(workflow).toContain('cargo install cargo-audit --version 0.22.2 --locked');
	expect(workflow).not.toContain('|| true');

	for (const action of [
		'actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1',
		'pnpm/action-setup@0977fd99725f1db4007ccb2928dbb4e90d06cc86',
		'actions/setup-node@820762786026740c76f36085b0efc47a31fe5020',
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
