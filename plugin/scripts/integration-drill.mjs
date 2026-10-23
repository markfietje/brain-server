#!/usr/bin/env node
/**
 * III-8 · D5 — the OpenClaw integration, proved against the SHIPPED SIGNED
 * binary.
 *
 * ── WHAT THIS IS, AND WHY A UNIT SUITE CANNOT BE IT ─────────────────────────
 *
 * The plugin's own suite drives `fetch`. It proves the plugin's REQUEST
 * SHAPES. It cannot prove that the binary that actually ships answers them.
 * Gate 9 — "the OpenClaw integration still works against the shipped binary" —
 * names the SHIPPED BINARY, and since III-7 there IS one: the release
 * publishes `Brain-Shell.app.tar.gz`, its minisign `.sig`, and the
 * `latest.json` that names both.
 *
 * So this drill does the whole chain, in order, and refuses to report green
 * at every step it cannot measure:
 *
 *   1. fetch the manifest the APP fetches (`plugins.updater.endpoints[0]` of
 *      `shell/src-tauri/tauri.conf.json` — the app's own configuration, not a
 *      URL written here);
 *   2. fetch the artefact the manifest names;
 *   3. verify the artefact's minisign signature against the app's OWN PINNED
 *      public key — the same key the app's updater verifies with, read from
 *      the app's own configuration;
 *   4. check the signature's BINDING (which file, which version) and the
 *      kernel version the manifest announces;
 *   5. extract the bundle and run the kernel that ships INSIDE it;
 *   6. drive the plugin's REAL client (`src/brain-client.ts` — the production
 *      transport, not a mock) against that process: store a memory, read it
 *      back, and read the review queue;
 *   7. print the OBSERVATION — what the shipped binary answered, and what the
 *      plugin made of it.
 *
 * ── FAIL CLOSED (D6) ────────────────────────────────────────────────────────
 *
 *   exit 0  the drill ran, the chain held, and the observation is recorded;
 *   exit 1  the drill REFUSED: something did not hold, and it says what;
 *   exit 2  the drill COULD NOT RUN: the network, the toolchain, or the
 *           platform would not let it measure. **Never reported as green.**
 *
 * ── THE ANTI-VACUITY ARM: `--artifact` ──────────────────────────────────────
 *
 * `--artifact <path>` verifies a LOCAL file against the app's pinned key and
 * nothing else. A file that is not the shipped artefact — a local build, a
 * tampered copy, a signature made by a different key — is REFUSED, and the
 * refusal names the reason. That is what makes the drill's green mean
 * something: there is no way to point it at a binary of your choosing and
 * have it agree.
 *
 * Usage:
 *   node plugin/scripts/integration-drill.mjs                 # the full drill
 *   node plugin/scripts/integration-drill.mjs --artifact PATH # verify only
 */
import { createHash, createPublicKey, verify } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import {
	chmodSync,
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	readdirSync,
	rmSync,
	writeFileSync
} from 'node:fs';
import { register } from 'node:module';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'node:net';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const PLUGIN_ROOT = dirname(SCRIPT_DIR);
const REPO_ROOT = dirname(PLUGIN_ROOT);
const APP_CONFIG = join(REPO_ROOT, 'shell', 'src-tauri', 'tauri.conf.json');

/** The Ed25519 SubjectPublicKeyInfo prefix (RFC 8410) — 12 bytes. */
const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

/** The drill's marker: a sentence no corpus contains, so a hit IS the round trip. */
const DRILL_TEXT = 'the quokka fleet sails at midnight on the eleventh tide';

/**
 * The download ceiling, against the channel's own measurement: the published
 * artefact is 26,371,332 bytes (v1.29.28), so anything past 64 MiB is not the
 * release channel answering. Every answer is bounded, and the ceiling is
 * named when it fires.
 */
const MAX_DOWNLOAD_BYTES = 64 * 1024 * 1024;

class Refusal extends Error {}
class CouldNotRun extends Error {}

function fail(message) {
	throw new Refusal(message);
}

