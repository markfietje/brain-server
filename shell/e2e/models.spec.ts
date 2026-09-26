import { expect, test } from '@playwright/test';
import { stripInvisible } from '../src/lib/sanitize';

/**
 * R36 M6-S2 -- the Model Registry view, over the REAL wire.
 *
 * The dedicated-port harness (e2e/global-setup.ts) boots a fresh, temporary
 * kernel on 127.0.0.1:8799 and registers ONE synthetic candidate through the
 * live route: id `rules-r32-e2e`, version `1.0.0`, kind `deterministic-rules`,
 * asserted `status: candidate`. The operator's 8765 service, the live database
 * and the live kernel are never queried, mutated, or restarted by this file.
 *
 * The first test is one roundtrip -- list, detail, propose, pending receipt --
 * with the three refusals this surface is built around MEASURED on the wire
 * rather than read off the markup:
 *
 *   1. A DIGEST IS A PIN, NOT A SIGNATURE. The `row_digest` the client
 *      forwards is the EXACT digest the detail read returned, byte for byte,
 *      and the value the human will later review is that same digest.
 *   2. A PROPOSAL IS NOT AN APPLICATION. The receipt is pending only, the
 *      review queue gained one undecided `registry_lifecycle` row, and the
 *      registry row's status AND its digest are both unmoved.
 *   3. NO APPROVE/REJECT CONTROL AND NO DIRECT STATUS WRITE. The only write
 *      affordances are lifecycle proposals, and the only non-GET request the
 *      surface ever issues over the whole roundtrip is that one proposal POST.
 */

const kernelPort = Number(process.env['E2E_KERNEL_PORT'] ?? 8799);
const kernelUrl = `http://127.0.0.1:${kernelPort}`;

/** The identity the harness seeded through the live route. Never invented here. */
const SEEDED_ID = 'rules-r32-e2e';
const SEEDED_VERSION = '1.0.0';
const SEEDED_REF = `${SEEDED_ID}@${SEEDED_VERSION}`;

/**
 * U+200B ZERO WIDTH SPACE, built from its CODEPOINT.
 *
 * A literal U+200B byte in source was MEASURED at being stripped by the
 * tooling, which silently guts the assertion it was supposed to make. This
 * file therefore holds no literal invisible byte at all.
 */
const ZWSP = String.fromCharCode(0x200b);

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function requiredObject(value: unknown, label: string): Record<string, unknown> {
	if (!isRecord(value)) throw new Error(`the ${label} was not a JSON object`);
	return value;
}

function requiredArray(value: unknown, label: string): unknown[] {
	if (!Array.isArray(value)) throw new Error(`the ${label} was not a JSON array`);
	return value;
}

function requiredString(value: unknown, label: string): string {
	if (typeof value !== 'string' || value.length === 0) {
		throw new Error(`the ${label} was not a non-empty string`);
	}
	return value;
}

/** Exact lowercase 64-hex. Never computed, never repaired, only carried. */
function requiredDigest(value: unknown, label: string): string {
	if (typeof value !== 'string' || !/^[0-9a-f]{64}$/.test(value)) {
		throw new Error(`the ${label} was not a lowercase SHA-256 digest`);
	}
	return value;
}

/** Every object key anywhere in a JSON value: a presence law needs KEYS. */
function everyKey(value: unknown, found: string[] = []): string[] {
	if (Array.isArray(value)) {
		for (const item of value) everyKey(item, found);
	} else if (isRecord(value)) {
		for (const [key, item] of Object.entries(value)) {
			found.push(key);
			everyKey(item, found);
		}
	}
	return found;
}

interface WireRead {
	readonly status: number;
	readonly text: string;
	readonly body: unknown;
}

async function wireGet(path: string): Promise<WireRead> {
	const response = await fetch(`${kernelUrl}${path}`);
	const text = await response.text();
	const body: unknown = JSON.parse(text);
	return { status: response.status, text, body };
}

async function wirePost(path: string, body: unknown): Promise<WireRead> {
	const response = await fetch(`${kernelUrl}${path}`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(body)
	});
	const text = await response.text();
	const parsed: unknown = JSON.parse(text);
	return { status: response.status, text, body: parsed };
}

/** The one write this surface is allowed to issue. */
function isProposalPost(url: string, method: string): boolean {
	return method === 'POST' && new URL(url).pathname === '/ingest/proposal';
}

