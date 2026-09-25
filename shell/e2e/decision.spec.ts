import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';

const traceId = Number(process.env['E2E_DECISION_TRACE_ID']);
const runId = Number(process.env['E2E_DECISION_RUN_ID']);
const configDigest = process.env['E2E_DECISION_CONFIG_DIGEST'] ?? '';
const kernelUrl = `http://127.0.0.1:${process.env['E2E_KERNEL_PORT'] ?? 8799}`;
const syntheticQuery = 'r32 decision explorer seed';

const rulesConfig = {
	model_id: 'rules-r32-e2e',
	model_version: '1.0.0',
	rules: [
		{
			question_id: 'needs_human',
			min_evidence: 1,
			min_tier: 'untrusted',
			output: {
				Choice: { options: ['act', 'reject'], label: 'act' }
			}
		}
	]
} as const;

function decisionConfig(): Record<string, unknown> {
	return {
		config_schema: 'harness.pipeline/v1',
		pipeline_id: 'r32-e2e',
		stages: [
			'normalize',
			'retrieve_context',
			'candidate_generation',
			'decision_model',
			'rules_policy',
			'rerank',
			'threshold',
			'action_escalation'
		],
		retrieval: { rrf_k: 60, limit: 5, leg: 'both' },
		model: { key: 'rules:rules-r32-e2e', digest: configDigest },
		thresholds: {
			act_labels: ['act'],
			reject_labels: ['reject'],
			score_act_at_or_above: 50,
			score_reject_at_or_below: 10,
			noul_act_when: true,
			fallback: 'approve'
		}
	};
}

test('decision_explorer_live_wire_roundtrip_is_real_and_dedicated_port', async ({ page }) => {
	expect(Number.isSafeInteger(traceId) && traceId >= 0, 'E2E_DECISION_TRACE_ID').toBe(true);
	expect(Number.isSafeInteger(runId) && runId >= 0, 'E2E_DECISION_RUN_ID').toBe(true);
	expect(configDigest).toMatch(/^[0-9a-f]{64}$/);

	const listResponse = await fetch(`${kernelUrl}/workflow/decision-runs?limit=50&run_id=${runId}`);
	expect(listResponse.ok, `the decision list over the live wire (${listResponse.status})`).toBe(true);
	const listBefore: unknown = await listResponse.json();
	expect(listBefore).toMatchObject({ count: 1 });

	await page.goto('/decisions');
	await expect(page.getByTestId('decision-list-row')).toBeVisible();
	await expect(page.getByTestId('decision-list-nonpromotable')).toContainText('cannot promote');
	await expect(page.getByTestId('decision-list-link')).toHaveAttribute('href', `/decisions/${traceId}`);

	const detailWire = await fetch(`${kernelUrl}/workflow/decision-runs/${traceId}`);
	expect(detailWire.ok, `the decision detail over the live wire (${detailWire.status})`).toBe(true);
	const detailBytes = Buffer.from(await detailWire.arrayBuffer());
	await page.getByTestId('decision-list-link').click();
	await expect(page).toHaveURL(new RegExp(`/decisions/${traceId}$`));
	await expect(page.getByTestId('decision-detail-ready')).toBeVisible();
	await expect(page.getByTestId('decision-detail-nonpromotable')).toContainText('cannot promote');
	await expect(page.getByTestId('decision-context-table')).toContainText('untrusted');
	await expect(page.getByTestId('decision-model-table')).toContainText('rules-r32-e2e');
	expect(await page.getByTestId('decision-stage').count()).toBeGreaterThan(0);
	await page.getByRole('button', { name: /normalize/ }).click();
	await expect(page.getByTestId('decision-stage-output')).toBeVisible();
	await expect(page.getByTestId('decision-detail-ready')).not.toContainText(syntheticQuery);

	const rawDownload = page.waitForEvent('download');
	await page.getByTestId('decision-raw-download').click();
	const rawFile = await rawDownload;
	const rawPath = await rawFile.path();
	expect(rawPath).not.toBeNull();
	const exportedBytes = readFileSync(rawPath!);
	expect(exportedBytes.equals(detailBytes), 'decision raw download === live detail bytes').toBe(true);

	await page.getByTestId('replay-config').fill(JSON.stringify(decisionConfig()));
	await page.getByTestId('replay-rules').fill(JSON.stringify(rulesConfig));
	await page.getByTestId('replay-request-id').fill('r32-e2e-request');
	await page.getByTestId('replay-question-ids').fill('needs_human');
	await page.getByTestId('replay-question-id').fill('needs_human');
	await page.getByTestId('replay-question-kind').selectOption('choice');
	await page.getByTestId('replay-query').fill(syntheticQuery);
	await page.getByTestId('replay-submit').click();
	await expect(page.getByTestId('decision-replay-report')).toBeVisible();
	await expect(page.getByTestId('decision-replay-report')).toContainText('Config agreement');
	await expect(page.getByTestId('decision-replay-report')).toContainText('Input agreement');
	await expect(page.getByTestId('decision-replay-report')).toContainText('normalize');
	await expect(page.getByTestId('decision-replay-report')).not.toContainText(/promote|approve|pass\/fail/i);
	await expect(page).toHaveURL(new RegExp(`/decisions/${traceId}$`));
	await expect(page.getByTestId('decision-outcome')).toHaveText('escalate');

	const listAfterResponse = await fetch(`${kernelUrl}/workflow/decision-runs?limit=50&run_id=${runId}`);
	expect(listAfterResponse.ok).toBe(true);
	const listAfter: unknown = await listAfterResponse.json();
	expect(listAfter).toEqual(listBefore);
});
