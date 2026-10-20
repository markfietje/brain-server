#!/usr/bin/env node
// @ts-nocheck — this is a Node-builtins CLI, executed by Node and DRIVEN by
// shell/tests/updater-manifest.test.ts. It is not a type-checked library and
// `checkJs` would demand annotations on every internal helper without
// proving anything the behavioral pins do not do.
/**
 * III-4 — the manifest writer: the `latest.json` a release publishes.
 *
 * One module, builtins only, one subcommand:
 *
 *   write   Build the update manifest for a release, and REFUSE rather than
 *           guess. Every refusal names what it found and what it expected.
 *
 * The manifest is untrusted input to every app that fetches it, so the
 * writer's job is to make it say exactly one true thing. The refusals, and
 * the law behind each:
 *
 *  - **A dirty tree.** A manifest names a release; a release names a commit;
 *   a commit cannot be a dirty working tree. This is the same law
 *   `sidecar.mjs stage` enforces, and it exists because a build recorded
 *   against a commit it was not built from is a false provenance claim.
 *
 *  - **A staged record from another commit.** The sealed sidecar record
 *   names the commit the bundled kernel was built from. A manifest built
 *   beside a record of a different commit would announce a kernel the
 *   release never built.
 *
 *  - **A manifest that does not answer for EVERY Apple arch.** The bundle is
 *   a UNIVERSAL build: two compilations lipo'd together, each slice carrying
 *   its own compile-time `target_arch`. The updater plugin looks the
 *   manifest up by `{os}-{arch}` — the arch of the slice that is RUNNING — so
 *   a manifest carrying one arch key serves exactly half the machines, and
 *   fails SILENTLY on the other half ("no update found"). Both keys, always.
 *
 *  - **A release that vouches for no kernel version.** The update replaces
 *   the whole `.app`, INCLUDING the bundled kernel sidecar, so an update
 *   changes the kernel too. A manifest that does not say which kernel it
 *   carries does not get to replace the one the running app carries.
 *
 *  - **A missing or mismatched signature.** The signature beside the
 *   artifact is the update's only proof of WHO produced it. A missing one
 *   means the artifact cannot be verified at all; one that names another
 *   file, or another version, does not bind this release.
 *
 *  - **An announced version the artifact does not carry.** The writer READS
 *   the artifact's own `CFBundleShortVersionString` — it does not take the
 *   version on trust. This is the anti-relabel law: a re-signed OLD artifact
 *   carries a valid signature and a new version number, the app installs it,
 *   reports the OLD version, and offers itself the same update FOREVER. So
 *   the rollback is a REBUILD under a new version, never a re-sign.
 *
 * What the writer does NOT do: it does not verify the signature's
 * CRYPTOGRAPHY. Minisign verification belongs to the app, at install time,
 * against the key the app pins — that is the one place the bytes and the key
 * meet. The writer checks the signature's BINDING: which artifact it names
 * and which version it was made for.
 */
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

/** The Apple targets every manifest must answer for. */
export const APPLE_TARGETS = ['darwin-aarch64', 'darwin-x86_64'];

/** Where the sealed sidecar record lives, relative to the repository root. */
const STAGED_REL = join('shell', 'src-tauri', 'binaries');
const PROVENANCE_NAME = 'SIDECAR_PROVENANCE.json';

/** The base the release publishes its artifacts under. `{version}` is filled in. */
const DEFAULT_URL_TEMPLATE =
	'https://github.com/markfietje/brain-server/releases/download/v{version}';

function run(command, args, options = {}) {
	const result = spawnSync(command, args, {
		encoding: 'utf8',
		maxBuffer: 64 * 1024 * 1024,
		...options
	});
	return {
		status: result.status,
		signal: result.signal,
		stdout: result.stdout ?? '',
		stderr: result.stderr ?? '',
		error: result.error
	};
}

function output(result) {
	return `${result.stdout}${result.stderr}`.trim();
}

function git(root, args) {
	const result = run('git', ['-C', root, ...args]);
	if (result.status !== 0) {
		throw new Error(`git ${args.join(' ')} failed: ${output(result) || 'no output'}`);
	}
	return result.stdout.trim();
}

/** The tree's HEAD and whether it is dirty. */
export function treeState(root = REPO_ROOT) {
	return {
		commit: git(root, ['rev-parse', 'HEAD']),
		dirty: git(root, ['status', '--porcelain']).length > 0
	};
}

/** The shell's own version — the version the updater compares. */
export function shellVersion(root = REPO_ROOT) {
	const config = JSON.parse(
		readFileSync(join(root, 'shell', 'src-tauri', 'tauri.conf.json'), 'utf8')
	);
	const version = config?.version;
	if (typeof version !== 'string' || version.length === 0) {
		throw new Error('the shell configuration carries no version');
	}
	return version;
}