test('model_registry_live_wire_roundtrip_is_real_and_dedicated_port', async ({ page }) => {
	// The dedicated port is a hard precondition of this file: the operator's
	// 8765 service and the live database are out of scope by construction.
	expect(kernelPort, 'E2E_KERNEL_PORT must be the dedicated e2e port').toBe(8799);
	expect(kernelPort, 'this spec never targets the operator service port').not.toBe(8765);

	// Every request the page makes to the e2e kernel, for the write-surface law.
	const kernelRequests: string[] = [];
	page.on('request', (request) => {
		const url = new URL(request.url());
		if (url.origin === kernelUrl) {
			kernelRequests.push(`${request.method()} ${url.pathname}`);
		}
	});

	// ---- LIST, over the live wire ------------------------------------------
	const list = await wireGet('/workflow/model-registry?limit=50');
	expect(list.status, `the model listing over the live wire (${list.status}: ${list.text})`).toBe(
		200
	);
	const listBody = requiredObject(list.body, 'listing body');
	const listRows = requiredArray(listBody['rows'], 'listing rows');
	expect(listBody['count'], 'the listing count is the row count').toBe(listRows.length);

	const seeded: unknown = listRows.find(
		(row) => isRecord(row) && row['id'] === SEEDED_ID && row['version'] === SEEDED_VERSION
	);
	expect(seeded, `the seeded ${SEEDED_REF} row rides the listing`).toBeDefined();
	const seededRow = requiredObject(seeded, `the seeded ${SEEDED_REF} listing row`);
	expect(seededRow).toMatchObject({
		id: SEEDED_ID,
		version: SEEDED_VERSION,
		kind: 'deterministic-rules',
		status: 'candidate'
	});

	// The listing carries artifact digest as a PRESENCE BOOLEAN on the wire --
	// never as a value, and never as the row digest.
	const listKeys = everyKey(list.body);
	expect(listKeys, 'the listing carries artifact PRESENCE').toContain('artifact_digest_present');
	expect(listKeys, 'the listing never rides an artifact digest VALUE').not.toContain(
		'artifact_digest'
	);
	expect(listKeys, 'the listing never rides a row digest').not.toContain('row_digest');
	expect(
		seededRow['artifact_digest_present'],
		'the seeded rules row declares its artifact absent'
	).toBe(false);

	// ---- LIST, in the DOM ---------------------------------------------------
	await page.goto('/models');
	const listRow = page.getByTestId('models-list-row').filter({ hasText: SEEDED_ID });
	await expect(listRow, 'the seeded row renders in the listing').toHaveCount(1);
	await expect(listRow.getByTestId('models-list-link')).toHaveText(SEEDED_ID);
	await expect(
		listRow.getByTestId('models-list-link'),
		'the row links to its own single-row read'
	).toHaveAttribute('href', `/models?ref=${encodeURIComponent(SEEDED_REF)}`);
	await expect(listRow, 'the row carries its identity').toContainText(SEEDED_VERSION);
	await expect(listRow, 'the row carries its KIND').toContainText('Deterministic rules');
	await expect(
		listRow.getByTestId('models-list-status'),
		'the row carries its STATUS as text, not as decoration'
	).toHaveText('Candidate');

	// The artifact column is a PRESENCE WORD. A digest value here would be the
	// listing leaking a pin it has no authority to display.
	const artifactCell = listRow.getByTestId('models-list-artifact');
	await expect(artifactCell, 'the artifact column is a presence word').toHaveText('No artifact');
	await expect(artifactCell, 'no digest value in the artifact column').not.toContainText(
		/[0-9a-f]{64}/
	);
	await expect(
		page.getByTestId('models-detail-digest-artifact'),
		'the artifact digest VALUE is a single-row-read field, absent from the listing'
	).toHaveCount(0);

	// ---- DETAIL, over the live wire and in the DOM --------------------------
	const detailRequest = page.waitForRequest(
		(request) =>
			request.method() === 'GET' &&
			new URL(request.url()).pathname.startsWith('/workflow/model-registry/')
	);
	await listRow.getByTestId('models-list-link').click();
	const detailRequestSeen = await detailRequest;
	expect(
		decodeURIComponent(new URL(detailRequestSeen.url()).pathname),
		'the detail read is the id-scoped single-row route for exactly this citation'
	).toBe(`/workflow/model-registry/${SEEDED_REF}`);

	const detailWire = await wireGet(`/workflow/model-registry/${SEEDED_REF}`);
	expect(
		detailWire.status,
		`the single-row detail over the live wire (${detailWire.status}: ${detailWire.text})`
	).toBe(200);
	const detailBody = requiredObject(detailWire.body, 'single-row detail body');
	expect(detailBody).toMatchObject({
		id: SEEDED_ID,
		version: SEEDED_VERSION,
		kind: 'deterministic-rules',
		status: 'candidate'
	});
	// CAPTURED HERE, off the live read. Everything downstream compares against
	// this one value: it is the pin, and it is never recomputed anywhere.
	const rowDigest = requiredDigest(detailBody['row_digest'], 'live row_digest');

	await expect(page).toHaveURL(new RegExp(`/models\\?ref=${encodeURIComponent(SEEDED_REF)}$`));
	await expect(page.getByTestId('models-detail-ready')).toBeVisible();
	await expect(page.getByTestId('models-detail-status')).toHaveText('Candidate');
	await expect(
		page.getByTestId('models-detail-digest-row'),
		'the row digest is displayed VERBATIM: no truncation, no re-casing, no repair'
	).toHaveText(rowDigest);
	await expect(
		page.getByTestId('models-digest-pin-note'),
		'a digest is a pin, not a signature and not authentication'
	).toContainText('A content pin, not a signature');

	// ---- THE ONLY WRITE AFFORDANCES ARE LEGAL LIFECYCLE PROPOSALS -----------
	const actionLabels = await page
		.getByTestId('models-detail-actions')
		.locator('button')
		.allInnerTexts();
	expect(actionLabels.slice().sort(), 'the only write affordances are the legal PROPOSALS').toEqual(
		['Propose promotion', 'Propose retirement']
	);
	await expect(page.getByTestId('models-action-approve')).toHaveCount(0);
	await expect(page.getByTestId('models-action-reject')).toHaveCount(0);
	await expect(
		page.getByTestId('models-detail-actions'),
		'the surface states that a human approves the change by digest'
	).toContainText(
		'This change is a proposal. A human approves it by digest; nothing is applied automatically.'
	);

	// ---- PROPOSE ------------------------------------------------------------
	const proposalRequest = page.waitForRequest((request) =>
		isProposalPost(request.url(), request.method())
	);
	// The receipt is the only place the client reports the outcome, so the
	// kernel's own answer to that POST is captured too: a refused proposal must
	// name its refusal, never just fail to render a receipt.
	const proposalResponse = page.waitForResponse((response) =>
		isProposalPost(response.url(), response.request().method())
	);
	await page.getByTestId('models-action-promote').click();
	const proposal = await proposalRequest;
	const proposalResponseSeen = await proposalResponse;
	const proposalStatus = proposalResponseSeen.status();
	const proposalBody = (await proposalResponseSeen.text()).slice(0, 500);
	const sentBody = requiredObject(
		JSON.parse(proposal.postData() ?? ''),
		'the outbound proposal request body'
	);
	expect(sentBody['kind'], 'a lifecycle change is a proposal, never a mutation').toBe(
		'registry_lifecycle'
	);
	expect(
		Object.keys(sentBody),
		'the outbound body carries no status channel: a client cannot write a status'
	).not.toContain('status');
	const content = requiredObject(
		JSON.parse(requiredString(sentBody['content'], 'outbound proposal content')),
		'the outbound lifecycle payload'
	);
	expect(content).toMatchObject({ action: 'promote', id: SEEDED_ID, version: SEEDED_VERSION });
	// THE PIN: the exact digest the detail read returned, forwarded verbatim.
	expect(
		content['row_digest'],
		'the proposal carries the EXACT row_digest captured from the detail read'
	).toBe(rowDigest);
	expect(
		content['status'],
		'the payload declares no target status: there is nothing to apply'
	).toBeUndefined();
	const sentRow = requiredObject(content['row'], 'the outbound proposal row');
	expect(sentRow).toMatchObject({ id: SEEDED_ID, version: SEEDED_VERSION, status: 'candidate' });

	// The pin travels BESIDE the row, never inside it. This was a LIVE 400
	// (`registry_payload_invalid`, unknown field `row_digest`): the kernel's
	// RegistryRow is #[serde(deny_unknown_fields)] with no row_digest member,
	// and the kernel's own contract test clones the detail and REMOVES
	// row_digest before deserializing.
	expect(
		sentRow['row_digest'],
		'the row must NOT repeat the pin: RegistryRow is deny_unknown_fields and has no such member'
	).toBeUndefined();
	// The row is exactly the kernel's field set, with approved_by present even
	// when null (no serde default) -- also a live 400 when omitted.
	expect(Object.keys(sentRow).sort()).toEqual(
		[
			'id',
			'version',
			'kind',
			'name',
			'output_vocabulary',
			'artifact_digest',
			'config_digest',
			'calibration_ref',
			'status',
			'evaluation_refs',
			'proposed_by',
			'approved_by',
			'created_at',
			'updated_at'
		].sort()
	);

	// ---- THE RECEIPT IS PENDING ONLY ----------------------------------------
	const receipt = page.getByTestId('models-proposal-pending');
	await expect(
		receipt,
		`the receipt reports a pending proposal (the kernel answered the proposal POST ${proposalStatus}: ${proposalBody})`
	).toBeVisible();
	expect(
		proposalStatus,
		`a lifecycle proposal is accepted into the review queue, never refused (${proposalBody})`
	).toBe(200);
	await expect(receipt).toContainText('Pending human approval');
	await expect(
		receipt,
		'a receipt never claims the change was promoted, approved, applied, or decided'
	).not.toContainText(/promoted|approved|applied|complete|rejected|retired/i);
	await expect(
		receipt.locator('button, a'),
		'a pending receipt offers no decision control of any kind'
	).toHaveCount(0);
	const proposalId = requiredString(
		await page.getByTestId('models-proposal-id').innerText(),
		'the receipt proposal id'
	);
	expect(proposalId, 'the receipt names the proposal that was created').toMatch(/^\d+$/);

	// ---- A PROPOSAL IS NOT AN APPLICATION ------------------------------------
	const afterWire = await wireGet(`/workflow/model-registry/${SEEDED_REF}`);
	expect(
		afterWire.status,
		`the post-proposal detail read (${afterWire.status}: ${afterWire.text})`
	).toBe(200);
	const afterBody = requiredObject(afterWire.body, 'the post-proposal detail body');
	expect(afterBody['status'], 'the row status is UNCHANGED on the server').toBe('candidate');
	expect(
		afterBody['row_digest'],
		'a proposal moves not one byte of the registry row'
	).toBe(rowDigest);
	await expect(
		page.getByTestId('models-detail-status'),
		'the view still shows the unchanged status'
	).toHaveText('Candidate');

	// The proposal exists as a QUEUE ROW a human must decide, and it is the same
	// pinned bytes the view displayed.
	const queue = await wireGet('/proposals?status=pending&limit=200');
	expect(queue.status, `the pending review queue (${queue.status}: ${queue.text})`).toBe(200);
	const queueRows = requiredArray(queue.body, 'the pending review queue rows');
	const queued: unknown = queueRows.find(
		(row) => isRecord(row) && row['id'] === Number(proposalId)
	);
	expect(queued, `proposal ${proposalId} is queued for a human`).toBeDefined();
	const queuedRow = requiredObject(queued, `the queued proposal ${proposalId}`);
	expect(queuedRow['kind'], 'the queued row is a registry lifecycle proposal').toBe(
		'registry_lifecycle'
	);
	expect(queuedRow['decided_at'], 'the queued proposal is undecided').toBeNull();
	const queuedContent = requiredObject(
		JSON.parse(requiredString(queuedRow['content'], 'the queued proposal content')),
		'the queued lifecycle payload'
	);
	expect(
		queuedContent['row_digest'],
		'the human reviews the same pinned bytes the view displayed'
	).toBe(rowDigest);

	// ---- NO APPROVE/REJECT CONTROL, NO DIRECT STATUS WRITE -------------------
	expect(
		kernelRequests.filter((entry) => !entry.startsWith('GET ')),
		'the only write the surface ever issued is the one proposal POST'
	).toEqual(['POST /ingest/proposal']);
	expect(
		kernelRequests.filter((entry) => /^(PUT|PATCH|DELETE) /.test(entry)),
		'no status write was ever attempted by any control on this surface'
	).toEqual([]);

	const controlLabels = await page
		.getByTestId('models-view')
		.locator('a, button, input, select, textarea')
		.evaluateAll((nodes) =>
			nodes.map((node) => {
				const field =
					node instanceof HTMLInputElement ||
					node instanceof HTMLSelectElement ||
					node instanceof HTMLTextAreaElement
						? node.value
						: '';
				return [
					node.textContent ?? '',
					node.getAttribute('aria-label') ?? '',
					node.getAttribute('placeholder') ?? '',
					node.getAttribute('data-testid') ?? '',
					field
				].join(' ');
			})
		);
	for (const label of controlLabels) {
		expect(
			label,
			`no approve/reject/status control exists on the model-registry surface: "${label}"`
		).not.toMatch(/\b(approve|reject|accept|deny|apply|status|delete|remove)\b/i);
	}
});


