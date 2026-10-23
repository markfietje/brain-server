/**
 * The test lane's ambient surface — the Node APIs only the plugin's OWN pins
 * use, and nothing else.
 *
 * The plugin deliberately carries no `@types/node`: its production ambient set
 * (`src/ambient.d.ts`) is the minimal surface the plugin's own code needs, so
 * the tree type-checks against exactly the APIs it can name. This file extends
 * that set for the test lane — a pin that must drive a real process (the
 * standalone install, the integration drill) needs `node:child_process`, and
 * the production code never does. Ambient module declarations MERGE, so the
 * overloads here extend `src/ambient.d.ts`'s surface rather than replacing it.
 */

declare type BufferLike = {
	length: number;
	toString(encoding?: string): string;
	subarray(start?: number, end?: number): BufferLike;
	equals(other: BufferLike): boolean;
};

declare const Buffer: {
	from(input: string, encoding?: string): BufferLike;
	concat(list: BufferLike[]): BufferLike;
};

declare module "node:fs" {
	/** The one-argument form: without an encoding the file comes back as bytes. */
	export function readFileSync(path: string): BufferLike;

	/** The endpoint arms mirror the drill into a scratch tree beside a hostile
	 * app configuration, so the lane needs the two directory/copy primitives. */
	export function mkdirSync(path: string, options?: { recursive?: boolean }): void;
	export function copyFileSync(src: string, dest: string): void;
}

declare module "node:url" {
	/** Used by `vitest.config.ts` to turn the alias's URL into a path. */
	export function fileURLToPath(url: URL): string;
}

declare module "node:child_process" {
	export type SpawnSyncResult = {
		status: number | null;
		stdout: string;
		stderr: string;
		error?: Error;
	};

	export type SpawnSyncOptions = {
		cwd?: string;
		encoding?: string;
		timeout?: number;
		env?: Record<string, string | undefined>;
		maxBuffer?: number;
	};

	export function spawnSync(
		file: string,
		args?: string[],
		options?: SpawnSyncOptions,
	): SpawnSyncResult;
}

declare module "node:crypto" {
	export type KeyObject = {
		export(options: { format: "der" | "pem"; type: "spki" | "pkcs8" }): BufferLike;
	};

	export function createHash(algorithm: string): {
		update(data: BufferLike): { digest(): BufferLike };
	};

	export function generateKeyPairSync(type: "ed25519"): {
		publicKey: KeyObject;
		privateKey: KeyObject;
	};

	export function sign(algorithm: null, data: BufferLike, key: KeyObject): BufferLike;

	export function verify(
		algorithm: null,
		data: BufferLike,
		key: KeyObject,
		signature: BufferLike,
	): boolean;
}
