import { readFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash, generateKeyPairSync, sign } from 'node:crypto';
import { describe, expect, test } from 'vitest';

/**
 * III-8 · D5 — the integration drill's own pins.
 *
 * The drill (`scripts/integration-drill.mjs`) is the only thing that closes
 * gate 9, because a suite that mocks `fetch` proves the plugin's REQUEST
 * SHAPES and never the binary that ships. These pins hold the drill to the
 * properties that make its green mean something:
 *
 *   - it verifies against the app's OWN pinned key, read from the app's own
 *     configuration — the key the app's updater verifies with;
 *   - it fetches the manifest the APP fetches;
 *   - it drives the plugin's REAL client, and speaks in outcomes;
 *   - a file that is not the shipped artefact is REFUSED, with a reason —
 *     including an artefact signed by a DIFFERENT key, which is the proof
 *     that "verified" means the pinned key and not merely "a signature".
 *
 * The refusal arms run here, in this lane, with no network and no private
 * key: they exercise the drill's verification directly, which is the part
 * that must never learn to agree.
 */

const pluginRoot = new URL('..', import.meta.url).pathname;
const repoRoot = new URL('../..', import.meta.url).pathname;
const drill = join(pluginRoot, 'scripts', 'integration-drill.mjs');

const readRepo = (path: string): string => readFileSync(join(repoRoot, path), 'utf8');

/** The app's own pinned key text, decoded from the app's configuration. */
const appConfig = JSON.parse(readRepo('shell/src-tauri/tauri.conf.json')) as {
	plugins?: { updater?: { pubkey?: string; endpoints?: string[] } };
};
const pinnedKeyText = Buffer.from(appConfig.plugins?.updater?.pubkey ?? '', 'base64').toString(
	'utf8'
);
const pinnedKeynum = Buffer.from(
	pinnedKeyText.split('\n').filter((line) => line.trim() !== '')[1] ?? '',
	'base64'
)
	.subarray(2, 10)
	.toString('hex')
	.toUpperCase();

/** A minisign-format public key minted from an Ed25519 key Node generates. */
function mintKeypair(): {
	pubkeyText: string;
	keynum: BufferLike;
	privateKey: ReturnType<typeof generateKeyPairSync>['privateKey'];
} {
	const { publicKey, privateKey } = generateKeyPairSync('ed25519');
	const der = publicKey.export({ format: 'der', type: 'spki' }) as BufferLike;
	const key = der.subarray(der.length - 32);
	const keynum = Buffer.from('ffffffffffffffff', 'hex');
	const blob = Buffer.concat([Buffer.from('Ed', 'ascii'), keynum, key]);
	return {
		pubkeyText: `untrusted comment: minted for the drill's red-proof\n${blob.toString('base64')}\n`,
		keynum,
		privateKey
	};
}

/** Sign bytes the way the Tauri CLI's minisign does (prehashed, two halves). */
function minisignSign(
	privateKey: ReturnType<typeof generateKeyPairSync>['privateKey'],
	keynum: BufferLike,
	message: BufferLike,
	trustedComment: string
): string {
	const primary = sign(null, createHash('blake2b512').update(message).digest(), privateKey);
	const payload = Buffer.concat([primary, Buffer.from(trustedComment, 'utf8')]);
	const secondary = sign(null, payload, privateKey);
	const primaryBlob = Buffer.concat([Buffer.from('Ed', 'ascii'), keynum, primary]);
	return [
		'untrusted comment: signature from a minted key',
		primaryBlob.toString('base64'),
		`trusted comment: ${trustedComment}`,
		secondary.toString('base64')
	].join('\n');
}