/**
 * R36 §9 MANDATED DIVERGENCE TEST — REWRITTEN AS A CHARACTERIZATION, and this is
 * a DATED DEVIATION from the prereg, recorded in the R36 evidence file.
 *
 * The prereg mandated proving that a name carrying a canonical invisible
 * character "renders stripped text in the DOM while the captured POST body
 * contains the original wire value". That is UNOBSERVABLE on this wire, and the
 * reason is structural rather than a shell defect — three independent layers,
 * each verified against the pinned kernel source, not inferred:
 *
 *   1. WRITE. `valid_text` (src/workflow/registry.rs:94-100) calls
 *      `is_invisible` on the declared `name` (line 116), so a registration
 *      carrying U+200B is REFUSED with 400 `registry_identity_required`.
 *   2. READ. Even if one were written, the read seam strips it anyway:
 *      `sanitize_value_strings` at src/handlers/model_registry.rs:347 and :394.
 *   3. DISPLAY. The shell's INVISIBLE set (src/lib/sanitize.ts:12-13) is
 *      byte-identical to the kernel's `is_invisible` class, so the two agree.
 *
 * So no model-registry row can ever hold an invisible character, and a renderer
 * can never be handed one. The ORIGINAL test could only ever have passed by
 * stubbing the wire — which is exactly what the unit-level law in
 * tests/models-view.test.ts does, and what the verbatim-forwarding half of it
 * proves for real in the roundtrip test above.
 *
 * This characterization is STRONGER than the test it replaces: it does not check
 * that the shell sanitizes on the way to the DOM, it PROVES that the divergence
 * is unreachable by construction. The display/submission separation law itself
 * is unchanged and still pinned — see the note at the foot of this file.
 */