/** The sealed sidecar record, or null when it is absent or unreadable. */
export function readProvenance(staged) {
	const path = join(staged, PROVENANCE_NAME);
	if (!existsSync(path)) return null;
	try {
		return JSON.parse(readFileSync(path, 'utf8'));
	} catch {
		return null;
	}
}

/**
 * The version the BUILT bundle carries, read from its own `Info.plist`.
 *
 * A plain read, not `plutil`: the writer runs on any machine that can build
 * a release, and the plist's `CFBundleShortVersionString` is a plain string
 * inside a plain dictionary.
 */
export function bundleVersion(bundle) {
	const path = join(bundle, 'Contents', 'Info.plist');
	if (!existsSync(path)) return null;
	const plist = readFileSync(path, 'utf8');
	const match = /<key>\s*CFBundleShortVersionString\s*<\/key>\s*<string>([^<]*)<\/string>/.exec(
		plist
	);
	return match?.[1]?.trim() ?? null;
}

/**
 * The signature's trusted comment: the fields the Tauri CLI writes as
 * tab-separated `key:value` pairs (`timestamp:…\tfile:…\tversion:…`).
 *
 * The trusted comment is covered by the signature, so it is not
 * attacker-controlled — but the writer does not verify the signature, so it
 * only READS the binding here.
 */
export function signatureBinding(signaturePath) {
	if (!existsSync(signaturePath)) return null;
	// The `.sig` file's CONTENT is the base64 of the minisign signature text —
	// the same string the manifest's `signature` field carries. The trusted
	// comment is on the THIRD line of the decoded text.
	const encoded = readFileSync(signaturePath, 'utf8').trim();
	let text;
	try {
		text = Buffer.from(encoded, 'base64').toString('utf8');
	} catch {
		return null;
	}
	if (!text.includes('trusted comment: ')) return null;
	const lines = text.split('\n');
	const comment = lines.find((line) => line.startsWith('trusted comment: '));
	if (comment === undefined) return null;
	const fields = {};
	for (const field of comment.slice('trusted comment: '.length).split('\t')) {
		const separator = field.indexOf(':');
		if (separator > 0) fields[field.slice(0, separator)] = field.slice(separator + 1);
	}
	return { file: fields.file ?? null, version: fields.version ?? null };
}

/** The targets a manifest does NOT answer for. An empty array is the only pass. */
export function targetFailures(targets) {
	return APPLE_TARGETS.filter((target) => !targets.includes(target)).map(
		(target) => `the manifest must answer for ${target} as well as ${APPLE_TARGETS.join(' and ')}`
	);
}

/**
 * The manifest the writer would publish.
 *
 * `platforms` carries EVERY Apple target, each pointing at the same
 * universal artifact: one build serves both arches, so one artifact and one
 * signature answer for both.
 */
export function manifestFor({
	version,
	notes = '',
	pubDate,
	kernelVersion,
	kernelVersions,
	signature,
	url
}) {
	const platforms = {};
	for (const target of APPLE_TARGETS) {
		platforms[target] = { url, signature };
	}
	return {
		version,
		notes,
		pub_date: pubDate,
		kernel_version: kernelVersion,
		kernel_versions: kernelVersions,
		platforms
	};
}

/**
 * Every refusal the writer has, as one sentence each. An empty array is the
 * only pass.
 */
export function manifestFailures({
	root = REPO_ROOT,
	staged = join(root, STAGED_REL),
	artifact,
	bundle,
	version,
	kernelVersions = [],
	targets = APPLE_TARGETS
}) {
	const failures = [];

	// The tree the release names must be the tree that was built.
	let state = null;
	try {
		state = treeState(root);
	} catch (error) {
		failures.push(`the tree cannot be read: ${error instanceof Error ? error.message : error}`);
	}
	if (state !== null && state.dirty) {
		failures.push('the working tree is dirty: a manifest names a commit, and a commit is not a dirty tree');
	}

	// The sealed record names the commit the bundled kernel was built from.
	const record = readProvenance(staged);
	if (record === null) {
		failures.push(`the staged record is missing or unreadable: ${join(staged, PROVENANCE_NAME)}`);
	} else {
		if (state !== null && record.kernel_commit !== state.commit) {
			failures.push(
				`the staged record names another commit: ${String(record.kernel_commit)} != ${state.commit}`
			);
		}
		if (typeof record.kernel_version !== 'string' || record.kernel_version.length === 0) {
			failures.push('the staged record carries no kernel version');
		} else if (!kernelVersions.includes(record.kernel_version)) {
			failures.push(
				`the release vouches for ${kernelVersions.join(', ') || 'no kernel version'}, but the artifact carries ${record.kernel_version}`
			);
		}
	}

	// The manifest must answer for every Apple arch.
	failures.push(...targetFailures(targets));

	// The kernel versions this release vouches for.
	if (kernelVersions.length === 0) {
		failures.push('the release vouches for no kernel version');
	}

	// The artifact and the bundle.
	if (!existsSync(artifact)) {
		failures.push(`the artifact is absent: ${artifact}`);
	}
	if (!existsSync(join(bundle, 'Contents', 'Info.plist'))) {
		failures.push(`the bundle carries no Contents/Info.plist: ${bundle}`);
	}

	// The signature beside the artifact: present, and bound to THIS artifact
	// and THIS version.
	const signaturePath = `${artifact}.sig`;
	const binding = signatureBinding(signaturePath);
	if (binding === null) {
		failures.push(`the artifact carries no signature beside it: ${signaturePath}`);
	} else {
		const artifactName = basename(artifact);
		if (binding.file !== null && binding.file !== artifactName) {
			failures.push(`the signature names another file: ${binding.file} != ${artifactName}`);
		}
		if (binding.version !== null && binding.version !== version) {
			failures.push(
				`the signature was made for version ${binding.version}, not the announced ${version}`
			);
		}
	}

	// The announced version must be the artifact's OWN version.
	const carried = bundleVersion(bundle);
	if (carried !== null && carried !== version) {
		failures.push(`the artifact's own version is ${carried}, not the announced ${version}`);
	}

	return failures;
}

