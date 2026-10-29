#!/usr/bin/env node
// @ts-nocheck — this is a Node-builtins CLI, executed by Node and DRIVEN by
// shell/tests/updater-manifest.test.ts. It is not a type-checked library and
// `checkJs` would demand annotations on every internal helper without
// proving anything the behavioral pins do not do.
/**
 * III-4 — the manifest writer: the `latest.json` a release publishes.
 * III-7 — the release TAG the manifest's URL names.
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
 *  - **A download URL that names a tag this repository does not have.** The
 *   app's version (`0.1.1`) and the release tag (`v1.29.26`) are two facts on
 *   two different lines, and they are not interchangeable: the manifest's
 *   `version` is what the updater COMPARES against the installed bundle, and
 *   the URL's tag is what the artefact is FETCHED from. A writer that built
 *   the URL from the app's version produced `releases/download/v0.1.1/…` — a
 *   tag that does not exist and never will — and the manifest was a promise
 *   the release could not keep. So the tag is a writer INPUT of its own, and
 *   a tag `git rev-parse` cannot resolve is refused, naming the URL.
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

/**
 * The base the release publishes its artifacts under. `{tag}` is the RELEASE
 * TAG — the whole tag, `v` included, because the tag is what git names and
 * the URL must name the same thing. (A template that added its own `v`
 * produced `releases/download/vv1.29.26/…`, and the pin that checks the
 * URL found it on the day it was written.)
 */
const DEFAULT_URL_TEMPLATE =
	'https://github.com/stewardmem/stewardmem/releases/download/{tag}';

/**
 * The release tag, and whether this repository HAS it.
 *
 * The tag is a separate fact from the app's version, and it is checked
 * against the repository rather than taken on trust: a URL naming a tag that
 * `git rev-parse` cannot resolve names a download that will 404. A manifest
 * is a promise the release keeps, so the writer keeps only the ones it can.
 */
export function releaseTagResolves(root, tag) {
	if (typeof tag !== 'string' || tag.length === 0) return false;
	const result = run('git', ['-C', root, 'rev-parse', '--verify', '--quiet', `refs/tags/${tag}`]);
	return result.status === 0 && result.stdout.trim().length > 0;
}

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
	releaseTag = '',
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
		release_tag: releaseTag,
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
	releaseTag = null,
	url = '',
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

	// The artefact's NAME is part of the URL, and a release asset does not
	// keep every character a file may carry. **MEASURED (2026-10-22):**
	// GitHub replaces a SPACE in an asset's name with a DOT, so an artefact
	// called `Brain Shell.app.tar.gz` is published as
	// `Brain.Shell.app.tar.gz` — and the manifest's URL, which names the
	// space (`Brain%20Shell.app.tar.gz`), then 404s. The updater would
	// download NOTHING, and the manifest would be a promise the release
	// cannot keep. A name the asset cannot keep is a name the URL cannot
	// name, so the writer refuses it rather than publish a URL that cannot
	// resolve.
	const artifactName = basename(artifact);
	if (/[ \t]/.test(artifactName)) {
		failures.push(
			`the artifact's name carries a character a release asset does not keep: ${artifactName} ` +
				'(a space is published as a dot, so the download URL would name a file that does not exist)'
		);
	}

	// The signature beside the artifact: present, and bound to THIS artifact
	// and THIS version.
	const signaturePath = `${artifact}.sig`;
	const binding = signatureBinding(signaturePath);
	if (binding === null) {
		failures.push(`the artifact carries no signature beside it: ${signaturePath}`);
	} else {
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

	// The URL must name a tag this repository HAS. The app's version and the
	// release tag are two different facts, and the URL fetches the release —
	// so the tag is its own input, and a tag that does not resolve is a
	// download that will 404. The refusal names the URL it refused.
	if (typeof releaseTag !== 'string' || releaseTag.length === 0) {
		failures.push(
			`the manifest names no release tag: the download URL must name the tag the artifact is fetched from (${url})`
		);
	} else if (!releaseTagResolves(root, releaseTag)) {
		failures.push(
			`the release tag does not resolve in this repository: ${releaseTag} (the download URL would name a tag that does not exist: ${url})`
		);
	} else if (!url.includes(releaseTag)) {
		// The declared tag and the URL must be the same fact. A manifest that
		// says `release_tag: v1.29.26` and fetches from another tag is two
		// truths, and the one a reader checks first is the one that lies.
		failures.push(
			`the download URL does not name the declared release tag: ${releaseTag} is not in ${url}`
		);
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
	// The RELEASE TAG is a writer input of its own: the URL fetches the
	// release, and the release is named by a tag, while `version` stays the
	// app's own version (the thing the updater compares). When the tag is
	// missing the URL says so, so the refusal can name the URL it refused.
	const releaseTag = options.releaseTag ?? null;
	const urlTag = releaseTag ?? '<no release tag>';
	const url = `${urlTemplate.replace('{tag}', urlTag)}/${encodeURIComponent(basename(options.artifact))}`;

	const failures = manifestFailures({
		...options,
		root,
		staged,
		version,
		kernelVersions,
		targets,
		urlTemplate,
		releaseTag,
		url
	});
	const signature = existsSync(`${options.artifact}.sig`)
		? readFileSync(`${options.artifact}.sig`, 'utf8').trim()
		: '';

	const manifest = manifestFor({
		version,
		releaseTag: releaseTag ?? '',
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
		releaseTag: options['release-tag'] ?? null,
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
	console.error('       --release-tag <vX.Y.Z>   the RELEASE tag the URL names (not the app version)');
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