test('model_registry_invisible_name_is_refused_at_write_so_the_divergence_is_unreachable', async () => {
	expect(kernelPort, 'E2E_KERNEL_PORT must be the dedicated e2e port').toBe(8799);
	expect(kernelPort, 'this spec never targets the operator service port').not.toBe(8765);

	const fixtureId = 'acme-e2e-invisible';
	const rawName = `Acme${ZWSP}models`;

	const registration = await wirePost('/workflow/model-registry/register', {
		kind: 'learned',
		id: fixtureId,
		version: '1.0.0',
		name: rawName,
		output_vocabulary: ['choice'],
		artifact_digest: 'a'.repeat(64)
	});

	// The kernel refuses it. This is the whole point: the write seam is the
	// first of three layers, and it is enough on its own.
	expect(
		registration.status,
		`an invisible character in a declared name must be refused at write (${registration.status}: ${registration.text})`
	).toBe(400);
	expect(requiredObject(registration.body, 'the refusal body')['error']).toMatchObject({
		code: 'registry_identity_required'
	});

	// And the row therefore does not exist, so no detail read can deliver the
	// name to a renderer and no proposal can echo it back.
	const detail = await wireGet(`/workflow/model-registry/${fixtureId}@1.0.0`);
	expect(
		detail.status,
		`the refused row must not exist (${detail.status}: ${detail.text})`
	).toBe(404);
	expect(JSON.stringify(detail.body)).not.toContain(ZWSP);

	// The display law is untouched and still true at the unit level: stripping
	// REMOVES the byte, it does not substitute a space, so the word boundary is
	// destroyed rather than normalized. A renderer must not "repair" it.
	expect(stripInvisible(rawName)).toBe('Acmemodels');
	expect(stripInvisible(rawName)).not.toBe('Acme models');
});