function couldNotRun(message) {
	throw new CouldNotRun(message);
}

/**
 * The app's OWN updater configuration: the pinned public key and the endpoint
 * the app fetches. Read from the app's configuration rather than written here,
 * so the drill and the app cannot drift apart.
 */
function readAppUpdater() {
	if (!existsSync(APP_CONFIG)) {
		couldNotRun(`the app's configuration is missing: ${APP_CONFIG}`);
	}
	const config = JSON.parse(readFileSync(APP_CONFIG, 'utf8'));
	const updater = config?.plugins?.updater;
	if (updater === undefined) {
		couldNotRun(`the app's configuration declares no updater: ${APP_CONFIG}`);
	}
	const pubkeyText = Buffer.from(String(updater.pubkey), 'base64').toString('utf8');
	const endpoint = updater.endpoints?.[0];
	if (typeof endpoint !== 'string' || endpoint === '') {
		couldNotRun(`the app's configuration declares no updater endpoint: ${APP_CONFIG}`);
	}
	return { pubkeyText, endpoint, requireSignedVersion: updater.requireSignedVersion === true };
}

/** A minisign PUBLIC key: `alg(2) || keynum(8) || key(32)`. */
function parsePublicKey(text) {
	const lines = text.split('\n').filter((line) => line.trim() !== '');
	if (lines.length < 2) {
		fail('the pinned public key is not a minisign public key');
	}
	const raw = Buffer.from(lines[1].trim(), 'base64');
	if (raw.length !== 42) {
		fail(`the pinned public key is ${raw.length} bytes, not the 42 a minisign key is`);
	}
	if (raw.subarray(0, 2).toString('ascii') !== 'Ed') {
		fail(`the pinned public key is not an Ed25519 minisign key`);
	}
	return { keynum: raw.subarray(2, 10), key: raw.subarray(10, 42) };
}

/**
 * A minisign signature: the primary line is `alg(2) || keynum(8) || sig(64)`
 * and the SECONDARY line is the RAW 64-byte signature — no algorithm marker
 * and no key id. (Measured 2026-10-22 against the published v1.29.29 chain:
 * the secondary line decodes to 64 bytes, and slicing it as a 74-byte blob
 * truncates the signature and refuses a perfectly good chain.)
 */
function parseSignature(text) {
	const lines = text.split('\n');
	const primary = Buffer.from((lines[1] ?? '').trim(), 'base64');
	const global = Buffer.from((lines[3] ?? '').trim(), 'base64');
	const trustedLine = lines.find((line) => line.startsWith('trusted comment: '));
	if (primary.length !== 74 || global.length !== 64 || trustedLine === undefined) {
		fail('the signature is not a minisign signature');
	}
	return {
		keynum: primary.subarray(2, 10),
		primarySig: primary.subarray(10, 74),
		globalSig: global,
		trustedComment: trustedLine.slice('trusted comment: '.length).trim()
	};
}

/**
 * Verify a minisign signature over `message` against a public key.
 *
 * The prehashed scheme (what the Tauri CLI writes, and what the app's updater
 * verifies): the primary signature is over BLAKE2b-512(message), and the
 * secondary is over `primarySig || trustedComment`. Both halves are checked —
 * a chain whose COMMENT signature does not verify is not the app's chain.
 */
function verifySignature(publicKey, signature, message) {
	const key = createPublicKey({
		key: Buffer.concat([ED25519_SPKI_PREFIX, publicKey.key]),
		format: 'der',
		type: 'spki'
	});
	const prehash = createHash('blake2b512').update(message).digest();
	const primaryOk = verify(null, prehash, key, signature.primarySig);
	const commentPayload = Buffer.concat([
		signature.primarySig,
		Buffer.from(signature.trustedComment, 'utf8')
	]);
	const commentOk = verify(null, commentPayload, key, signature.globalSig);
	const keynumOk = signature.keynum.equals(publicKey.keynum);
	return { primaryOk, commentOk, keynumOk };
}