/**
 * The whole decision: the manifest the writer would publish, and every
 * refusal it has. The manifest is returned even when there are refusals, so
 * a caller can see what WOULD have been published.
 */
export function planManifest(options) {
	const root = options.root ?? REPO_ROOT;
	const staged = options.staged ?? join(root, STAGED_REL);
	const record = readProvenance(staged);
	const version = options.version ?? shellVersion(root);
	const kernelVersions =
		options.kernelVersions ?? (record?.kernel_version ? [record.kernel_version] : []);
	const targets = options.targets ?? APPLE_TARGETS;
	const urlTemplate = options.urlTemplate ?? DEFAULT_URL_TEMPLATE;
	const url = `${urlTemplate.replace('{version}', version)}/${encodeURIComponent(basename(options.artifact))}`;

	const failures = manifestFailures({ ...options, root, staged, version, kernelVersions, targets, urlTemplate });
	const signature = existsSync(`${options.artifact}.sig`)
		? readFileSync(`${options.artifact}.sig`, 'utf8').trim()
		: '';

	const manifest = manifestFor({
		version,
		notes: options.notes ?? '',
		pubDate: options.pubDate ?? new Date().toISOString(),
		kernelVersion: record?.kernel_version ?? '',
		kernelVersions,
		signature,
		url
	});
	return { manifest, failures };
}

function parseArgs(argv) {
	const options = {};
	for (let index = 0; index < argv.length; index += 1) {
		const flag = argv[index];
		if (!flag.startsWith('--')) {
			throw new Error(`unexpected argument: ${flag}`);
		}
		const value = argv[index + 1];
		if (value === undefined) {
			throw new Error(`--${flag.slice(2)} needs a value`);
		}
		options[flag.slice(2)] = value;
		index += 1;
	}
	return options;
}

function write(options) {
	const root = options.root ?? REPO_ROOT;
	const staged = options.staged ?? join(root, STAGED_REL);
	if (options.artifact === undefined || options.bundle === undefined) {
		throw new Error('write needs --artifact and --bundle');
	}
	const targets = options.targets === undefined ? APPLE_TARGETS : options.targets.split(',');

	const { manifest, failures } = planManifest({
		root,
		staged,
		artifact: options.artifact,
		bundle: options.bundle,
		version: options.version,
		kernelVersions:
			options['kernel-versions'] === undefined
				? undefined
				: options['kernel-versions'].split(',').filter((value) => value.length > 0),
		targets,
		urlTemplate: options['url-template'] ?? DEFAULT_URL_TEMPLATE,
		notes: options.notes ?? '',
		pubDate: options['pub-date'] ?? new Date().toISOString()
	});

	if (failures.length > 0) {
		console.error('refusing to write the update manifest:');
		for (const failure of failures) console.error(`  ${failure}`);
		process.exit(2);
	}

	const out = options.out ?? join(root, 'latest.json');
	writeFileSync(out, `${JSON.stringify(manifest, null, 2)}\n`);
	console.log(`[updater] wrote ${out}`);
	console.log(JSON.stringify(manifest, null, 2));
}

function usage() {
	console.error('usage:');
	console.error('  node shell/scripts/updater.mjs write --artifact <file.tar.gz> --bundle <App.app>');
	console.error('       [--root <repo>] [--staged <dir>] [--version <v>] [--targets <csv>]');
	console.error('       [--kernel-versions <csv>] [--url-template <url>] [--out <file>]');
	process.exit(2);
}

function main() {
	const [command, ...rest] = process.argv.slice(2);
	if (command !== 'write') usage();
	let options;
	try {
		options = parseArgs(rest);
	} catch (error) {
		console.error(`refusing to write the update manifest: ${error instanceof Error ? error.message : error}`);
		process.exit(2);
	}
	write(options);
}

if (process.argv[1] !== undefined && import.meta.url === `file://${process.argv[1]}`) {
	main();
}
