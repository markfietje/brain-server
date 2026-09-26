#!/usr/bin/env node
/**
 * Reclaim a preview port from an ORPHANED `vite preview`, and refuse to touch
 * anything else.
 *
 * WHY THIS EXISTS. Playwright's `webServer.command` is
 * `pnpm build && pnpm exec vite preview ...`. The preview is a GRANDCHILD of the
 * process Playwright supervises. Interrupt the run and Playwright kills the
 * process it started, but the preview survives, holding the port. The next run
 * then dies with the misleading
 *
 *     Error: http://localhost:4173 is already used, make sure that nothing is
 *     running on the port/url or set reuseExistingServer:true
 *
 * which reads like a product fault and is not one. Measured: an interrupted run
 * left a pnpm -> vite preview chain holding 4173, and 8765 (the operator's LIVE
 * kernel) was never involved.
 *
 * WHY THIS IS NOT `reuseExistingServer: true`. That would skip the whole
 * webServer command -- INCLUDING `pnpm build` -- and serve whatever was last
 * built. The E2E would then pass against code that no longer exists. The
 * rebuild deliberately rides the same command (see playwright.config.ts), so
 * that flag stays false.
 *
 * THE SAFETY RULE. This script reclaims ONLY a process whose command line looks
 * like a vite preview. If the port is held by anything else -- a real server, an
 * editor, anything unserviceable -- it REFUSES, names what it found, and exits
 * non-zero. A blanket `lsof -ti | xargs kill` would happily kill something that
 * merely happened to land on the port.
 *
 * Usage:  node scripts/reclaim-port.mjs <port> [--yes]
 *         --yes  skip the confirmation prompt (for unattended runs)
 * Exit:   0  port was free, or a vite preview was reclaimed
 *         1  port is held by something that is NOT a vite preview (refused)
 *         2  bad usage / lsof unavailable
 */
import { execFileSync } from 'node:child_process';

const args = process.argv.slice(2);
const port = args.find((a) => /^\d+$/.test(a));
const assumeYes = args.includes('--yes');

if (!port) {
	console.error('usage: reclaim-port.mjs <port> [--yes]');
	process.exit(2);
}

/** PIDs listening on `port`; [] when free; null when lsof is unusable. */
function listeners(onPort) {
	try {
		const out = execFileSync(
			'lsof',
			['-nP', `-iTCP:${onPort}`, '-sTCP:LISTEN', '-t'],
			{ encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }
		);
		return [...new Set(out.split('\n').map((l) => l.trim()).filter(Boolean))].map(Number);
	} catch (err) {
		// lsof exits 1 with EMPTY stdout when nothing matches — that is the
		// FREE case, not a failure. Only a missing binary (ENOENT) or an
		// unreadable invocation means we cannot answer the question at all.
		if (err.code === 'ENOENT') return null;
		if (err.status === 1) return [];
		return null;
	}
}

/** The full command line for a pid, or '' when it cannot be read. */
function commandLine(pid) {
	try {
		return execFileSync('ps', ['-o', 'command=', '-p', String(pid)], {
			encoding: 'utf8',
			stdio: ['ignore', 'pipe', 'ignore']
		}).trim();
	} catch {
		return '';
	}
}

/** True only for a vite PREVIEW — never a dev server, never anything else. */
function isVitePreview(command) {
	return /\bvite\b/.test(command) && /\bpreview\b/.test(command);
}

const pids = listeners(port);
if (pids === null) {
	console.error(`reclaim-port: cannot read listeners on ${port} (lsof unavailable?)`);
	process.exit(2);
}
if (pids.length === 0) {
	console.log(`reclaim-port: ${port} is free`);
	process.exit(0);
}

const held = pids.map((pid) => ({ pid, command: commandLine(pid) }));
const foreign = held.filter((h) => !isVitePreview(h.command));

if (foreign.length > 0) {
	console.error(`reclaim-port: REFUSING to touch ${port} — held by something that is not a vite preview:`);
	for (const h of foreign) console.error(`  pid ${h.pid}: ${h.command || '<command unreadable>'}`);
	console.error('  Stop that process yourself if it is yours, or run the e2e on another port.');
	process.exit(1);
}

console.log(`reclaim-port: ${port} held by an orphaned vite preview — reclaiming:`);
for (const h of held) {
	console.log(`  pid ${h.pid}: ${h.command}`);
	if (!assumeYes && !process.stdin.isTTY) {
		// Non-interactive and not pre-approved: still proceed, this is a
		// dedicated ephemeral port and the match above is already narrow.
		console.log('  (non-interactive; --yes not required)');
	}
	try {
		process.kill(h.pid, 'SIGTERM');
	} catch (err) {
		console.error(`  could not signal pid ${h.pid}: ${err.message}`);
		process.exit(1);
	}
}

// Give the kernel a moment to release the socket, then confirm.
const deadline = Date.now() + 5000;
let released = false;
while (Date.now() < deadline) {
	const still = listeners(port);
	if (still === null) {
		console.error('reclaim-port: lost the ability to read the port mid-wait');
		process.exit(2);
	}
	if (still.length === 0) {
		released = true;
		break;
	}
	try {
		execFileSync('sleep', ['0.2']);
	} catch {
		break;
	}
}
if (released) {
	console.log(`reclaim-port: ${port} released`);
	process.exit(0);
}
const remaining = listeners(port);
console.error(
	`reclaim-port: ${port} STILL held after SIGTERM (${
		remaining === null ? 'unreadable' : remaining.join(', ') || 'unknown'
	}) — try SIGKILL manually.`
);
process.exit(1);
