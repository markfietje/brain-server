import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterAll, afterEach, expect, test } from 'vitest';
import { kernelRoot, shellRoot } from './fixtures';

/**
 * III-4 — the updater's pins: the trust anchor, the transport, the webview's
 * authority, and the manifest writer.
 * III-7 — the DISTRIBUTION ARTEFACT: the release must publish what the
 * updater can fetch, and the key that signs it must be the key the app ships.
 *
 * The writer is driven through its CLI (a subprocess), NOT through an import:
 * a module that does not exist yet must fail ONE pin with a readable text,
 * not take the whole file down with a resolution error and hide the other
 * seven. That is why nothing here imports `scripts/updater.mjs`.
 *
 * The signature fixture below is a SYNTHESIZED `.sig` FILE (the right shape:
 * an untrusted comment, a 74-byte signature packet, the trusted comment, a
 * 64-byte global signature). The writer checks the signature's BINDING —
 * which artifact it names and which version it was made for — because the
 * CRYPTOGRAPHIC verification is the app's, at install time, against the pinned
 * key (and `an_update_signed_by_a_key_that_is_not_the_pinned_key_is_refused`
 * pins that, in the Rust core, against the plugin's own verifier).
 */

const scriptPath = join(shellRoot, 'scripts', 'updater.mjs');
const signScriptPath = join(kernelRoot, 'scripts', 'release-app-sign.sh');
const releaseWorkflowPath = join(kernelRoot, '.github', 'workflows', 'release.yml');
const configPath = join(shellRoot, 'src-tauri', 'tauri.conf.json');
const macosConfigPath = join(shellRoot, 'src-tauri', 'tauri.macos.conf.json');
const capabilityPath = join(shellRoot, 'src-tauri', 'capabilities', 'main.json');

/**
 * A release tag that EXISTS in this repository, so the writer's tag check
 * can be exercised against a tag that resolves and against one that does
 * not. The tree's own tags are the ground truth — the writer resolves tags
 * with `git rev-parse`, so a fabricated tag name would be a fixture that
 * cannot be true.
 */
const RELEASED_TAG = 'v1.29.26';

const scratch: string[] = [];

afterEach(() => {
	for (const directory of scratch.splice(0)) {
		rmSync(directory, { recursive: true, force: true });
	}
});

// The fixture keypair outlives the per-test scratch — it is one key for the
// whole file — and it is removed when the file is done.
afterAll(() => {
	if (fixtureKeyDir !== null) rmSync(fixtureKeyDir, { recursive: true, force: true });
});

function tempDir(prefix = 'brain-updater-'): string {
	const directory = mkdtempSync(join(tmpdir(), prefix));
	scratch.push(directory);
	return directory;
}

/**
 * A REAL minisign signature over the artefact, made by the Tauri CLI.
 *
 * The fixture used to SYNTHESIZE the `.sig` file — the right shape: an
 * untrusted comment, a 74-byte packet, the trusted comment, a 64-byte global
 * signature. That is a fixture of the SHAPE, not of the THING, and it left a
 * real gap: **nobody had ever run the writer against a signature the Tauri
 * CLI actually produces.** The writer parses the trusted comment to learn
 * which artefact the signature names and which version it was made for, and
 * a hand-written imitation of that comment proves the parser against the
 * imitation.
 *
 * So the fixture now MINTS a keypair with the CLI and signs the artefact with
 * it, and the writer reads the real trusted comment
 * (`timestamp:…\tfile:…\tversion:…`). The version arm stays reachable
 * because the CLI's `--app-version` sets that field.
 *
 * The signature is base64 of the four-line minisign text — the CLI writes it
 * that way, and so does the bundler beside the artefact it built.
 */
const FIXTURE_KEY_PASSWORD = 'brain-shell-fixture-password';

let fixtureKey: { secretKey: string; publicKey: string } | null = null;
let fixtureKeyDir: string | null = null;

/**
 * ONE fixture keypair for the whole file: minting costs a CLI invocation.
 *
 * The key's directory is NOT one of the per-test scratch directories — the
 * scratch is emptied after EVERY test, and a key that vanished between two
 * tests would be a key the second test could not sign with.
 */
function fixtureSigningKey(): { secretKey: string; publicKey: string } {
	if (fixtureKey !== null && existsSync(fixtureKey.secretKey)) return fixtureKey;
	fixtureKeyDir = mkdtempSync(join(tmpdir(), 'brain-updater-fixturekey-'));
	const secretKey = join(fixtureKeyDir, 'fixture.key');
	const minted = spawnSync(
		'pnpm',
		['tauri', 'signer', 'generate', '--ci', '-w', secretKey, '-p', FIXTURE_KEY_PASSWORD],
		{ cwd: shellRoot, encoding: 'utf8' }
	);
	if (minted.status !== 0) {
		throw new Error(
			`the Tauri CLI could not mint the fixture keypair: ${minted.stderr}\n` +
				'Without a real key the writer would only ever see a hand-written signature.'
		);
	}
	fixtureKey = { secretKey, publicKey: `${secretKey}.pub` };
	return fixtureKey;
}