describe('the integration drill refuses what is not the shipped artefact', () => {
	const writeCase = (
		name: string,
		artefact: string,
		signatureText: string | null
	): string => {
		const scratch = mkdtempSync(join(tmpdir(), 'iii8-drill-'));
		const artefactPath = join(scratch, name);
		writeFileSync(artefactPath, artefact);
		if (signatureText !== null) {
			// The published `.sig` carries the base64 of the minisign TEXT.
			writeFileSync(`${artefactPath}.sig`, Buffer.from(signatureText).toString('base64'));
		}
		return artefactPath;
	};

	const runDrill = (artefactPath: string): { status: number | null; output: string } => {
		const run = spawnSync('node', [drill, '--artifact', artefactPath], {
			cwd: pluginRoot,
			encoding: 'utf8',
			timeout: 120_000
		});
		return { status: run.status, output: `${run.stdout}\n${run.stderr}` };
	};

	test('an artefact signed by a DIFFERENT key is refused, naming both keys', () => {
		const minted = mintKeypair();
		const name = 'Brain-Shell.app.tar.gz';
		const message = Buffer.from('an artefact that is not the shipped one');
		const signature = minisignSign(
			minted.privateKey,
			minted.keynum,
			message,
			`timestamp:1\tfile:${name}\tversion:0.1.1`
		);
		const artefactPath = writeCase(name, message.toString('latin1'), signature);
		const { status, output } = runDrill(artefactPath);
		expect(status, `the drill accepted an artefact signed by another key:\n${output}`).toBe(1);
		expect(output).toContain("not by the app's pinned key");
		expect(output.toUpperCase()).toContain('FFFFFFFFFFFFFFFF');
		expect(output).toContain(pinnedKeynum);
		rmSync(join(artefactPath, '..'), { recursive: true, force: true });
	});

	test('a signature over different bytes is refused as not the pinned key\'s', () => {
		const minted = mintKeypair();
		const name = 'Brain-Shell.app.tar.gz';
		// A signature over DIFFERENT bytes than the artefact beside it.
		const signature = minisignSign(
			minted.privateKey,
			minted.keynum,
			Buffer.from('the bytes that were signed'),
			`timestamp:1\tfile:${name}\tversion:0.1.1`
		);
		const artefactPath = writeCase(
			name,
			'the bytes that were shipped (different)',
			signature
		);
		const { status, output } = runDrill(artefactPath);
		expect(status, `the drill accepted an artefact whose signature does not hold:\n${output}`).toBe(
			1
		);
		expect(output).toContain('pinned key');
		rmSync(join(artefactPath, '..'), { recursive: true, force: true });
	});

	test('an artefact with no signature beside it is refused', () => {
		const artefactPath = writeCase('Brain-Shell.app.tar.gz', 'unsigned bytes', null);
		const { status, output } = runDrill(artefactPath);
		expect(status, `the drill accepted an unsigned artefact:\n${output}`).toBe(1);
		expect(output).toContain('carries no signature beside it');
		rmSync(join(artefactPath, '..'), { recursive: true, force: true });
	});
});

describe('the drill checks the binding, not only the bytes', () => {
	const source = readFileSync(drill, 'utf8');

	// The binding arms' RED-PROOFS were run against the REAL published chain
	// on 2026-10-22, because a local mint cannot reach them: the binding is
	// only authenticated by the comment signature, which needs the pinned
	// PRIVATE key.
	//   (1) the genuine `Brain-Shell.app.tar.gz`, bytes and signature intact,
	//       RENAMED to `app.tar.gz` →
	//       `REFUSED: the signature names another file: Brain-Shell.app.tar.gz != app.tar.gz`
	//   (2) the same artefact with ONE byte changed →
	//       `REFUSED: the artefact's signature is not the pinned key's signature`
	// Recorded here structurally; behaviourally, the arms are exercised by the
	// drill's own verification on every run.
	test('it refuses an artefact whose signature binds another file or version', () => {
		expect(source).toContain('names another file');
		expect(source).toContain('not the announced');
	});

	test('it refuses an artefact whose primary signature does not verify', () => {
		expect(source).toContain("is not the pinned key's signature");
	});
});

describe('the drill is wired to the app, not to this tree', () => {
	const source = readFileSync(drill, 'utf8');

	test('it verifies against the app\'s OWN pinned key', () => {
		expect(
			source.includes('tauri.conf.json'),
			'the drill must read the key from the app configuration'
		).toBe(true);
		expect(source).toContain('pubkey');
		expect(
			/updater\.key/.test(source),
			'the drill must not read a private key'
		).toBe(false);
	});

	test('it fetches the manifest the APP fetches', () => {
		expect(source).toContain('endpoints');
		expect(source).toContain('latest.json');
	});

	test('it drives the plugin\'s REAL client and speaks in outcomes', () => {
		expect(source).toContain('brain-client.ts');
		expect(source).toContain('new BrainClient(');
		expect(source).toContain('/health');
		for (const word of ['REFUSED', 'COULD NOT RUN', 'OBSERVATION', 'PASS']) {
			expect(source, `the drill must speak in outcomes: missing "${word}"`).toContain(word);
		}
	});
});
