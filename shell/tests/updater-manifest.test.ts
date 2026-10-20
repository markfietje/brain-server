import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, test } from 'vitest';
import { kernelRoot, shellRoot } from './fixtures';

/**
 * III-4 — the updater's pins: the trust anchor, the transport, the webview's
 * authority, and the manifest writer.
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
const configPath = join(shellRoot, 'src-tauri', 'tauri.conf.json');
const macosConfigPath = join(shellRoot, 'src-tauri', 'tauri.macos.conf.json');
const capabilityPath = join(shellRoot, 'src-tauri', 'capabilities', 'main.json');

const scratch: string[] = [];

afterEach(() => {
	for (const directory of scratch.splice(0)) {
		rmSync(directory, { recursive: true, force: true });
	}
});

function tempDir(prefix = 'brain-updater-'): string {
	const directory = mkdtempSync(join(tmpdir(), prefix));
	scratch.push(directory);
	return directory;
}

/** A `.sig` file with the given trusted comment, in the shape minisign writes. */
function signatureFile(trustedComment: string): string {
	const packet = Buffer.alloc(74);
	packet.write('Ed', 0, 'ascii');
	const global = Buffer.alloc(64);
	return [
		'untrusted comment: signature from minisign secret key',
		packet.toString('base64'),
		`trusted comment: ${trustedComment}`,
		global.toString('base64'),
		''
	].join('\n');
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
};

type FixtureOptions = {
	kernelCommit?: string;
	kernelVersion?: string;
	artifactVersion?: string;
	signedVersion?: string;
	shellVersion?: string;
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
	// beside it — exactly where the bundler writes it.
	const work = tempDir('brain-updater-build-');
	const bundle = join(work, 'Brain Shell.app');
	mkdirSync(join(bundle, 'Contents'), { recursive: true });
	writeFileSync(join(bundle, 'Contents', 'Info.plist'), infoPlist(artifactVersion));
	const artifact = join(work, 'Brain Shell.app.tar.gz');
	writeFileSync(artifact, 'the updater artifact bytes\n');
	const signature = `${artifact}.sig`;
	writeFileSync(
		signature,
		signatureFile(
			`timestamp:1792477513\tfile:Brain Shell.app.tar.gz\tversion:${options.signedVersion ?? artifactVersion}`
		)
	);

	return { root, staged, artifact, bundle, signature, manifest: join(work, 'latest.json') };
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
	const fixtureValue = fixture({ artifactVersion: '0.1.0' });

	// Announcing 0.1.2 for an artifact that says 0.1.0 is refused, and the
	// refusal names both versions.
	const relabel = write(fixtureValue, ['--version', '0.1.2']);
	expect(relabel.status).toBe(2);
	expect(relabel.stderr).toContain('0.1.2');
	expect(relabel.stderr).toContain('0.1.0');

	// The version the artifact carries is accepted.
	const honest = write(fixtureValue, ['--version', '0.1.0']);
	expect(honest.status, honest.stderr).toBe(0);
	expect(manifestOf(fixtureValue).version).toBe('0.1.0');
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
