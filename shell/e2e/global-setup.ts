/**
 * The e2e global setup: boot a REAL loopback kernel from the repo's own
 * offline recipe (the aqueduct-smoke shape: BIND_PORT + temp BRAIN_DB_PATH,
 * /health poll), wait for it, and tear it down after the run.
 *
 * SAFETY LAWS (learned the hard way, recorded):
 *  - The SERVER binary is `target/debug/brain-server` (src/main.rs). The
 *    `brain` bin is the CLIENT CLI — never boot that as a server.
 *  - The operator's own live kernel may be running on the DEFAULT port
 *    8765 on a dev machine. The e2e therefore boots on a DEDICATED port
 *    (8799) and REFUSES to start unless that port is free — a health
 *    response from a process we did not boot is never acceptable.
 *  - The health poll also requires OUR child process to stay alive, so a
 *    dead boot fails fast instead of answering from a stranger.
 *
 * The token store stays unconfigured — loopback requests pass through as
 * the opaque back-compat principal, which is exactly the Phase A posture.
 * The page is REBUILT with VITE_BRAIN_API_BASE pointed at the e2e kernel
 * (build/ is gitignored; the next plain `pnpm build` restores the default
 * origin). The test context sets bypassCSP because the built page's strict
 * CSP meta pins ONLY the default kernel origin (8765) — the CSP pin
 * itself is asserted by the spec against build/index.html, so the shipped
 * artifact stays strict while the harness crosses its own test origin.
 */
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { createConnection } from 'node:net';
import { mkdtempSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const shellRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const kernelRoot = join(shellRoot, '..');
const KERNEL_PORT = Number(process.env['E2E_KERNEL_PORT'] ?? 8799);
const KERNEL_URL = `http://127.0.0.1:${KERNEL_PORT}`;

let kernel: ChildProcess | null = null;
let dataDir: string | null = null;

function portIsFree(port: number): Promise<boolean> {
	return new Promise((resolve) => {
		const socket = createConnection({ port, host: '127.0.0.1' });
		socket.once('connect', () => {
			socket.destroy();
			resolve(false);
		});
		socket.once('error', () => {
			socket.destroy();
			resolve(true);
		});
	});
}

async function waitForHealth(timeoutMs: number): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		if (kernel && kernel.exitCode !== null) {
			throw new Error(`the e2e kernel exited early (code ${kernel.exitCode})`);
		}
		try {
			const res = await fetch(`${KERNEL_URL}/health`);
			if (res.ok) return;
		} catch {
			// not up yet
		}
		await new Promise((r) => setTimeout(r, 500));
	}
	throw new Error(`the e2e kernel did not become healthy on ${KERNEL_URL} in time`);
}

export default async function () {
	if (!(await portIsFree(KERNEL_PORT))) {
		throw new Error(
			`port ${KERNEL_PORT} is already in use — refusing to run the e2e against a ` +
				'process this harness did not boot (the operator may run a live kernel; ' +
				'never test against it). Free the port or set E2E_KERNEL_PORT.'
		);
	}
	const bin = join(kernelRoot, 'target/debug/brain-server');
	if (!existsSync(bin)) {
		console.log('[e2e] building the kernel server binary (debug)…');
		const build = spawnSync('cargo', ['build', '--offline', '--locked', '--bin', 'brain-server'], {
			cwd: kernelRoot,
			stdio: 'inherit'
		});
		if (build.status !== 0) throw new Error('kernel server build failed');
	}
	dataDir = mkdtempSync(join(tmpdir(), 'wizard-e2e-kernel-'));
	console.log(`[e2e] booting the kernel server from ${dataDir} on ${KERNEL_PORT}`);
	kernel = spawn(bin, [], {
		cwd: kernelRoot,
		env: {
			...process.env,
			BIND_HOST: '127.0.0.1',
			BIND_PORT: String(KERNEL_PORT),
			BRAIN_DB_PATH: join(dataDir, 'brain.db'),
			// The page (preview origin, loopback) is a cross-origin caller:
			// the kernel's CORS allowlist is explicit and loopback-guarded —
			// the e2e declares exactly its own preview origin, nothing wider.
			CORS_ORIGINS: 'http://127.0.0.1:4173'
		},
		stdio: ['ignore', 'pipe', 'pipe']
	});
	kernel.stdout?.on('data', (d) => process.stdout.write(`[kernel] ${d}`));
	kernel.stderr?.on('data', (d) => process.stderr.write(`[kernel!] ${d}`));
	await waitForHealth(120_000);
	console.log('[e2e] e2e kernel healthy');

	// The page rebuild against this kernel origin happens in the playwright
	// webServer command (build → preview as ONE step, so preview never
	// serves a build replaced underneath it — see playwright.config.ts).

	return async () => {
		if (kernel) kernel.kill('SIGTERM');
		if (dataDir) rmSync(dataDir, { recursive: true, force: true });
		console.log('[e2e] kernel stopped, data dir removed');
	};
}
