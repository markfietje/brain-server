<script lang="ts">
	// II-3 — the chat surface.
	//
	// Plan 02 §2's layout: the conversation is the page and every other
	// surface is a drawer over it. This route is deliberately thin — it holds
	// composer focus, a transcript, and a per-turn rendering token — because
	// every DECISION about what a turn means lives in `lib/chat/turn.ts` and
	// every fact about what to show lives in `lib/chat/provenance.ts`.
	//
	// WHY THE MODULE SPLIT, stated because the temptation is to inline it: the
	// highest-risk step in this phase is a renderer collapsing abstention into
	// absence. A rule that lives in a component is a rule a refactor can
	// delete silently; a rule that lives in a pure module is a rule the pins
	// can drive directly, and one the compiler narrows.
	//
	// NO `{@html}` anywhere, per `eslint.config.js` and Plan 04 §3: every
	// interpolated value below is a text node.
	import { m } from '$paraglide/messages.js';
	import {
		recallMemory,
		typedRecallTransport,
		type AuthorizedRecall,
		type RecallCore,
		type RecallRequest
	} from '#lib/api/recall.js';
	import { renderTurn, renderingToken, type TurnRendering } from '#lib/chat/turn.js';
	import { provenanceBlock, type ProvenanceBlock } from '#lib/chat/provenance.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';

	interface Turn {
		id: number;
		rendering: TurnRendering;
		provenance: ProvenanceBlock | null;
	}

	let composer = $state('');
	let busy = $state(false);
	let turns = $state<Turn[]>([]);
	let nextId = 0;

	// The composer takes focus on mount with NO navigation first — Plan 02 §7's
	// "Opening the app focuses the composer; zero navigation before typing".
	//
	// This is a `$effect` INSIDE the component's initialisation, which is what
	// Svelte 5 requires; the first draft put it at the top level of the module
	// script and every render failed with `effect_orphan`. Focus is applied
	// imperatively rather than via an `autofocus` attribute because the
	// attribute only wins when the element is inserted before the document has
	// focus, which a Svelte mount does not guarantee.
	// `ref`, not `bind:this`: the shadcn Textarea takes its element through a
	// bindable `ref` prop (`ref = $bindable(null)`), and `bind:this` on a
	// COMPONENT is not a thing in Svelte 5. The first draft used `bind:this`,
	// which compiled and then failed every render with `effect_orphan`.
	let composerEl = $state<HTMLTextAreaElement | null>(null);
	$effect(() => {
		composerEl?.focus();
	});

	// The core is injected so the surface is pinnable without a Tauri runtime.
	// Production passes the real core; the pin passes a stub that answers with
	// each of the six states in turn.
	//
	// Typed as `RecallCore` rather than left to inference: the IPC boundary
	// returns `unknown`, and `shape` MUST return `unknown` because
	// `assertOutcome` is what validates it. Declaring the return type is what
	// keeps the compiler honest here instead of letting an `any` leak in and
	// quietly turn the core's six-state classification into `any` downstream.
	const core: RecallCore = {
		authorize: async (request: RecallRequest): Promise<AuthorizedRecall> => {
			const response = await import('@tauri-apps/api/core').catch(() => null);
			if (response && typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
				return (await response.invoke('recall_memory', { request })) as AuthorizedRecall;
			}
			throw new Error('no Tauri runtime: the core is only reachable inside the shell');
		},
		shape: async (
			exchange: { status: number; body: string },
			doc: AuthorizedRecall['doc']
		): Promise<unknown> => {
			const response = await import('@tauri-apps/api/core').catch(() => null);
			if (response && typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
				return response.invoke('shape_recall_command', { exchange, requested: doc });
			}
			throw new Error('no Tauri runtime: the core is only reachable inside the shell');
		}
	};

	async function send(): Promise<void> {
		const query = composer.trim();
		if (query.length === 0 || busy) return;
		busy = true;
		composer = '';
		const id = nextId++;
		// The PENDING arm is rendered rather than showing nothing, so a turn is
		// never an empty region: Plan 02 §4's first-paint rule is that the
		// surface is never blank while it works.
		turns = [...turns, { id, rendering: { kind: 'pending', query }, provenance: null }];
		try {
			const outcome = await recallMemory({ query }, { core, transport: typedRecallTransport });
			const rendering = renderTurn(outcome, query);
			// Provenance is built ONLY for the answered arm. Every other state
			// carries no passages, so a provenance block beside them would be
			// fabricated evidence -- the opposite failure of omitting one.
			const provenance =
				rendering.kind === 'answered' ? provenanceBlock(rendering.passages, rendering.confidence, Date.now()) : null;
			turns = turns.map((turn) => (turn.id === id ? { ...turn, rendering, provenance } : turn));
		} catch (error) {
			// A core that cannot be reached is DEGRADED and STATED, never an
			// empty transcript (Plan 04 §5).
			turns = turns.map((turn) =>
				turn.id === id
					? {
							...turn,
							rendering: {
								kind: 'unreachable' as const,
								query,
								status: 0,
								detail: error instanceof Error ? error.message : String(error)
							},
							provenance: null
						}
					: turn
			);
		} finally {
			busy = false;
			composerEl?.focus();
		}
	}

	function onKeydown(event: KeyboardEvent): void {
		// ⌘⏎ sends; a bare Enter inserts a newline. Plan 02 §3's gesture set, and
		// the one a user has to be able to discover — so it is on the element's
		// own hint text rather than only in a menu.
		if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
			event.preventDefault();
			void send();
		}
	}
