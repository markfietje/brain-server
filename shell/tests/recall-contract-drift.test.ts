import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { shellRoot } from './fixtures';

/**
 * I7 — the generated-wire drift gate, pointed at what THIS PHASE assumes.
 *
 * The premise first, because it contradicts the roadmap this phase was handed:
 * the drift gate ALREADY runs in this phase's lane, twice. `shell.yml` runs
 * `pnpm test` (whose vitest include is `tests/**\/*.test.ts`, so
 * `drift-gate.test.ts` is in it) and it also regenerates and `cmp`s the schema
 * inline. `ci-gates.test.ts` pins both lines. So the ROADMAP's "not in a lane
 * that runs" is false, and the pin below asserts the two lines exist rather
 * than adding a third.
 *
 * The gap that IS real is narrower and more dangerous. The drift gate proves
 * `schema.d.ts` is byte-equal to what `openapi.yaml` generates. It does NOT
 * prove the contract still says what this phase's code assumes. Delete
 * `decision` from the spec and `schema.d.ts` regenerates perfectly, the drift
 * gate stays green, and `shell/src-tauri/src/recall.rs` — whose whole taxonomy
 * hangs on that field being REQUIRED — starts answering `ContractViolation` on
 * every successful recall. The generated file is the artefact, so this pin
 * reads IT rather than the spec prose: a contract change fails here at the same
 * commit it changes the wire.
 */

const SCHEMA = readFileSync(join(shellRoot, 'src', 'lib', 'api', 'schema.d.ts'), 'utf8');

/**
 * The `/recall` post's 200 response body.
 *
 * The route entry does NOT inline its response — the generator emits
 * `post: operations["recall"]` and puts the shape in a separate `recall`
 * operation block. Slicing the route therefore finds the route and nothing
 * else, which is how the first version of this extraction returned a block
 * with no `decision` in it. Following the reference is the whole fix, and the
 * anti-vacuity assertions below are what keep a silent miss from becoming a
 * vacuous pin.
 */
function recallOkBlock(): string {
	const route = SCHEMA.indexOf('"/recall": {');
	expect(route, '/recall must exist in the generated contract').toBeGreaterThan(-1);
	const routeBlock = SCHEMA.slice(route, route + 1200);
	expect(routeBlock, 'the /recall operation must be a POST').toContain('post: operations["recall"]');

	const operation = SCHEMA.indexOf('    recall: {');
	expect(operation, 'the recall operation block must exist').toBeGreaterThan(-1);
	const block = SCHEMA.slice(operation, operation + 4000);
	expect(block).toContain('requestBody');
	return block;
}

test('the_recall_decision_enum_is_exactly_the_published_one', () => {
	const block = recallOkBlock();

	// The enum, read from the GENERATED file. This is the string the Rust
	// `Decision` enum mirrors and the string `low_confidence_is_an_answer` pins
	// on the Swift side; if the kernel grows or renames a value, all three move
	// or this goes red.
	expect(block).toContain('decision: "ok" | "low_confidence"');

	// And it is a CLOSED set, not a prefix: a third value added to the spec
	// must not slip past an assertion that only looks for the two we know.
	const enumLine = /decision: ([^;]+);/.exec(block)?.[1];
	expect(enumLine, 'the decision property must be a closed union').toBe('"ok" | "low_confidence"');
	expect(enumLine?.match(/"/g)?.length).toBe(4);
});

test('recall_still_carries_confidence_and_provenance_fields_the_core_reads', () => {
	const block = recallOkBlock();

	// `confidence` reaches the client only through `telemetry`, and only
	// because the core asks for `provenance: true`. If either link is cut, an
	// abstention renders with no number attached and nothing else notices.
	expect(block).toContain('telemetry?: components["schemas"]["SearchTelemetry"]');
	expect(SCHEMA).toContain('SearchTelemetry:');

	// The hits array and its item type — the projection's subject.
	// The generator writes `T[]`, not `Array<T>` — reading the artefact means
	// reading what the generator emits, not what one imagines it emits.
	expect(block).toContain('hits?: components["schemas"]["RecallHit"][]');
	expect(SCHEMA).toContain('RecallHit:');

	// `SearchTelemetry.confidence`, located inside that schema's own block so
	// a `confidence` on some other telemetry type cannot satisfy it.
	const telemetry = SCHEMA.slice(SCHEMA.indexOf('SearchTelemetry:'), SCHEMA.indexOf('SearchTelemetry:') + 1200);
	expect(telemetry, 'SearchTelemetry must carry confidence').toContain('confidence?: number');
});

test('the_querydoc_still_carries_exactly_the_fields_the_core_mints', () => {
	const queryDoc = SCHEMA.slice(SCHEMA.indexOf('QueryDoc:'), SCHEMA.indexOf('QueryDoc:') + 2500);

	// The four the core is allowed to set, and the two the core's bounds read.
	// `v` is NOT optional in the generated type: the spec gives it a default, so
	// openapi-typescript emits `v: number`. Asserting `v?: number` was wrong
	// about the generator, and the pin that cannot read the artefact accurately
	// is worse than no pin.
	for (const field of ['query?: string', 'domain?: string', 'limit?: number', 'provenance?: boolean', 'v: number']) {
		expect(queryDoc, `QueryDoc must still carry ${field}`).toContain(field);
	}

	// Anti-vacuity: the extraction found a real block. A slice that returned
	// empty would satisfy nothing above, so assert it reached into content.
	expect(queryDoc.length).toBeGreaterThan(200);
	expect(queryDoc).toContain('QueryDoc');
});

test('this_phase_lives_in_a_lane_that_re_derives_the_wire', () => {
	const workflow = readFileSync(
		join(shellRoot, '..', '.github', 'workflows', 'shell.yml'),
		'utf8'
	);

	// The two existing arms. Adding a third would be churn; asserting these
	// exist is what stops them being quietly deleted.
	expect(workflow).toContain('pnpm exec openapi-typescript ../openapi.yaml -o "$generated"');
	expect(workflow).toContain('cmp src/lib/api/schema.d.ts "$generated"');
	expect(workflow).toContain('pnpm test');

	// And the lane is actually TRIGGERED by this phase's files. The recall path
	// lives under shell/, and a paths filter that missed it would leave the
	// whole phase ungated on the runner that matters.
	expect(workflow).toContain("- 'shell/**'");
	expect(workflow).toContain("- 'openapi.yaml'");

	// The Rust core's pins run in the same lane (added in I2) — the drift gate
	// alone would leave the core's six states unguarded.
	expect(workflow).toContain('cargo test --manifest-path src-tauri/Cargo.toml');
});
