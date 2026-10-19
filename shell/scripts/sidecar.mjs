#!/usr/bin/env node
// @ts-nocheck — this is a Node-builtins CLI, executed by Node and DRIVEN by
// shell/tests/sidecar-bundle.test.ts. The test imports its pure exports; the
// module's own types are inferred through `allowJs`, but it is not a
// type-checked library and `checkJs` would demand annotations on every
// internal helper without proving anything the behavioral pins do not.
/**
 * III-1 — the kernel sidecar that ships INSIDE the Brain Shell.app bundle.
 *
 * One module, builtins only, two subcommands:
 *
 *   stage   Build the kernel for both Apple silicon and Intel, lipo a
 *           universal binary, stage all THREE files Tauri requires, and
 *           write SIDECAR_PROVENANCE.json. Refuses a dirty tree: a build
 *           record names a commit, and a dirty tree has no truthful one.
 *
 *   verify  Walk a real bundle, FIND every Mach-O by PATH (never by the
 *           line count `file` emits — a fat binary prints one line per
 *           slice plus a summary), verify the bundle seal, then each file
 *           in isolation with `--verify --strict --ignore-resources`, and
 *           require the hardened-runtime flag. Exit 2 = cannot verify;
 *           exit 1 = verified and failed, every failure named.
 *
 * Why the shapes below are what they are (all measured 2026-10-19):
 *
 *  - `externalBin: ["binaries/brain-server"]` resolves the FULL target
 *    triple. A universal build needs `-aarch64-apple-darwin`,
 *    `-x86_64-apple-darwin` (both compile steps) AND
 *    `-universal-apple-darwin` (the bundle copy step). Only the universal
 *    file present fails with `resource path ... doesn't exist`.
 *  - The wiring lives in `tauri.macos.conf.json`, never the base config:
 *    the base is read on Linux too, where the files do not exist.
 *  - Every Rust binary is LINKER-SIGNED. `codesign -dv` exits 0 on the
 *    unsigned bundle's internals, so `-dv` succeeding proves nothing. The
 *    discriminator is the runtime flag (`flags=…runtime`), and the per-file
 *    verify must use `--ignore-resources`: inside a broken bundle every
 *    nested file otherwise reports the BUNDLE's seal error.
 *  - Tauri re-signs the copied sidecar, so the staged and bundled bytes
 *    differ. Provenance hashes the STAGED artifacts; the bundled sidecar is
 *    identified by signature + the embedded `brain-server/<version>` stamp +
 *    arch parity. A sha equality pin there would be false.
 *  - An ad-hoc signature has no timestamp and `spctl` rejects it. Hardened
 *    runtime is pinned; the secure timestamp is OWED to III-5, not faked.
 */
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import {
	accessSync,
	chmodSync,
	constants as fsConstants,
	copyFileSync,
	existsSync,
	mkdirSync,
	readFileSync,
	readdirSync,
	statSync,
	writeFileSync
} from 'node:fs';
import { delimiter, dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

const STAGED_DIR = join(REPO_ROOT, 'shell', 'src-tauri', 'binaries');
const PROVENANCE_NAME = 'SIDECAR_PROVENANCE.json';
const PROVENANCE_RESOURCE = ['Contents', 'Resources', 'binaries', PROVENANCE_NAME];
const SIDECAR_IN_BUNDLE = ['Contents', 'MacOS', 'brain-server'];
const UNIVERSAL_TRIPLE = 'universal-apple-darwin';
const TARGETS = [
	{ triple: 'aarch64-apple-darwin', arch: 'arm64' },
	{ triple: 'x86_64-apple-darwin', arch: 'x86_64' }
];

/** cputype -> the spelling `file`/`lipo` use. */
const CPU_ARCH = new Map([
	[0x01000007, 'x86_64'],
	[0x0100000c, 'arm64'],
	[0x0200000c, 'arm64_32'],
	[0x00000007, 'i386'],
	[0x0000000c, 'arm'],
	[0x00000012, 'ppc'],
	[0x01000012, 'ppc64']
]);

function archName(cputype) {
	const name = CPU_ARCH.get(cputype >>> 0);
	return name ?? `cpu-0x${(cputype >>> 0).toString(16)}`;
}

/**
 * Read the arch set out of a thin or fat Mach-O header. Pure Node — the
 * verifier must not depend on `lipo`, which exists only on macOS.
 */
export function machoArchs(path) {
	const buf = readFileSync(path);
	if (buf.length < 8) return [];

	const magicLE = buf.readUInt32LE(0);
	const magicBE = buf.readUInt32BE(0);

	// Thin: MH_MAGIC/MH_MAGIC_64 (0xfeedface/0xfeedfacf) in either byte order.
	if (magicLE === 0xfeedface || magicLE === 0xfeedfacf) {
		return [archName(buf.readUInt32LE(4))];
	}
	if (magicBE === 0xfeedface || magicBE === 0xfeedfacf) {
		return [archName(buf.readUInt32BE(4))];
	}

	// Fat: FAT_MAGIC/FAT_MAGIC_64 (0xcafebabe/0xcafebabf) in either byte order.
	const fat = (read, is64) => {
		const count = read(4);
		const stride = is64 ? 32 : 20;
		const archs = [];
		for (let index = 0; index < count; index += 1) {
			const at = 8 + index * stride;
			if (at + stride > buf.length) break;
			archs.push(archName(read(at)));
		}
		return archs;
	};
	if (magicLE === 0xcafebabe || magicLE === 0xcafebabf) {
		return fat((offset) => buf.readUInt32LE(offset), magicLE === 0xcafebabf);
	}
	if (magicBE === 0xcafebabe || magicBE === 0xcafebabf) {
		return fat((offset) => buf.readUInt32BE(offset), magicBE === 0xcafebabf);
	}
	return [];
}

/**
 * The TWO versions the provenance record carries, read from the tree.
 * The kernel pins them equal (`readme_badges_and_openapi_version_are_derived_not_hand_typed`);
 * a divergence here means a bad checkout, so `stage` refuses it.
 */
export function readVersions(root = REPO_ROOT) {
	const cargo = readFileSync(join(root, 'Cargo.toml'), 'utf8');
	const packageBlock = /^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m.exec(cargo);
	if (packageBlock === null) {
		throw new Error('Cargo.toml has no [package] version');
	}
	const openapi = readFileSync(join(root, 'openapi.yaml'), 'utf8');
	const apiVersion = /^\s{2}x-api-version:\s*"([^"]+)"\s*$/m.exec(openapi);
	if (apiVersion === null) {
		throw new Error('openapi.yaml has no info.x-api-version line');
	}
	return {
		kernelVersion: packageBlock[1],
		xApiVersion: apiVersion[1]
	};
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