</script>

<div class="mx-auto grid w-full max-w-2xl gap-4" data-testid="chat-surface">
	<h1 class="sr-only">{m['chat.title']()}</h1>

	<!-- The transcript. `aria-live="polite"` so a streaming or slow turn
	     announces without interrupting (Plan 02 §5). -->
	<div
		class="grid gap-4"
		role="log"
		aria-live="polite"
		aria-label={m['chat.transcript']()}
		data-testid="chat-transcript"
	>
		{#each turns as turn (turn.id)}
			<article
				data-rendering={renderingToken(turn.rendering)}
				data-testid="chat-turn"
				class="grid gap-2 rounded-lg border p-4
					{turn.rendering.kind === 'refused'
					? 'border-destructive/60'
					: turn.rendering.kind === 'abstained'
						? 'border-brand/50'
						: 'border-border'}"
			>
				<p class="text-sm text-muted-foreground">{turn.rendering.query}</p>

				{#if turn.rendering.kind === 'pending'}
					<p aria-busy="true" class="text-sm">{m['chat.working']()}</p>
				{:else if turn.rendering.kind === 'answered'}
					{#if turn.provenance}
						<p class="text-sm font-semibold">
							{m['chat.recalled']({ count: turn.provenance.count })}
							{#if turn.provenance.confidence.kind === 'word'}
								· {m['chat.confidence']({ word: turn.provenance.confidence.word })}
							{:else}
								· {m['chat.confidence.unknown']()}
							{/if}
						</p>
						<ul class="grid gap-3">
							{#each turn.provenance.citations as citation (citation.id)}
								<li class="grid gap-1 text-sm" data-testid="chat-citation">
									<span class="font-medium">
										{citation.title.kind === 'present' ? citation.title.value : m['chat.untitled']()}
									</span>
									<span class="text-muted-foreground">{citation.text}</span>
									<!-- The validity line carries its own state token, so a
									     historical fact is distinguishable in the DOM and not
									     only in its wording. -->
									<span
										data-historical={citation.validity.historical}
										class:text-brand={citation.validity.historical}
										class="text-xs text-muted-foreground"
									>
										{citation.validity.summary}
									</span>
									{#if citation.flagged}
										<span class="text-xs text-destructive">{m['chat.quarantined']()}</span>
									{/if}
								</li>
							{/each}
						</ul>
					{/if}
				{:else if turn.rendering.kind === 'absent'}
					<!-- An ABSENCE: the search ran and the store holds nothing. -->
					<p data-testid="chat-absent" class="text-sm">
						{m['chat.absent.body']()}
						<span class="text-muted-foreground">{m['chat.searched']({ query: turn.rendering.searched })}</span>
					</p>
				{:else if turn.rendering.kind === 'abstained'}
					<!-- An ABSTENTION: an ANSWER, not an error. Plan 06 §3 calls
					     this the product's sharpest behaviour, so it is styled as
					     a result and offers the next action. -->
					<div data-testid="chat-abstained" class="grid gap-1 text-sm">
						<p class="font-semibold">{m['chat.abstained.title']()}</p>
						<p class="text-muted-foreground">{m['chat.searched']({ query: turn.rendering.searched })}</p>
						<p class="text-xs text-muted-foreground">{m['chat.abstained.next']()}</p>
					</div>
				{:else if turn.rendering.kind === 'refused'}
					<!-- A REFUSAL: named as one, with the status, and never phrased
					     as an absence. It does not say how many rows exist. -->
					<p data-testid="chat-refused" role="status" class="text-sm">
						{m['chat.refused.body']({ status: turn.rendering.status })}
					</p>
				{:else if turn.rendering.kind === 'unreachable'}
					<p data-testid="chat-unreachable" role="status" class="text-sm">
						{m['chat.unreachable.body']({ detail: turn.rendering.detail })}
					</p>
				{:else if turn.rendering.kind === 'contract_violation'}
					<p data-testid="chat-violation" role="alert" class="text-sm">
						{m['chat.violation.body']({ detail: turn.rendering.detail })}
					</p>
				{/if}
			</article>
		{/each}
	</div>

	<!-- The composer. Focused on mount, so the app opens ready to type. -->
	<form
		class="grid gap-2"
		onsubmit={(event) => {
			event.preventDefault();
			void send();
		}}
	>
		<label class="sr-only" for="chat-composer">{m['chat.composer.label']()}</label>
		<Textarea
			id="chat-composer"
			ref={composerEl}
			bind:value={composer}
			onkeydown={onKeydown}
			placeholder={m['chat.composer.placeholder']()}
			rows={2}
			data-testid="chat-composer"
		/>
		<div class="flex items-center justify-between gap-2">
			<p class="text-xs text-muted-foreground">{m['chat.composer.hint']()}</p>
			<Button type="submit" disabled={busy || composer.trim().length === 0} data-testid="chat-send">
				{m['chat.send']()}
			</Button>
		</div>
	</form>
</div>