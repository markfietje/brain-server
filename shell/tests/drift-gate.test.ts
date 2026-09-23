import { execFileSync } from 'node:child_process';
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { shellRoot, kernelRoot } from './fixtures';

/**
 * D5 — the drift gate (the M1 no-guesses law): the committed
 * src/lib/api/schema.d.ts must be EXACTLY what openapi-typescript
 * generates from the kernel's openapi.yaml — byte equality. Drift = red
 * build. (RED-then-GREEN is proven by mutating the committed file in a
 * scratch check and watching this test fail, then restoring.)
 */
test('openapi_types_are_drift_free_vs_the_kernel_contract', () => {
	const committed = readFileSync(join(shellRoot, 'src/lib/api/schema.d.ts'), 'utf8');
	const tmp = mkdtempSync(join(tmpdir(), 'wizard-schema-'));
	try {
		const out = join(tmp, 'schema.d.ts');
		execFileSync('pnpm', ['exec', 'openapi-typescript', join(kernelRoot, 'openapi.yaml'), '-o', out], {
			cwd: shellRoot,
			stdio: 'pipe'
		});
		const regenerated = readFileSync(out, 'utf8');
		expect(regenerated).toBe(committed);
	} finally {
		rmSync(tmp, { recursive: true, force: true });
	}
});