/** The trusted comment's `key:value` fields. */
function trustedFields(comment) {
	const fields = {};
	for (const field of comment.split('\t')) {
		const separator = field.indexOf(':');
		if (separator > 0) fields[field.slice(0, separator)] = field.slice(separator + 1);
	}
	return fields;
}

/**
 * The drill talks to the app's release channel and to NOTHING else. Both URLs
 * it fetches are untrusted input BY DESIGN — the manifest endpoint is read
 * from the app's own configuration and the artefact URL is named by the
 * published manifest — so each is held to https, on a host the channel
 * actually uses, before anything is fetched. A config that names another
 * origin is not a network hiccup: it is REFUSED (exit 1), not shrugged at.
 */
function releaseChannelUrl(raw, what) {
	let url;
	try {
		url = new URL(String(raw));
	} catch {
		fail(`the ${what} is not a URL: ${raw}`);
	}
	if (url.protocol !== 'https:') {
		fail(`the ${what} is ${url.protocol} — the drill fetches https only`);
	}
	if (url.hostname !== 'github.com' && !url.hostname.endsWith('.githubusercontent.com')) {
		fail(`the ${what} names a host the release channel does not use: ${url.hostname}`);
	}
	return url;
}

async function fetchBytes(url, what) {
	releaseChannelUrl(url, what);
	let response;
	try {
		// The URL is untrusted input BY DESIGN — the endpoint comes from the
		// app's configuration and the artefact URL from the published manifest
		// — and `releaseChannelUrl` above holds it to https on the release
		// channel's hosts before anything is fetched. That guard is the
		// mitigation for the analyser's alert on this line: the origin
		// allowlist is not something it can see, so the alert is DISMISSED
		// with this comment's reasoning as its record (the gate-law register
		// carries the row).
		response = await fetch(url, { redirect: 'follow' });
	} catch (error) {
		couldNotRun(`the ${what} could not be fetched from ${url}: ${error?.message ?? error}`);
	}
	if (!response.ok) {
		couldNotRun(`the ${what} answered HTTP ${response.status} at ${url}`);
	}
	const body = Buffer.from(await response.arrayBuffer());
	if (body.length > MAX_DOWNLOAD_BYTES) {
		fail(
			`the ${what} is ${body.length} bytes — past the ${MAX_DOWNLOAD_BYTES}-byte ceiling, ` +
				'so it is not the release channel answering'
		);
	}
	return body;
}

/** A free TCP port, released before the kernel binds it. */
function freePort() {
	return new Promise((resolve, reject) => {
		const server = createServer();
		server.on('error', reject);
		server.listen(0, '127.0.0.1', () => {
			const address = server.address();
			const port = typeof address === 'object' && address !== null ? address.port : 0;
			server.close(() => resolve(port));
		});
	});
}

/** macOS stamps a provenance xattr on newly-copied executables; the kernel dies on it. */
function clearQuarantine(path) {
	if (process.platform !== 'darwin') return;
	for (const attribute of ['com.apple.provenance', 'com.apple.quarantine']) {
		spawnSync('xattr', ['-dr', attribute, path], { stdio: 'ignore' });
	}
}

/** Find the kernel that ships INSIDE the bundle: `…/Contents/MacOS/brain-server`. */
function findShippedKernel(root) {
	const stack = [root];
	while (stack.length > 0) {
		const current = stack.pop();
		for (const entry of readdirSync(current, { withFileTypes: true })) {
			const full = join(current, entry.name);
			if (!existsSync(full)) continue;
			if (
				!entry.isDirectory() &&
				entry.name === 'brain-server' &&
				basename(dirname(full)) === 'MacOS' &&
				basename(dirname(dirname(full))) === 'Contents'
			) {
				return full;
			}
			if (entry.isDirectory()) stack.push(full);
		}
	}
	return null;
}