function kernelState(root = REPO_ROOT) {
	return {
		commit: git(root, ['rev-parse', 'HEAD']),
		describe: git(root, ['describe', '--tags', '--always']),
		dirty: git(root, ['status', '--porcelain']).length > 0
	};
}

export function sha256File(path) {
	return createHash('sha256').update(readFileSync(path)).digest('hex');
}

/**
 * The record `stage` writes. `built_at` is UTC ISO-8601; the artifacts are
 * the STAGED files, because those are the bytes the bundler copies.
 */
export function provenanceFor({ root = REPO_ROOT, artifacts, now = new Date() }) {
	const versions = readVersions(root);
	if (versions.kernelVersion !== versions.xApiVersion) {
		throw new Error(
			`version lockstep broken: Cargo.toml ${versions.kernelVersion} != openapi.yaml ${versions.xApiVersion}`
		);
	}
	const state = kernelState(root);
	return {
		schema: 1,
		kernel_commit: state.commit,
		kernel_describe: state.describe,
		kernel_version: versions.kernelVersion,
		x_api_version: versions.xApiVersion,
		source_dirty: state.dirty,
		built_at: now.toISOString(),
		artifacts: artifacts.map((artifact) => {
			const bytes = readFileSync(artifact.path);
			return {
				name: artifact.name,
				target: artifact.target,
				sha256: createHash('sha256').update(bytes).digest('hex'),
				size: bytes.length,
				archs: machoArchs(artifact.path)
			};
		})
	};
}