/** Sign the artefact with the fixture key, for the given version. */
function signFixtureArtifact(artifact: string, version: string): void {
	const key = fixtureSigningKey();
	const signed = spawnSync(
		'pnpm',
		[
			'tauri',
			'signer',
			'sign',
			'--app-version',
			version,
			'-f',
			key.secretKey,
			'-p',
			FIXTURE_KEY_PASSWORD,
			artifact
		],
		{ cwd: shellRoot, encoding: 'utf8' }
	);
	if (signed.status !== 0) {
		throw new Error(
			`the Tauri CLI could not sign the fixture artefact: ${signed.stderr}\n` +
				'Without a REAL signature the writer would only ever see a hand-written one.'
		);
	}
}

/** A minimal but real `Info.plist` for a built `.app`. */
function infoPlist(version: string): string {
	return [
		'<?xml version="1.0" encoding="UTF-8"?>',
		'<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
		'<plist version="1.0">',
		'<dict>',
		'\t<key>CFBundleIdentifier</key>',
		'\t<string>app.brain.shell</string>',
		'\t<key>CFBundleShortVersionString</key>',
		`\t<string>${version}</string>`,
		'</dict>',
		'</plist>',
		''
	].join('\n');
}

type Fixture = {
	root: string;
	staged: string;
	artifact: string;
	bundle: string;
	signature: string;
	manifest: string;
	tag: string;
};

type FixtureOptions = {
	kernelCommit?: string;
	kernelVersion?: string;
	artifactVersion?: string;
	signedVersion?: string;
	shellVersion?: string;
	tag?: string;
};

/**
 * A release fixture: a git tree with a commit, the staged sidecar record
 * that names it, a built `.app` carrying its own version, and the updater
 * artifact with its signature beside it.
 */