/** Run the shipped kernel and wait for it to answer. */
async function startKernel(kernelPath, dataRoot) {
	const token = `iii8-drill-${process.pid}-${Date.now()}`;
	const tokenFile = join(dataRoot, 'auth-token');
	writeFileSync(tokenFile, `${token}\n`, { mode: 0o600 });
	chmodSync(tokenFile, 0o600);
	const port = await freePort();
	const child = spawn(
		kernelPath,
		[],
		{
			env: {
				PATH: process.env.PATH ?? '',
				BIND_HOST: '127.0.0.1',
				BIND_PORT: String(port),
				BRAIN_DB_PATH: join(dataRoot, 'brain.db'),
				AUTH_TOKEN_FILE: tokenFile
			},
			stdio: ['ignore', 'pipe', 'pipe']
		}
	);
	const log = [];
	child.stdout.on('data', (chunk) => log.push(String(chunk)));
	child.stderr.on('data', (chunk) => log.push(String(chunk)));
	let exited = null;
	child.on('exit', (code) => {
		exited = code;
	});
	const baseUrl = `http://127.0.0.1:${port}`;
	for (let attempt = 0; attempt < 60; attempt += 1) {
		if (exited !== null) {
			couldNotRun(
				`the shipped kernel exited with ${exited} before it answered: ${log.join('').slice(-600)}`
			);
		}
		try {
			const response = await fetch(`${baseUrl}/health`, {
				signal: AbortSignal.timeout(2000)
			});
			if (response.ok) {
				const body = await response.json();
				return { child, baseUrl, token, log, health: body };
			}
		} catch {
			// not up yet
		}
		await new Promise((resolve) => setTimeout(resolve, 500));
	}
	couldNotRun(`the shipped kernel never answered /health: ${log.join('').slice(-600)}`);
	return undefined;
}

/** The signature that must accompany an artefact: `<artefact>.sig`, base64'd. */
function readSignatureBeside(artefactPath) {
	const beside = `${artefactPath}.sig`;
	if (!existsSync(beside)) {
		fail(`the artefact carries no signature beside it: ${beside}`);
	}
	const raw = readFileSync(beside, 'utf8').trim();
	try {
		return Buffer.from(raw, 'base64').toString('utf8');
	} catch {
		fail(`the signature beside the artefact is not base64: ${beside}`);
	}
	return '';
}

/** The `--artifact` arm: verify a LOCAL file against the app's pinned key,
 *  and against the manifest the app would fetch — so a local artefact cannot
 *  stand in for a published one. */
function verifyLocalArtefact(artefactPath) {
	const updater = readAppUpdater();
	const publicKey = parsePublicKey(updater.pubkeyText);
	const message = readFileSync(artefactPath);
	const beside = `${artefactPath}.sig`;
	if (!existsSync(beside)) {
		fail(`the artefact carries no signature beside it: ${beside}`);
	}
	const signatureText = readFileSync(beside, 'utf8').trim();
	const signature = parseSignature(
		Buffer.from(signatureText, 'base64').toString('utf8')
	);
	const verdict = verifySignature(publicKey, signature, message);
	if (!verdict.keynumOk) {
		fail(
			`the artefact's signature was made by key ${signature.keynum.toString('hex').toUpperCase()}, ` +
				`not by the app's pinned key ${publicKey.keynum.toString('hex').toUpperCase()}`
		);
	}
	if (!verdict.primaryOk) {
		fail("the artefact's signature is not the pinned key's signature");
	}
	if (!verdict.commentOk) {
		fail("the artefact's comment signature is not the pinned key's signature");
	}
	const fields = trustedFields(signature.trustedComment);
	if (fields.file !== basename(artefactPath)) {
		fail(
			`the signature names another file: ${fields.file} != ${basename(artefactPath)}`
		);
	}
	return { updater, publicKey, signature, signatureText, fields };
}

/**
 * The manifest the app would fetch, and the answer it must give about a
 * `--artifact` run: the release still names this artefact, and the embedded
 * signature is byte-identical to the one beside the local file.
 */