/**
 * The provenance law, read from the record SEALED inside the bundle. Returns
 * one message per violation; an empty array is the only pass.
 */
export function checkProvenance({ record, recordPath, bundle, root = REPO_ROOT }) {
	const failures = [];
	if (record === null || typeof record !== 'object') {
		failures.push(`provenance unrecorded: ${recordPath}`);
		return failures;
	}
	if (record.schema !== 1) {
		failures.push(`provenance schema: record ${String(record.schema)} != 1`);
	}
	if (record.source_dirty !== false) {
		failures.push(
			`provenance source_dirty: record ${String(record.source_dirty)} — a dirty tree cannot produce a truthful record`
		);
	}

	let versions;
	let state;
	try {
		versions = readVersions(root);
		state = kernelState(root);
	} catch (error) {
		failures.push(`provenance tree: ${error instanceof Error ? error.message : String(error)}`);
		return failures;
	}

	if (record.kernel_commit !== state.commit) {
		failures.push(
			`provenance kernel_commit: record ${String(record.kernel_commit)} != tree ${state.commit}`
		);
	}
	if (record.kernel_version !== versions.kernelVersion) {
		failures.push(
			`provenance kernel_version: record ${String(record.kernel_version)} != Cargo.toml ${versions.kernelVersion}`
		);
	}
	if (record.x_api_version !== versions.xApiVersion) {
		failures.push(
			`provenance x_api_version: record ${String(record.x_api_version)} != openapi.yaml ${versions.xApiVersion}`
		);
	}

	const artifacts = Array.isArray(record.artifacts) ? record.artifacts : [];
	const universal = artifacts.find(
		(artifact) =>
			artifact !== null &&
			typeof artifact === 'object' &&
			artifact.target === UNIVERSAL_TRIPLE &&
			artifact.name === `brain-server-${UNIVERSAL_TRIPLE}`
	);
	if (universal === undefined) {
		failures.push(`provenance universal: no brain-server-${UNIVERSAL_TRIPLE} artifact is recorded`);
	}

	// The bundled copy is NOT byte-identical to the staged one (Tauri signs
	// it). Identity is signature + embedded version stamp + arch parity.
	const sidecar = join(bundle, ...SIDECAR_IN_BUNDLE);
	if (!existsSync(sidecar)) {
		failures.push(`provenance sidecar: ${SIDECAR_IN_BUNDLE.join('/')} is absent from the bundle`);
	} else {
		const stamp = Buffer.from(`brain-server/${versions.kernelVersion}`);
		if (!readFileSync(sidecar).includes(stamp)) {
			failures.push(
				`provenance sidecar: bundled bytes carry no brain-server/${versions.kernelVersion} stamp`
			);
		}
	}
	return failures;
}

function stagedFailures(record, stagedDir) {
	const failures = [];
	const artifacts = Array.isArray(record?.artifacts) ? record.artifacts : [];
	for (const artifact of artifacts) {
		if (artifact === null || typeof artifact !== 'object' || typeof artifact.name !== 'string') {
			failures.push('staged record: an artifact entry carries no name');
			continue;
		}
		const path = join(stagedDir, artifact.name);
		if (!existsSync(path)) {
			failures.push(`staged ${artifact.name}: absent from ${stagedDir}`);
			continue;
		}
		const sha256 = sha256File(path);
		if (sha256 !== artifact.sha256) {
			failures.push(`staged ${artifact.name}: sha256 ${sha256} != record ${String(artifact.sha256)}`);
		}
		const size = statSync(path).size;
		if (size !== artifact.size) {
			failures.push(`staged ${artifact.name}: size ${size} != record ${String(artifact.size)}`);
		}
	}
	return failures;
}

function findOnPath(name) {
	const path = process.env.PATH ?? '';
	for (const directory of path.split(delimiter)) {
		if (directory.length === 0) continue;
		const candidate = join(directory, name);
		try {
			accessSync(candidate, fsConstants.X_OK);
			return candidate;
		} catch {
			// next PATH entry
		}
	}
	return null;
}

