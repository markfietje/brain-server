<script lang="ts">
	// W-2/W-4 — "How StewardMem runs": the one-time gate decision, re-runnable
	// here (the plan's law: a Settings SECTION, not a dead-end dialog). Every
	// transition ends in a NAMED outcome — installed, already installed, the
	// port's named holder, the uninstall's removed list and its OFFERED data
	// (never a silent deletion). Direct channel only: the store edition's
	// core has none of these commands (the config boundary compiles them
	// out), so this section renders nothing there — the mas-boundary pin
	// holds that shape.
	import { m } from '$paraglide/messages.js';
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import DetectionCards from '#lib/components/settings/detection-cards.svelte';
	import type { DetectDto } from '#lib/detection.js';

	interface PostureDto {
		posture: 'app' | 'service' | 'external';
		url?: string | null;
		chosen: boolean;
	}

	interface InstallDto {
		outcome: string;
		holder?: string | null;
		expected?: string | null;
		observed?: string | null;
		kind?: string | null;
		error?: string | null;
	}

	interface UninstallDto {
		outcome: string;
		removed: string[];
		offered: string[];
		residue: string[];
		kind?: string | null;
		error?: string | null;
	}

	let posture = $state<PostureDto | null>(null);
	let supported = $state(false);
	let busy = $state(false);
	let choice = $state<'app' | 'service' | 'external'>('app');
	let externalUrl = $state('');
	let externalInvalid = $state(false);
	let outcome = $state<{ kind: 'ok' | 'refused'; sentence: string } | null>(null);
	let detectReport = $state<DetectDto | null>(null);

	// W-T2 — the same report the gate's ONE screen shows, re-run beside the
	// same choice here. Loaded when Change unrolls (never on mount — the
	// section's posture read stays the only mount-time probe).
	async function loadDetections(): Promise<void> {
		if (!inTauri() || detectReport !== null) return;
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			detectReport = (await invoke('detect_cards')) as DetectDto;
		} catch {
			detectReport = null; // an older core: no cards, the choice stands
		}
	}
	let offeredPaths = $state<string[]>([]);
	let removedSummary = $state('');

	function inTauri(): boolean {
		return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
	}

	onMount(async () => {
		if (!inTauri()) return;
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			if ((await invoke('build_channel')) !== 'direct') return;
			supported = true;
			posture = (await invoke('service_posture')) as PostureDto;
			if (posture.posture === 'external' && posture.url) externalUrl = posture.url;
		} catch {
			// The core did not answer: the section stays inert rather than
			// pretending a posture it cannot see.
			posture = null;
		}
	});

	function currentSentence(p: PostureDto): string {
		if (p.posture === 'service') return m['settings.runmode.current.service']();
		if (p.posture === 'external' && p.url) {
			return m['settings.runmode.current.external']({ url: p.url });
		}
		return m['settings.runmode.current.app']();
	}

	async function apply(kind: 'app' | 'service' | 'external'): Promise<void> {
		if (!inTauri() || busy) return;
		const { invoke } = await import('@tauri-apps/api/core');
		busy = true;
		outcome = null;
		try {
			if (kind === 'external') {
				const url = externalUrl.trim();
				if (!isLoopbackUrl(url)) {
					externalInvalid = true;
					return;
				}
				externalInvalid = false;
				await invoke('store_service_posture', { posture: 'external', url });
				outcome = {
					kind: 'ok',
					sentence: m['settings.runmode.current.external']({ url })
				};
				posture = { posture: 'external', url, chosen: true };
				return;
			}
			await invoke('store_service_posture', { posture: kind });
			if (kind === 'app') {
				outcome = { kind: 'ok', sentence: m['settings.runmode.current.app']() };
				posture = { posture: 'app', url: null, chosen: true };
				return;
			}
			const install = (await invoke('service_install')) as InstallDto;
			if (install.outcome === 'installed' || install.outcome === 'alreadyInstalled') {
				outcome = {
					kind: 'ok',
					sentence:
						install.outcome === 'installed'
							? m['posture.outcome.installed']()
							: m['posture.outcome.alreadyInstalled']()
				};
				posture = { posture: 'service', url: null, chosen: true };
				return;
			}
			outcome =
				install.outcome === 'portHeld'
					? {
							kind: 'refused',
							sentence: m['posture.outcome.portHeld']({ holder: install.holder ?? '' })
						}
					: {
							kind: 'refused',
							sentence: m['posture.outcome.refused']({
								error: install.error ?? (install.kind ?? '')
							})
						};
		} catch (error) {
			outcome = {
				kind: 'refused',
				sentence: m['settings.runmode.serviceError']({
					error: error instanceof Error ? error.message : String(error)
				})
			};
		} finally {
			busy = false;
		}
	}

	async function uninstall(): Promise<void> {
		if (!inTauri() || busy) return;
		const { invoke } = await import('@tauri-apps/api/core');
		busy = true;
		outcome = null;
		try {
			await invoke('store_service_posture', { posture: 'app' });
			const dto = (await invoke('service_uninstall')) as UninstallDto;
			if (dto.outcome === 'refused') {
				outcome = {
					kind: 'refused',
					sentence: m['settings.runmode.serviceError']({ error: dto.error ?? (dto.kind ?? '') })
				};
				return;
			}
			removedSummary = dto.removed.join(' · ');
			offeredPaths = dto.offered;
			posture = { posture: 'app', url: null, chosen: true };
			outcome = {
				kind: 'ok',
				sentence:
					dto.outcome === 'notInstalled'
						? m['settings.runmode.notInstalled']()
						: m['settings.runmode.uninstalled']({ removed: removedSummary })
			};
		} catch (error) {
			outcome = {
				kind: 'refused',
				sentence: m['settings.runmode.serviceError']({
					error: error instanceof Error ? error.message : String(error)
				})
			};
		} finally {
			busy = false;
		}
	}

	async function removeData(): Promise<void> {
		if (!inTauri() || busy) return;
		const { invoke } = await import('@tauri-apps/api/core');
		busy = true;
		try {
			await invoke('service_remove_data');
			offeredPaths = [];
			outcome = { kind: 'ok', sentence: m['settings.runmode.removed']() };
		} catch {
			// The removal names its own refusals; a failure here leaves the
			// offer standing rather than pretending the data is gone.
			outcome = { kind: 'refused', sentence: m['settings.runmode.removeFailed']() };
		} finally {
			busy = false;
		}
	}

	/** The wire's standing local-only law, mirrored at the choice's door
	 * (the authoritative check lives with the wire in client.ts). */
	function isLoopbackUrl(raw: string): boolean {
		try {
			const parsed = new URL(raw);
			if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return false;
			const host = parsed.hostname;
			return host === '127.0.0.1' || host === 'localhost' || host === '[::1]' || host === '::1';
		} catch {
			return false;
		}
	}