async function crossCheckAgainstManifest(updater, artefactPath, signatureText) {
	const manifest = JSON.parse(
		(await fetchBytes(updater.endpoint, 'update manifest')).toString('utf8')
	);
	const platform = manifest.platforms?.['darwin-aarch64'] ?? manifest.platforms?.['darwin-x86_64'];
	if (platform === undefined) {
		fail('the published manifest answers for no Apple platform');
	}
	if (basename(platform.url) !== basename(artefactPath)) {
		fail(
			`the release now names another artefact: ${basename(platform.url)} != ${basename(artefactPath)}`
		);
	}
	if (platform.signature !== signatureText) {
		fail('the signature beside the local artefact is not the one the release published');
	}
	return manifest;
}


async function main() {
	const artifactArgument = process.argv.indexOf('--artifact');
	if (artifactArgument !== -1) {
		const artefactPath = process.argv[artifactArgument + 1];
		if (artefactPath === undefined) {
			couldNotRun('--artifact needs a path');
		}
		const { updater, publicKey, signatureText, fields } = verifyLocalArtefact(artefactPath);
		const manifest = await crossCheckAgainstManifest(updater, artefactPath, signatureText);
		console.log('VERIFIED');
		console.log(`  the artefact:         ${artefactPath} (a local copy — its identity is the signature)`);
		console.log(`  the release:          ${manifest.release_tag}, manifest version ${manifest.version}`);
		console.log(`  the signature:        the app's pinned key's (${publicKey.keynum.toString('hex').toUpperCase()})`);
		console.log(`  the binding:          file:${fields.file} version:${fields.version}`);
		return 0;
	}

	const updater = readAppUpdater();
	const publicKey = parsePublicKey(updater.pubkeyText);
	console.log(`the app's endpoint:  ${updater.endpoint}`);
	console.log(
		`the app's pinned key: ${publicKey.keynum.toString('hex').toUpperCase()}`
	);

	const manifest = JSON.parse(
		(await fetchBytes(updater.endpoint, 'update manifest')).toString('utf8')
	);
	const platform = manifest.platforms?.['darwin-aarch64'] ?? manifest.platforms?.['darwin-x86_64'];
	if (platform === undefined) {
		fail('the published manifest answers for no Apple platform');
	}
	const artefactUrl = platform.url;
	const manifestSignature = platform.signature;
	if (typeof artefactUrl !== 'string' || typeof manifestSignature !== 'string') {
		fail('the published manifest names no artefact or no signature');
	}
	console.log(`the manifest:         version ${manifest.version}, release ${manifest.release_tag}`);
	console.log(`the artefact:         ${artefactUrl}`);

	const dataRoot = mkdtempSync(join(tmpdir(), 'brain-shell-iii8-drill-'));
	let kernel = null;
	try {
		const artefact = await fetchBytes(artefactUrl, 'artefact');
		const signature = parseSignature(Buffer.from(manifestSignature, 'base64').toString('utf8'));
		const verdict = verifySignature(publicKey, signature, artefact);
		if (!verdict.keynumOk) {
			fail(
				`the artefact's signature was made by key ${signature.keynum.toString('hex').toUpperCase()}, ` +
					`not by the app's pinned key ${publicKey.keynum.toString('hex').toUpperCase()}`
			);
		}
		if (!verdict.primaryOk) {
			fail("the artefact's signature is not the pinned key's signature");
		}
		if (!verdict.commentOk) {
			fail("the artefact's comment signature is not the pinned key's signature");
		}
		console.log(
			`the chain:            VERIFIED against the app's pinned key ` +
				`(primary + comment signature)`
		);

		const fields = trustedFields(signature.trustedComment);
		if (fields.file !== basename(artefactUrl)) {
			fail(`the signature names another file: ${fields.file} != ${basename(artefactUrl)}`);
		}
		if (fields.version !== manifest.version) {
			fail(
				`the signature was made for version ${fields.version}, not the announced ${manifest.version}`
			);
		}
		console.log(
			`the binding:          file:${fields.file} version:${fields.version} timestamp:${fields.timestamp}`
		);

		const extracted = join(dataRoot, 'extracted');
		mkdirSync(extracted);
		// The VERIFIED bytes go straight into the extractor: the drill never
		// writes the download to the filesystem, so no window exists in which
		// network data sits on disk as a file.
		const untar = spawnSync('tar', ['-xzf', '-', '-C', extracted], {
			input: artefact,
			encoding: 'utf8'
		});
		if (untar.status !== 0) {
			couldNotRun(`the artefact could not be unpacked: ${untar.stderr}`);
		}
		clearQuarantine(extracted);
		const kernelPath = findShippedKernel(extracted);
		if (kernelPath === null) {
			couldNotRun('the unpacked bundle carries no kernel at Contents/MacOS/brain-server');
		}
		const version = spawnSync(kernelPath, ['--version'], { encoding: 'utf8' }).stdout.trim();
		console.log(`the shipped kernel:   ${kernelPath}`);
		console.log(`the shipped kernel:   ${version}`);
		if (!version.includes(String(manifest.kernel_version))) {
			fail(
				`the shipped kernel is ${version}, but the manifest announces ${manifest.kernel_version}`
			);
		}

		kernel = await startKernel(kernelPath, dataRoot);
		console.log(`the kernel answered:  ${JSON.stringify(kernel.health)}`);

		// The plugin's REAL client — the production transport, imported from the
		// plugin's own sources, against the process that ships.
		register('./ts-resolve-hooks.mjs', import.meta.url);
		const { BrainClient } = await import('../src/brain-client.ts');
		const { resolveConfig } = await import('../src/config.ts');
		const config = resolveConfig({
			enabled: true,
			baseUrl: kernel.baseUrl,
			authToken: kernel.token
		});
		const client = new BrainClient(config);

		const healthy = await client.health();
		if (!healthy) {
			fail("the plugin's client could not read the shipped kernel's health");
		}
		const stored = await client.store({
			title: 'III-8 integration drill',
			content: DRILL_TEXT,
			domain: 'global'
		});
		if (stored.status !== 'created' && stored.status !== 'duplicate') {
			fail(`the plugin's client could not store a memory: ${JSON.stringify(stored)}`);
		}
		const recalled = await client.recall({ query: 'quokka fleet midnight tide', limit: 3 });
		const hit = recalled.hits.find((candidate) => candidate.content.includes('quokka fleet'));
		if (hit === undefined) {
			fail(
				`the memory the plugin stored was not recalled by the shipped kernel: ${JSON.stringify(recalled)}`
			);
		}
		const proposals = await client.listProposals({ limit: 5 });

		console.log('');
		console.log('OBSERVATION');
		console.log(`  the release:            ${manifest.release_tag} (manifest version ${manifest.version})`);
		console.log(`  the kernel shipped:     ${version}`);
		console.log(`  the kernel answered:    ${JSON.stringify(kernel.health)}`);
		console.log(`  the plugin stored:      id ${stored.id}, status ${stored.status}`);
		console.log(`  the plugin recalled:    decision "${recalled.decision}", ${recalled.hits.length} hit(s)`);
		console.log(`  the recalled content:   ${JSON.stringify(hit.content)}`);
		console.log(`  the recall score:       ${hit.score}`);
		console.log(`  the review queue read:  ${proposals.length} row(s)`);
		console.log('');
		console.log('PASS — the OpenClaw integration works against the SHIPPED SIGNED binary.');
		return 0;
	} finally {
		if (kernel?.child !== undefined) {
			kernel.child.kill('SIGTERM');
			await new Promise((resolve) => setTimeout(resolve, 300));
			if (kernel.child.exitCode === null) kernel.child.kill('SIGKILL');
		}
		rmSync(dataRoot, { recursive: true, force: true });
	}
}

try {
	process.exit(await main());
} catch (error) {
	if (error instanceof Refusal) {
		console.error(`REFUSED: ${error.message}`);
		process.exit(1);
	}
	if (error instanceof CouldNotRun) {
		console.error(`COULD NOT RUN: ${error.message}`);
		process.exit(2);
	}
	console.error(`COULD NOT RUN: ${error?.stack ?? error}`);
	process.exit(2);
}