function walk(dir, prefix = '') {
	const found = [];
	let entries;
	try {
		entries = readdirSync(join(dir, prefix), { withFileTypes: true });
	} catch {
		return found;
	}
	for (const entry of entries) {
		const relative = prefix === '' ? entry.name : `${prefix}${sep}${entry.name}`;
		if (entry.isDirectory()) {
			found.push(...walk(dir, relative));
		} else if (entry.isFile() || entry.isSymbolicLink()) {
			found.push(relative.split(sep).join('/'));
		}
	}
	return found;
}

function fileDescription(path) {
	const result = run('file', ['-b', path]);
	const first = result.stdout.split(/\r?\n/)[0] ?? '';
	return first.trim();
}

/** The census: PATH per Mach-O, with its arch set. Never a line count. */
function census(bundle) {
	const found = [];
	for (const relative of walk(bundle).sort()) {
		const description = fileDescription(join(bundle, relative));
		if (!description.includes('Mach-O')) continue;
		found.push({ path: relative, archs: machoArchs(join(bundle, relative)) });
	}
	return found;
}

function mainExecutable(bundle, entries) {
	let executable = null;
	try {
		const plist = readFileSync(join(bundle, 'Contents', 'Info.plist'), 'utf8');
		const match = /<key>CFBundleExecutable<\/key>\s*<string>([^<]+)<\/string>/.exec(plist);
		if (match !== null) executable = match[1];
	} catch {
		// no plist: fall back to the conventional name
	}
	const name = executable ?? 'brain-shell';
	return entries.find((entry) => entry.path === `Contents/MacOS/${name}`);
}

function sameArchSet(left, right) {
	return [...left].sort().join(',') === [...right].sort().join(',');
}

function cannotVerify(reason) {
	console.error(`cannot verify: ${reason}`);
	process.exit(2);
}

function verifyBundle(bundleArgument, stagedArgument) {
	const bundle = resolve(bundleArgument);
	if (!existsSync(bundle) || !statSync(bundle).isDirectory()) {
		cannotVerify(`bundle absent: ${bundle}`);
	}
	if (findOnPath('codesign') === null) cannotVerify('codesign is not on PATH');
	if (findOnPath('file') === null) cannotVerify('file is not on PATH');

	const entries = census(bundle);
	if (entries.length === 0) cannotVerify(`no Mach-O files under ${bundle}`);

	console.log(`mach-o files: ${entries.length}`);
	for (const entry of entries) {
		console.log(`  ${entry.path} [${entry.archs.join(' ') || 'unreadable'}]`);
	}

	const failures = [];
	if (entries.length < 2) {
		failures.push(
			`census floor: ${entries.length} Mach-O file(s) — the main executable and the sidecar are both required`
		);
	}
	const sidecarPath = SIDECAR_IN_BUNDLE.join('/');
	if (!entries.some((entry) => entry.path === sidecarPath)) {
		failures.push(`membership floor: ${sidecarPath} is not among the Mach-O files`);
	}

	const seal = run('codesign', ['--verify', '--strict', bundle]);
	if (seal.status !== 0) {
		failures.push(`signature bundle: ${output(seal) || `codesign exited ${String(seal.status)}`}`);
	}

	for (const entry of entries) {
		const file = join(bundle, entry.path);
		if (entry.archs.length === 0) {
			failures.push(`arch ${entry.path}: unreadable Mach-O header`);
		}
		const verify = run('codesign', ['--verify', '--strict', '--ignore-resources', file]);
		if (verify.status !== 0) {
			failures.push(
				`signature ${entry.path}: ${output(verify) || `codesign exited ${String(verify.status)}`}`
			);
		}
		const display = run('codesign', ['-dv', '--verbose=2', file]);
		const details = `${display.stdout}\n${display.stderr}`;
		const flags = /flags=([^\s]+)/.exec(details)?.[1] ?? 'absent';
		if (!details.includes('runtime')) {
			failures.push(
				`runtime ${entry.path}: flags=${flags} — a linker-signed or unsigned Mach-O is not a signed sidecar`
			);
		}
	}

	const main = mainExecutable(bundle, entries);
	if (main === undefined) {
		failures.push('arch main: CFBundleExecutable is not among the Mach-O files');
	} else {
		for (const entry of entries) {
			if (entry.path === main.path) continue;
			if (!sameArchSet(entry.archs, main.archs)) {
				failures.push(
					`arch ${entry.path}: [${entry.archs.join(' ')}] != main [${main.archs.join(' ')}]`
				);
			}
		}
	}

	const recordPath = join(bundle, ...PROVENANCE_RESOURCE);
	let record = null;
	if (!existsSync(recordPath)) {
		failures.push(`provenance unrecorded: ${recordPath}`);
	} else {
		try {
			record = JSON.parse(readFileSync(recordPath, 'utf8'));
		} catch (error) {
			failures.push(
				`provenance unreadable: ${recordPath} — ${error instanceof Error ? error.message : String(error)}`
			);
		}
	}
	if (record !== null) {
		failures.push(...checkProvenance({ record, recordPath, bundle }));
		const staged = stagedArgument === null ? (existsSync(STAGED_DIR) ? STAGED_DIR : null) : resolve(stagedArgument);
		if (staged !== null) {
			if (!existsSync(staged)) {
				failures.push(`staged directory absent: ${staged}`);
			} else {
				failures.push(...stagedFailures(record, staged));
			}
		}
	}

	if (failures.length > 0) {
		for (const failure of failures) console.error(`FAIL ${failure}`);
		process.exit(1);
	}
	const version = record?.kernel_version ?? 'unknown';
	const commit = typeof record?.kernel_commit === 'string' ? record.kernel_commit.slice(0, 12) : 'unknown';
	console.log(
		`ok — bundle seal, ${entries.length} Mach-O files, hardened runtime, arch parity [${main?.archs.join(' ') ?? 'unknown'}], provenance ${version} @ ${commit}`
	);
}