</script>

{#if supported && posture !== null}
	<div class="grid gap-2" data-testid="settings-runmode">
		<h3 class="text-sm font-semibold">{m['settings.runmode.heading']()}</h3>
		<p class="text-sm text-muted-foreground" data-testid="settings-runmode-current">
			{currentSentence(posture)}
		</p>

		<details
			class="mt-1"
			ontoggle={(event) => {
				if (event.currentTarget.open) void loadDetections();
			}}
		>
			<summary class="cursor-pointer text-sm">{m['settings.runmode.choose']()}</summary>
			<div class="mt-2 grid gap-2">
				<DetectionCards report={detectReport} />
				<label class="flex items-start gap-3 rounded-md border border-border p-3">
					<input
						type="radio"
						name="runmode"
						value="app"
						bind:group={choice}
						class="mt-1"
						data-testid="settings-runmode-app"
					/>
					<span>
						<span class="block text-sm font-medium">{m['posture.a.title']()}</span>
						<span class="block text-sm text-muted-foreground">{m['posture.a.detail']()}</span>
					</span>
				</label>
				<label class="flex items-start gap-3 rounded-md border border-border p-3">
					<input
						type="radio"
						name="runmode"
						value="service"
						bind:group={choice}
						class="mt-1"
						data-testid="settings-runmode-service"
					/>
					<span>
						<span class="block text-sm font-medium">{m['posture.b.title']()}</span>
						<span class="block text-sm text-muted-foreground">{m['posture.b.detail']()}</span>
					</span>
				</label>
				<label class="flex items-start gap-3 rounded-md border border-border p-3">
					<input
						type="radio"
						name="runmode"
						value="external"
						bind:group={choice}
						class="mt-1"
						data-testid="settings-runmode-external"
					/>
					<span>
						<span class="block text-sm font-medium">{m['posture.c.title']()}</span>
						<span class="block text-sm text-muted-foreground">{m['posture.c.detail']()}</span>
					</span>
				</label>
				{#if choice === 'external'}
					<div class="grid gap-1">
						<label for="runmode-url" class="text-sm font-medium">
							{m['posture.c.urlLabel']()}
						</label>
						<input
							id="runmode-url"
							data-testid="settings-runmode-url"
							bind:value={externalUrl}
							placeholder="http://127.0.0.1:8765"
							class="rounded-md border border-border bg-background px-3 py-2 text-sm"
						/>
						{#if externalInvalid}
							<p class="text-sm text-destructive" role="alert">
								{m['posture.c.urlInvalid']()}
							</p>
						{/if}
					</div>
				{/if}
				{#if choice === 'service'}
					<p class="text-xs text-muted-foreground">{m['posture.signal']()}</p>
				{/if}
				<div>
					<Button size="sm" onclick={() => void apply(choice)} disabled={busy}>
						{m['settings.runmode.apply']()}
					</Button>
				</div>
			</div>
		</details>

		{#if posture.posture === 'service'}
			<div class="mt-1">
				<Button variant="outline" size="sm" onclick={() => void uninstall()} disabled={busy}>
					{m['settings.runmode.uninstall']()}
				</Button>
			</div>
		{/if}

		{#if removedSummary !== ''}
			<p class="text-sm text-muted-foreground" data-testid="settings-runmode-removed">
				{removedSummary}
			</p>
		{/if}
		{#if offeredPaths.length > 0}
			<div class="grid gap-1" data-testid="settings-runmode-offered">
				{#each offeredPaths as path (path)}
					<p class="text-sm text-muted-foreground">
						{m['settings.runmode.offeredData']({ path })}
					</p>
				{/each}
				<div>
					<Button variant="outline" size="sm" onclick={() => void removeData()} disabled={busy}>
						{m['settings.runmode.removeData']()}
					</Button>
				</div>
			</div>
		{/if}

		{#if outcome !== null}
			<p
				class="text-sm"
				role="status"
				aria-live="polite"
				data-testid="settings-runmode-outcome"
			>
				{outcome.sentence}
			</p>
		{/if}
	</div>
{/if}