function fixture(options: FixtureOptions = {}): Fixture {
	const root = tempDir('brain-updater-root-');
	const kernelVersion = options.kernelVersion ?? '1.29.23';
	const artifactVersion = options.artifactVersion ?? '0.1.1';
	const shellVersion = options.shellVersion ?? '0.1.1';
	const tag = options.tag ?? RELEASED_TAG;

	// The tree the writer checks: a clean repository with ONE commit.
	mkdirSync(join(root, 'shell', 'src-tauri', 'binaries'), { recursive: true });
	mkdirSync(join(root, 'shell', 'src-tauri'), { recursive: true });
	// The staged record is an OUTPUT, never a source — the real repository
	// ignores it for the same reason, so the fixture ignores it too and the
	// tree stays clean while the record names the commit.
	writeFileSync(join(root, '.gitignore'), 'shell/src-tauri/binaries/\n');
	writeFileSync(
		join(root, 'shell', 'src-tauri', 'tauri.conf.json'),
		`${JSON.stringify({ version: shellVersion }, null, 2)}\n`
	);
	const git = (args: string[]): string =>
		execFileSync('git', args, {
			cwd: root,
			encoding: 'utf8',
			env: {
				...process.env,
				GIT_AUTHOR_NAME: 'fixture',
				GIT_AUTHOR_EMAIL: 'fixture@example.invalid',
				GIT_COMMITTER_NAME: 'fixture',
				GIT_COMMITTER_EMAIL: 'fixture@example.invalid'
			}
		}).trim();
	git(['init', '--quiet']);
	git(['add', '--all']);
	git(['-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'fixture']);
	const head = git(['rev-parse', 'HEAD']);
	// The release TAG, and it is a TAG: the manifest's download URL names
	// it, and the writer resolves it with `git rev-parse` (III-7's D1). A
	// fixture without a tag could not exercise the check at all.
	git(['tag', tag]);

	const staged = join(root, 'shell', 'src-tauri', 'binaries');
	writeFileSync(
		join(staged, 'SIDECAR_PROVENANCE.json'),
		`${JSON.stringify(
			{
				schema: 1,
				kernel_commit: options.kernelCommit ?? head,
				kernel_describe: 'fixture',
				kernel_version: kernelVersion,
				x_api_version: kernelVersion,
				source_dirty: false,
				built_at: '2026-10-20T00:00:00Z',
				artifacts: []
			},
			null,
			2
		)}\n`
	);

	// The built app and the updater artifact, with the artifact's signature
	// beside it — exactly where the bundler writes it, and made by the same
	// CLI that makes the real one.
	const work = tempDir('brain-updater-build-');
	const bundle = join(work, 'Brain Shell.app');
	mkdirSync(join(bundle, 'Contents'), { recursive: true });
	writeFileSync(join(bundle, 'Contents', 'Info.plist'), infoPlist(artifactVersion));
	const artifact = join(work, 'Brain Shell.app.tar.gz');
	writeFileSync(artifact, 'the updater artifact bytes\n');
	signFixtureArtifact(artifact, options.signedVersion ?? artifactVersion);
	const signature = `${artifact}.sig`;
	if (!existsSync(signature)) {
		throw new Error(`the Tauri CLI wrote no signature beside ${artifact}`);
	}

	return { root, staged, artifact, bundle, signature, manifest: join(work, 'latest.json'), tag };
}

type Run = { status: number | null; stdout: string; stderr: string };

function run(args: string[]): Run {
	const result = spawnSync('node', [scriptPath, ...args], { encoding: 'utf8' });
	return {
		status: result.status,
		stdout: result.stdout ?? '',
		stderr: result.stderr ?? ''
	};
}

function write(fixtureValue: Fixture, extra: string[] = []): Run {
	return run([
		'write',
		'--root',
		fixtureValue.root,
		'--staged',
		fixtureValue.staged,
		'--artifact',
		fixtureValue.artifact,
		'--bundle',
		fixtureValue.bundle,
		'--release-tag',
		fixtureValue.tag,
		'--out',
		fixtureValue.manifest,
		...extra
	]);
}

function manifestOf(fixtureValue: Fixture): Record<string, unknown> {
	return JSON.parse(readFileSync(fixtureValue.manifest, 'utf8')) as Record<string, unknown>;
}

// ── P4 · the manifest answers for BOTH Apple arches ───────────────────────

/**
 * P4 — the manifest must answer for both Apple arches.
 *
 * The bundle is a UNIVERSAL build: two compilations lipo'd together, each
 * slice carrying its own compile-time `target_arch`. The plugin looks the
 * manifest up by `{os}-{arch}` (the compile-time arch of the slice that is
 * running), so a manifest carrying one arch key serves exactly half the
 * machines — and fails SILENTLY on the other half ("no update found").
 *
 * The refusal arm is the writer's own: it must refuse to write a manifest
 * that does not answer for every Apple target, rather than publish one that
 * half the machines can never use.
 */
test('the_manifest_writer_refuses_a_manifest_missing_either_apple_arch', () => {
	const fixtureValue = fixture();
	const result = write(fixtureValue);
	expect(result.status, result.stderr).toBe(0);

	const manifest = manifestOf(fixtureValue);
	const platforms = manifest.platforms as Record<string, unknown>;
	expect(Object.keys(platforms).sort()).toEqual(['darwin-aarch64', 'darwin-x86_64']);
	for (const [target, entry] of Object.entries(platforms)) {
		expect(typeof (entry as { url?: unknown }).url, target).toBe('string');
		expect(typeof (entry as { signature?: unknown }).signature, target).toBe('string');
	}

	// The refusal arm: a manifest that answers for one arch only is refused,
	// and the refusal NAMES the arch that is missing.
	const oneArch = write(fixtureValue, ['--targets', 'darwin-aarch64', '--out', `${fixtureValue.manifest}.one`]);
	expect(oneArch.status).toBe(2);
	expect(oneArch.stderr).toContain('darwin-x86_64');
	expect(oneArch.stderr).toContain('darwin-aarch64');
});

// ── P5 · the sealed kernel version, and a record from another commit ──────

/**
 * P5 — the writer names the SEALED kernel version, and refuses a staged
 * record that names another commit.
 *
 * The record is the same one III-3's refuse law reads: it names the commit
 * the kernel was built from. A manifest published from a record of a
 * different commit would announce a kernel the release never built.
 */
test('the_manifest_writer_names_the_sealed_kernel_version_and_refuses_a_stale_record', () => {
	const fixtureValue = fixture();
	const result = write(fixtureValue);
	expect(result.status, result.stderr).toBe(0);

	const manifest = manifestOf(fixtureValue);
	expect(manifest.kernel_version).toBe('1.29.23');
	expect(manifest.kernel_versions).toEqual(['1.29.23']);

	// A record from another commit is refused, and BOTH commits are named.
	const stale = fixture({ kernelCommit: '0000000000000000000000000000000000000000' });
	const staleRun = write(stale);
	expect(staleRun.status).toBe(2);
	expect(staleRun.stderr).toContain('0000000000000000000000000000000000000000');
	expect(staleRun.stderr).toContain(
		execFileSync('git', ['rev-parse', 'HEAD'], {
			cwd: stale.root,
			encoding: 'utf8'
		}).trim()
	);
});

// ── P13 · the announced version must be the artifact's own ────────────────

/**
 * P13 — the announced version must equal the artifact's OWN
 * `CFBundleShortVersionString`, and the writer READS the plist.
 *
 * This is the anti-relabel law. A re-signed OLD artifact carries a valid
 * signature and a new version number; the app would install it, report the
 * OLD version, and offer itself the same update FOREVER. So the writer
 * refuses a manifest that announces a version the artifact does not carry —
 * and it reads the artifact's own plist rather than taking the version on
 * trust.
 */
test('the_manifest_writer_refuses_a_version_the_artifact_does_not_carry', () => {
	// The signature is made for the ANNOUNCED version, so the only refusal
	// the writer can reach here is the one this pin is about. (With the
	// signature binding the artifact's own version, the writer would refuse
	// for a different law and this pin would pass on the wrong refusal.)
	const fixtureValue = fixture({ artifactVersion: '0.1.0', signedVersion: '0.1.2' });

	// Announcing 0.1.2 for an artifact that says 0.1.0 is refused, and the
	// refusal names both versions.
	const relabel = write(fixtureValue, ['--version', '0.1.2']);
	expect(relabel.status).toBe(2);
	expect(relabel.stderr).toContain('0.1.2');
	expect(relabel.stderr).toContain('0.1.0');
	// ... and it is THIS law that refused it, not another one.
	expect(relabel.stderr).toContain("the artifact's own version is 0.1.0, not the announced 0.1.2");

	// The version the artifact carries is accepted — with a signature made
	// for it, which is what a real release looks like.
	const honestFixture = fixture({ artifactVersion: '0.1.0', signedVersion: '0.1.0' });
	const honest = write(honestFixture, ['--version', '0.1.0']);
	expect(honest.status, honest.stderr).toBe(0);
	expect(manifestOf(honestFixture).version).toBe('0.1.0');
});

// ── P6 · the private key is in no tracked file ────────────────────────────

/**
 * P6 — the private updater key is in NO tracked file.
 *
 * The marker is the base64 of the minisign secret key's own header, so it is
 * what a private key file starts with. The scan is over `git ls-files` — the
 * TRACKED tree, not the working tree — and it is built at runtime from the
 * plaintext marker, so this file does not itself carry the needle.
 *
 * The second arm is the environment variable: a `TAURI_SIGNING_PRIVATE_KEY`
 * with a VALUE is a key in the tree, whether or not it is base64.
 */
test('the_private_updater_key_is_in_no_tracked_file', () => {
	const needle = Buffer.from('untrusted comment: rsign encrypted secret key').toString('base64');
	const tracked = execFileSync('git', ['ls-files'], {
		cwd: kernelRoot,
		encoding: 'utf8',
		maxBuffer: 64 * 1024 * 1024
	})
		.split('\n')
		.filter((line) => line.length > 0);

	expect(tracked.length).toBeGreaterThan(100);

	const offenders: string[] = [];
	for (const path of tracked) {
		const bytes = readFileSync(join(kernelRoot, path));
		const text = bytes.toString('latin1');
		if (text.includes(needle)) {
			offenders.push(`${path}: carries the private key marker`);
		}
		const assignment = /TAURI_SIGNING_PRIVATE_KEY\s*=\s*["']?[A-Za-z0-9+/=]{24,}/.exec(text);
		if (assignment !== null) {
			offenders.push(`${path}: ${assignment[0].slice(0, 60)}`);
		}
	}
	expect(offenders, `the private updater key is in the tracked tree:\n${offenders.join('\n')}`).toEqual(
		[]
	);
});

// ── P7 · the pinned public key is in the tree, and parses ─────────────────

/**
 * P7 — the pinned public key is in the tree and PARSES.
 *
 * The shape is measured off a real generated key: the base64 decodes to two
 * lines — an untrusted comment naming a 16-hex keynum, and a 42-byte key
 * packet that begins `Ed` (the minisign public-key signature).
 *
 * And it is NOT A PATH. The CLI reads a `pubkey` that happens to be an
 * existing file as that file's contents, while the plugin base64-decodes the
 * same string at runtime — so a path in the config would verify against a
 * different key on the machine that built it than on the machine that runs
 * it.
 */
test('the_pinned_public_key_is_in_the_tree_and_parses', () => {
	const config = JSON.parse(readFileSync(configPath, 'utf8')) as {
		plugins?: { updater?: { pubkey?: string } };
	};
	const pubkey = config.plugins?.updater?.pubkey;
	expect(typeof pubkey, 'the configuration carries no updater public key').toBe('string');
	const trimmed = (pubkey as string).trim();

	expect(trimmed).not.toBe('');
	expect(trimmed).not.toContain('/');
	expect(trimmed.endsWith('.pub')).toBe(false);
	expect(existsSync(join(shellRoot, trimmed))).toBe(false);

	const decoded = Buffer.from(trimmed, 'base64').toString('utf8');
	const lines = decoded.split('\n');
	expect(lines[0]).toMatch(/^untrusted comment: minisign public key: [0-9A-F]{16}$/);
	const packet = Buffer.from(lines[1] ?? '', 'base64');
	expect(packet.length).toBe(42);
	expect(packet.subarray(0, 2).toString('ascii')).toBe('Ed');
});

// ── P8 · a signed version, and no downgrades ──────────────────────────────

/**
 * P8 — `requireSignedVersion: true` and `allowDowngrades: false`.
 *
 * The endpoint's response is fetched over TLS but is NOT itself signed, so
 * anyone who can serve it could pair an inflated version with an older
 * release's artifact and signature. The signed version is what stops that —
 * and it is free here, because no release has ever been published.
 */
test('the_updater_requires_a_signed_version_and_forbids_downgrades', () => {
	const config = JSON.parse(readFileSync(configPath, 'utf8')) as {
		plugins?: { updater?: Record<string, unknown> };
	};
	const updater = config.plugins?.updater;
	expect(updater, 'the configuration carries no updater block').toBeTruthy();
	expect(updater?.requireSignedVersion).toBe(true);
	expect(updater?.allowDowngrades).toBe(false);
});

// ── P9 · HTTPS only, and no dangerous flag ────────────────────────────────

/**
 * P9 — every endpoint is `https://`, and no `dangerous*` flag is set.
 *
 * The plugin refuses a non-HTTPS endpoint in a release build
 * (`Error::InsecureTransportProtocol`), and the three `dangerous*` flags
 * exist to turn that off: they accept invalid certificates, invalid
 * hostnames, or plain HTTP. An updater that verifies nothing is not an
 * updater.
 */
test('the_updater_endpoints_are_https_and_carry_no_dangerous_flag', () => {
	const config = JSON.parse(readFileSync(configPath, 'utf8')) as {
		plugins?: { updater?: Record<string, unknown> };
	};
	const updater = config.plugins?.updater;
	expect(updater, 'the configuration carries no updater block').toBeTruthy();

	const endpoints = updater?.endpoints;
	expect(Array.isArray(endpoints), 'the updater declares no endpoints').toBe(true);
	expect((endpoints as unknown[]).length).toBeGreaterThan(0);
	for (const endpoint of endpoints as string[]) {
		expect(endpoint.startsWith('https://'), `${endpoint} is not https`).toBe(true);
	}

	for (const flag of [
		'dangerousInsecureTransportProtocol',
		'dangerousAcceptInvalidCerts',
		'dangerousAcceptInvalidHostnames'
	]) {
		expect(Object.hasOwn(updater as object, flag), `${flag} must not be set`).toBe(false);
	}
});

// ── P11 · the webview holds no updater authority ──────────────────────────

/**
 * P11 — the capability grants the webview NO updater permission.
 *
 * The Rust core owns the check and the install; the webview receives a DTO.
 * Granting `updater:default` would hand the renderer the install authority,
 * and the renderer is the part of this app that renders model output.
 */
test('the_capability_grants_the_webview_no_updater_permission', () => {
	const capability = JSON.parse(readFileSync(capabilityPath, 'utf8')) as {
		permissions?: string[];
	};
	const permissions = capability.permissions ?? [];
	for (const permission of permissions) {
		expect(
			permission.startsWith('updater:'),
			`the webview must not hold the updater permission: ${permission}`
		).toBe(false);
	}
});

// ══ III-7 · THE DISTRIBUTION ARTEFACT ═══════════════════════════════════
//
// The updater shipped in v1.29.26 can fetch NOTHING: no release published an
// app, a signature or a manifest, and the manifest writer built the download
// URL from the APP's version (`0.1.1`) rather than from the RELEASE TAG
// (`v1.29.26`) — a tag that does not exist and never will.
//
// The labels below are `III-7 P1` … and not `P1` … because this tree's
// updater pins already carry the short labels (P1–P13, split between this
// file and the Rust core), and two pins with one label are two pins nobody
// can find again.

/**
 * The KEYNUM out of a minisign public key, in either spelling.
 *
 * The Tauri CLI writes `minisign public key: <KEYNUM>` and minisign itself
 * writes `minisign public key <KEYNUM>`; the pattern accepts both, because
 * a pin that only reads one of them would be a pin that reads a shape
 * instead of a key.
 *
 * The input is the DECODED public key text — the `.pub` FILE carries the
 * base64 of that text, so a caller that forgot to decode would read no
 * keynum at all, and an assertion comparing an empty string against the
 * shipped one would pass for the wrong reason.
 */
function keynumOf(publicKeyText: string): string {
	const first = publicKeyText.split('\n')[0] ?? '';
	const match = /minisign public key:?\s+([0-9A-Fa-f]{16})/.exec(first);
	return match?.[1]?.toUpperCase() ?? '';
}

/** The base64 a `.pub` FILE carries, decoded to the key text it holds. */
function decodedPublicKey(base64Text: string): string {
	return Buffer.from(base64Text.trim(), 'base64').toString('utf8');
}

/** The KEYNUM the app SHIPS — read from the app's own configuration. */
function shippedKeynum(): string {
	const config = JSON.parse(readFileSync(configPath, 'utf8')) as {
		plugins?: { updater?: { pubkey?: string } };
	};
	const decoded = Buffer.from((config.plugins?.updater?.pubkey ?? '').trim(), 'base64').toString(
		'utf8'
	);
	return keynumOf(decoded);
}

/** The app's own public half, in the base64 form the configuration carries. */
function shippedPublicKey(): string {
	const config = JSON.parse(readFileSync(configPath, 'utf8')) as {
		plugins?: { updater?: { pubkey?: string } };
	};
	return (config.plugins?.updater?.pubkey ?? '').trim();
}
/**
 * A FRESHLY MINTED keypair, in the form the signing lane reads.
 *
 * This is the anti-vacuity arm's whole point. Every drill in III-4 minted a
 * fresh pair and threw it away, so the pair the app SHIPS was never proved:
 * a tree whose gates cannot see the pairing would let a second key sign the
 * release and nothing would say so. The pin that follows REFUSES this key,
 * so a pin that a fresh fixture could satisfy is a pin that would have
 * hidden the defect again.
 */
function freshKeypair(): { secretKey: string; publicKey: string; keynum: string } {
	const directory = tempDir('brain-updater-freshkey-');
	const secretKey = join(directory, 'fresh.key');
	const result = spawnSync(
		'pnpm',
		['tauri', 'signer', 'generate', '--ci', '-w', secretKey, '-p', 'brain-shell-fixture-password'],
		{ cwd: shellRoot, encoding: 'utf8' }
	);
	if (result.status !== 0) {
		throw new Error(
			`the Tauri CLI could not mint a fixture keypair: ${result.stderr}\n` +
				'Without a FRESH key the anti-vacuity arm of the pairing pin cannot run, and a ' +
				'pin that cannot see a second key is a pin that cannot see a swapped one.'
		);
	}
	const publicKey = `${secretKey}.pub`;
	const keynum = keynumOf(decodedPublicKey(readFileSync(publicKey, 'utf8')));
	if (keynum === '') {
		throw new Error(
			`the minted fixture key carries no keynum: ${publicKey}\n` +
				'An empty keynum would make the anti-vacuity assertion pass for the wrong reason.'
		);
	}
	return { secretKey, publicKey, keynum };
}

/** The signing lane, driven as a subprocess. */
function runSign(args: string[]): Run {
	const result = spawnSync('bash', [signScriptPath, ...args], { encoding: 'utf8' });
	return {
		status: result.status,
		stdout: result.stdout ?? '',
		stderr: result.stderr ?? ''
	};
}

// ── III-7 P1 · the manifest's URL names a tag that RESOLVES ───────────────

/**
 * III-7 P1 — the manifest carries BOTH versions, and the tag is checkable.
 *
 * The app's version (`0.1.1`) is what the updater COMPARES; the release tag
 * (`v1.29.26`) is what the artefact is FETCHED from. The writer used to
 * build the URL from the app's version, which produced
 * `releases/download/v0.1.1/…` — a tag that does not exist and never will,
 * in a manifest that was otherwise perfect.
 *
 * So the tag is a writer INPUT of its own, and a tag `git rev-parse` cannot
 * resolve is refused — naming the tag AND the URL it refused, because a
 * refusal that names one of the two is a refusal the reader has to guess
 * about.
 */
test('the_manifest_writer_names_a_resolvable_release_tag_and_refuses_one_that_does_not_exist', () => {
	const fixtureValue = fixture();
	const result = write(fixtureValue);
	expect(result.status, result.stderr).toBe(0);

	const manifest = manifestOf(fixtureValue);

	// The two facts, kept apart: the app's version stays the app's version.
	expect(manifest.version).toBe('0.1.1');
	expect(manifest.release_tag).toBe(RELEASED_TAG);

	const platforms = manifest.platforms as Record<string, { url: string }>;
	for (const [target, entry] of Object.entries(platforms)) {
		expect(entry.url, target).toContain(RELEASED_TAG);
		// The defect this round exists for: the URL used to carry the APP's
		// version, so every release pointed at a tag that could not exist.
		expect(entry.url, target).not.toContain('/v0.1.1/');
	}

	// The refusal arm: a tag this repository does not have is refused, and
	// the refusal names BOTH the tag and the URL it refused. The URL
	// assertion comes FIRST on purpose: a drill that turns the writer's
	// refusal off should fail with the URL in the message, not with a bare
	// status — "expected 0 to be 2" says nothing about what was wrong.
	const nope = write(fixtureValue, ['--release-tag', 'v0.0.0-nope']);
	expect(nope.stderr).toContain('v0.0.0-nope');
	expect(nope.stderr).toContain('does not resolve');
	expect(nope.stderr).toContain('releases/download/v0.0.0-nope');
	expect(nope.status).toBe(2);

	// And a manifest with NO tag at all is refused too: the URL would name
	// nothing, which is not a download.
	const untagged = write(fixtureValue, ['--release-tag', '']);
	expect(untagged.status).toBe(2);
	expect(untagged.stderr).toContain('release tag');
});

// ── III-7 P2 · the key that signs IS the key the app ships ───────────────

/**
 * III-7 P2 — the pinned key and the signing key are the SAME keypair.
 *
 * The law is about the PUBLIC HALVES matching, not about shapes: a public
 * key that PARSES is not a key anybody can sign with. The tree's own gates
 * could not see this — P7 checked the pinned key's shape and P6 checked
 * that no private key is in the tree — and every III-4 drill minted a fresh
 * pair, so the pair the operator HOLDS was never compared with the pair the
 * app SHIPS.
 *
 * **The anti-vacuity arm is the pin.** A freshly minted keypair is driven
 * through the signing lane, and the lane REFUSES it — naming both halves.
 * A pin that a fresh fixture could satisfy is the pin that hid the defect,
 * so this one is written so that it cannot be.
 *
 * The positive arm matters as much: the SHIPPED half must sail past the
 * pairing check, or the check is just a refusal that refuses everything.
 */
test('the_signing_lane_refuses_a_key_that_is_not_the_key_the_app_ships', () => {
	const shipped = shippedKeynum();
	expect(shipped, 'the app configuration pins no keynum').not.toBe('');

	const fresh = freshKeypair();
	// The anti-vacuity precondition, asserted: the fixture really is a
	// DIFFERENT key, so the refusal below is about the key and not about
	// some unrelated rule.
	expect(fresh.keynum).not.toBe('');
	expect(fresh.keynum).not.toBe(shipped);

	const refused = runSign([
		'--tag',
		RELEASED_TAG,
		'--public-key',
		fresh.publicKey,
		'--secret-key',
		fresh.secretKey,
		'--dry-run'
	]);
	expect(refused.status).not.toBe(0);
	// Both halves are NAMED — the pinned one and the one that was offered —
	// in ONE assertion, so the drill that turns this check off fails with a
	// message that names both rather than with the first half alone.
	expect(
		refused.stderr.includes(shipped) && refused.stderr.includes(fresh.keynum),
		`the refusal must name BOTH halves — the app pins ${shipped}, the offered key is ${fresh.keynum} — got: ${refused.stderr}`
	).toBe(true);
	expect(refused.stderr).toContain('the signing key is NOT the key the app ships');

	// The positive arm: the SHIPPED half passes the pairing check, and the
	// lane says so by name before it does anything else.
	const pinnedFile = join(tempDir('brain-updater-pinned-'), 'pinned.pub');
	writeFileSync(pinnedFile, `${shippedPublicKey()}\n`);
	const accepted = runSign([
		'--tag',
		RELEASED_TAG,
		'--public-key',
		pinnedFile,
		'--secret-key',
		fresh.secretKey,
		'--dry-run'
	]);
	expect(accepted.stdout).toContain('the signing key IS the key the app ships');
	expect(accepted.stdout).toContain(shipped);
	expect(accepted.stderr).not.toContain('the signing key is NOT the key the app ships');
});

// ── III-7 P3 · the chain, or none of it ──────────────────────────────────

/**
 * III-7 P3 — the release publishes the artefact, the signature and the
 * manifest, or it publishes NONE of them.
 *
 * Two halves, because the law has two halves:
 *
 *  - **STRUCTURAL.** The PUBLIC workflow holds no key and runs no signing
 *    step. A key in a public CI job is a key anyone with a push-trigger can
 *    run, so the public lane proves the build and the private lane signs
 *    what it proved — and the private lane is NAMED, so a reader is never
 *    left guessing where the signing happens.
 *
 *  - **BEHAVIOURAL.** The signing lane refuses to publish a partial chain:
 *    remove the signature from the staging set and the publish step refuses.
 */
test('the_release_publishes_the_app_the_signature_and_the_manifest_or_none_of_them', () => {
	// The structural half: the public workflow never signs.
	const workflow = readFileSync(releaseWorkflowPath, 'utf8');
	for (const forbidden of ['minisign', 'tauri signer', 'TAURI_SIGNING_PRIVATE_KEY']) {
		expect(
			workflow.includes(forbidden),
			`the public release workflow must never sign — it names ${forbidden}`
		).toBe(false);
	}
	// ... and the private lane is NAMED, so "where does the signing happen"
	// is answerable from the file that publishes the release.
	expect(workflow).toContain('release-app-sign.sh');

	// The behavioural half: the chain gate refuses a partial chain.
	const chain = tempDir('brain-updater-chain-');
	const artifact = join(chain, 'Brain Shell.app.tar.gz');
	writeFileSync(artifact, 'the updater artifact bytes\n');
	writeFileSync(`${artifact}.sig`, 'the signature\n');
	writeFileSync(join(chain, 'latest.json'), '{}\n');

	const complete = runSign(['--check-chain', artifact]);
	expect(complete.status, complete.stderr).toBe(0);

	// Remove the signature: the gate refuses, and it names what is missing.
	rmSync(`${artifact}.sig`);
	const partial = runSign(['--check-chain', artifact]);
	expect(partial.status).not.toBe(0);
	expect(partial.stderr).toContain('required asset missing');
	expect(partial.stderr).toContain('signature');
});

// ── III-7 P4 · the chain's three pieces are REQUIRED assets ───────────────

/**
 * III-7 P4 — the app tarball, the signature and the manifest are REQUIRED
 * assets, and the refusal NAMES the missing one.
 *
 * The same shape as the release's own per-binary loop: a missing asset is
 * refused by name, never published around. Each of the three is removed in
 * turn, because a gate that only knows about one of them is a gate with two
 * holes.
 */
test('the_app_tarball_the_signature_and_the_manifest_are_required_assets', () => {
	const chain = tempDir('brain-updater-required-');
	const artifact = join(chain, 'Brain Shell.app.tar.gz');
	const signature = `${artifact}.sig`;
	const manifest = join(chain, 'latest.json');

	const seed = () => {
		writeFileSync(artifact, 'the updater artifact bytes\n');
		writeFileSync(signature, 'the signature\n');
		writeFileSync(manifest, '{}\n');
	};

	seed();
	expect(runSign(['--check-chain', artifact]).status).toBe(0);

	// The tarball is gone.
	seed();
	rmSync(artifact);
	const noArtifact = runSign(['--check-chain', artifact]);
	expect(noArtifact.status).not.toBe(0);
	expect(noArtifact.stderr).toContain('required asset missing');
	expect(noArtifact.stderr).toContain('artifact');

	// The signature is gone.
	seed();
	rmSync(signature);
	const noSignature = runSign(['--check-chain', artifact]);
	expect(noSignature.status).not.toBe(0);
	expect(noSignature.stderr).toContain('required asset missing');
	expect(noSignature.stderr).toContain('signature');

	// The manifest is gone.
	seed();
	rmSync(manifest);
	const noManifest = runSign(['--check-chain', artifact]);
	expect(noManifest.status).not.toBe(0);
	expect(noManifest.stderr).toContain('required asset missing');
	expect(noManifest.stderr).toContain('manifest');
});

// ── III-7 P5 · the private key is nowhere the public can reach it ────────

/**
 * III-7 P5 — the private key is in no tracked file, AND the public lane
 * never signs.
 *
 * The tree's existing pin (`the_private_updater_key_is_in_no_tracked_file`)
 * proves the first half. This one proves BOTH halves together, because the
 * law is one law: **the key that signs lives where the public cannot reach
 * it.** A tree scan alone would pass while a public workflow handed the key
 * to a job, and a workflow scan alone would pass while the key sat in the
 * repository.
 *
 * The workflow half is a TEXT scan, and this tree's own lesson (R127's S2/S3
 * arms) is that a comment naming a tool turns such a scan red. So the
 * forbidden names are the ones the workflow has no reason to mention at all,
 * and the workflow's comments were written to keep it that way.
 */
test('the_private_updater_key_is_in_no_tracked_file_and_the_public_lane_never_signs', () => {
	// The tree half: the same scan the standing pin runs.
	const needle = Buffer.from('untrusted comment: rsign encrypted secret key').toString('base64');
	const tracked = execFileSync('git', ['ls-files'], {
		cwd: kernelRoot,
		encoding: 'utf8',
		maxBuffer: 64 * 1024 * 1024
	})
		.split('\n')
		.filter((line) => line.length > 0);
	expect(tracked.length).toBeGreaterThan(100);
	for (const path of tracked) {
		const text = readFileSync(join(kernelRoot, path)).toString('latin1');
		expect(
			text.includes(needle),
			`the private updater key is in the tracked tree: ${path}`
		).toBe(false);
	}

	// The workflow half: the public release workflow holds no key and runs
	// no signing step.
	const workflow = readFileSync(releaseWorkflowPath, 'utf8');
	for (const forbidden of ['minisign', 'tauri signer', 'TAURI_SIGNING_PRIVATE_KEY']) {
		expect(
			workflow.includes(forbidden),
			`the public release workflow must never sign — it names ${forbidden}`
		).toBe(false);
	}

	// And the private lane is NAMED — a reader must be able to find where
	// the signing happens without reading the whole tree.
	expect(existsSync(signScriptPath), 'the signing lane is missing').toBe(true);
	const lane = readFileSync(signScriptPath, 'utf8');
	expect(lane).toContain('the public Actions must never hold the signing key');
});