function stage() {
	if (process.platform !== 'darwin') {
		console.error('stage requires macOS: it builds both Apple targets and lipos them');
		process.exit(2);
	}

	const dirty = git(REPO_ROOT, ['status', '--porcelain']);
	if (dirty.length > 0) {
		console.error('refusing to stage: the working tree is dirty (a build record names a commit):');
		for (const line of dirty.split('\n')) console.error(`  ${line}`);
		process.exit(2);
	}

	let versions;
	try {
		versions = readVersions(REPO_ROOT);
	} catch (error) {
		console.error(`refusing to stage: ${error instanceof Error ? error.message : String(error)}`);
		process.exit(2);
	}
	if (versions.kernelVersion !== versions.xApiVersion) {
		console.error(
			`refusing to stage: version lockstep broken — Cargo.toml ${versions.kernelVersion} != openapi.yaml ${versions.xApiVersion}`
		);
		process.exit(2);
	}

	const installed = run('rustup', ['target', 'list', '--installed']);
	if (installed.status !== 0) {
		console.error(`refusing to stage: rustup target list failed — ${output(installed)}`);
		process.exit(2);
	}
	const present = new Set(installed.stdout.split(/\r?\n/).map((line) => line.trim()));
	const missing = TARGETS.filter((target) => !present.has(target.triple));
	if (missing.length > 0) {
		console.error(`refusing to stage: missing Rust target(s) ${missing.map((m) => m.triple).join(', ')}`);
		for (const target of missing) console.error(`  rustup target add ${target.triple}`);
		process.exit(2);
	}

	const started = Date.now();
	for (const target of TARGETS) {
		console.log(`[stage] cargo build --release --locked --bin brain-server --target ${target.triple}`);
		const build = spawnSync(
			'cargo',
			['build', '--release', '--locked', '--bin', 'brain-server', '--target', target.triple],
			{ cwd: REPO_ROOT, stdio: 'inherit' }
		);
		if (build.status !== 0) {
			console.error(`[stage] cargo build failed for ${target.triple}`);
			process.exit(1);
		}
	}

	// The stale-copy guard: an installed 1.29.21 binary must never satisfy a
	// 1.29.22 tree. The native slice runs here; the Intel slice cannot run on
	// an Apple-silicon host (no Rosetta), which is a documented local limit.
	const native = join(REPO_ROOT, 'target', 'aarch64-apple-darwin', 'release', 'brain-server');
	const expected = `brain-server ${versions.kernelVersion}`;
	const reported = run(native, ['--version']);
	const actual = reported.stdout.trim();
	if (reported.status !== 0 || actual !== expected) {
		console.error(
			`[stage] stale-copy guard: ${native} --version printed ${JSON.stringify(actual)}; expected ${JSON.stringify(expected)}`
		);
		process.exit(1);
	}

	mkdirSync(STAGED_DIR, { recursive: true });
	const artifacts = [];
	for (const target of TARGETS) {
		const source = join(REPO_ROOT, 'target', target.triple, 'release', 'brain-server');
		if (!existsSync(source)) {
			console.error(`[stage] build output absent: ${source}`);
			process.exit(1);
		}
		const archs = machoArchs(source);
		if (archs.length !== 1 || archs[0] !== target.arch) {
			console.error(
				`[stage] ${source} carries [${archs.join(' ')}], expected exactly [${target.arch}]`
			);
			process.exit(1);
		}
		const name = `brain-server-${target.triple}`;
		const stagedPath = join(STAGED_DIR, name);
		copyFileSync(source, stagedPath);
		chmodSync(stagedPath, 0o755);
		artifacts.push({ name, target: target.triple, path: stagedPath });
	}

	const universalName = `brain-server-${UNIVERSAL_TRIPLE}`;
	const universalPath = join(STAGED_DIR, universalName);
	const lipo = run('lipo', [
		'-create',
		artifacts[0].path,
		artifacts[1].path,
		'-output',
		universalPath
	]);
	if (lipo.status !== 0) {
		console.error(`[stage] lipo failed: ${output(lipo)}`);
		process.exit(1);
	}
	chmodSync(universalPath, 0o755);
	const universalArchs = machoArchs(universalPath);
	const expectedArchs = ['arm64', 'x86_64'];
	if (!sameArchSet(universalArchs, expectedArchs)) {
		console.error(
			`[stage] universal binary carries [${universalArchs.join(' ')}], expected [${expectedArchs.join(' ')}]`
		);
		process.exit(1);
	}
	artifacts.push({ name: universalName, target: UNIVERSAL_TRIPLE, path: universalPath });

	// The record is written LAST, after every artifact exists. A reader that
	// finds a record can therefore trust every entry it names.
	const record = provenanceFor({ root: REPO_ROOT, artifacts });
	writeFileSync(join(STAGED_DIR, PROVENANCE_NAME), `${JSON.stringify(record, null, 2)}\n`);

	console.log(`[stage] staged (${((Date.now() - started) / 1000).toFixed(1)}s):`);
	for (const artifact of artifacts) {
		const entry = record.artifacts.find((candidate) => candidate.name === artifact.name);
		console.log(
			`  ${artifact.path}  ${String(entry?.sha256)}  ${String(entry?.size)}B  [${(entry?.archs ?? []).join(' ')}]`
		);
	}
	console.log(`[stage] record: ${join(STAGED_DIR, PROVENANCE_NAME)}`);
	console.log(JSON.stringify(record, null, 2));
}

function usage() {
	console.error('usage:');
	console.error('  node shell/scripts/sidecar.mjs stage');
	console.error('  node shell/scripts/sidecar.mjs verify <bundle> [--staged <dir>]');
	process.exit(2);
}

function main() {
	const [command, ...rest] = process.argv.slice(2);
	if (command === 'stage') {
		stage();
		return;
	}
	if (command === 'verify') {
		const bundle = rest[0];
		if (bundle === undefined) usage();
		let staged = null;
		for (let index = 1; index < rest.length; index += 1) {
			if (rest[index] === '--staged') {
				staged = rest[index + 1] ?? null;
				if (staged === null) usage();
				index += 1;
			} else {
				usage();
			}
		}
		verifyBundle(bundle, staged);
		return;
	}
	usage();
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
	main();
}
