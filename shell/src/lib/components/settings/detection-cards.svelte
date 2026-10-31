<script lang="ts">
	// W-T2 — the proposal cards: the machine's REPORT rendered beside the ONE
	// choice. Deterministic end to end: the Rust detection core reads the
	// documented paths, this renders its DTO — no model, no generation, no
	// network. THE EVIDENCE LAW: every claim carries the path it was read
	// from, because a proposal the user cannot verify is a guess wearing a
	// badge; and nothing-found renders "nothing was touched", never a silent
	// absence.
	import { m } from '$paraglide/messages.js';
	import { formatBytes, type DetectDto } from '#lib/detection.js';

	let { report }: { report: DetectDto | null } = $props();

	const found = $derived(
		report !== null &&
			(report.existing !== null ||
				report.openclaw !== null ||
				report.workspace !== null ||
				report.kernelOnPort.running ||
				report.updates.length > 0)
	);
</script>

{#if report !== null}
	<div class="grid gap-2" data-testid="detection-cards">
		{#if report.existing !== null}
			<div class="rounded-md border border-border p-3" data-testid="detection-card-existing">
				<p class="text-sm">
					{m['posture.card.existing']({
						version: report.existing.version ?? '?',
						path: report.existing.binaryPath
					})}
				</p>
				<p class="mt-1 text-xs text-muted-foreground" data-testid="detection-evidence-existing">
					{m['posture.card.evidence']({ path: report.existing.evidence })}
				</p>
			</div>
		{/if}
		{#if report.openclaw !== null}
			<div class="rounded-md border border-border p-3" data-testid="detection-card-openclaw">
				<p class="text-sm">{m['posture.card.openclaw']()}</p>
				<p class="mt-1 text-xs text-muted-foreground" data-testid="detection-evidence-openclaw">
					{m['posture.card.evidence']({ path: report.openclaw.evidence })}
				</p>
			</div>
		{/if}
		{#if report.workspace !== null}
			<div class="rounded-md border border-border p-3" data-testid="detection-card-workspace">
				<p class="text-sm">
					{m['posture.card.workspace']({
						count: report.workspace.databases.length,
						size: formatBytes(report.workspace.totalBytes)
					})}
				</p>
				<p class="mt-1 text-sm text-muted-foreground">
					{m['posture.card.workspace.later']()}
				</p>
				<p class="mt-1 text-xs text-muted-foreground" data-testid="detection-evidence-workspace">
					{m['posture.card.evidence']({ path: report.workspace.evidence })}
				</p>
			</div>
		{/if}
		{#if report.kernelOnPort.running}
			<div class="rounded-md border border-border p-3" data-testid="detection-card-kernel">
				<p class="text-sm">
					{m['posture.card.kernel']({ version: report.kernelOnPort.version ?? '?' })}
				</p>
				<p class="mt-1 text-xs text-muted-foreground" data-testid="detection-evidence-kernel">
					{m['posture.card.evidence']({ path: report.kernelOnPort.evidence })}
				</p>
			</div>
		{/if}
		{#each report.updates as update (update.binary)}
			<div class="rounded-md border border-border p-3" data-testid="detection-card-update">
				<p class="text-sm">
					{m['posture.card.update']({
						binary: update.binary,
						installed: update.installedVersion ?? '?',
						offered: update.offeredVersion ?? '?'
					})}
				</p>
				<p class="mt-1 text-xs text-muted-foreground" data-testid="detection-evidence-update">
					{m['posture.card.evidence']({ path: update.evidence })}
				</p>
			</div>
		{/each}
		{#if !found}
			<p class="text-xs text-muted-foreground" data-testid="detection-nothing-found">
				{m['posture.card.nothingFound']()}
			</p>
		{/if}
	</div>
{/if}
