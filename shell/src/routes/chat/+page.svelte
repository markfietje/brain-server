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
	import {
		verifyClaim,
		typedVerifyTransport,
		type AuthorizedVerify,
		type VerifyCore,
		type VerifyOutcome,
		type VerifyRequest
	} from '#lib/api/verify.js';
	import {
		forgetRecord,
		typedForgetTransport,
		type AuthorizedForget,
		type ForgetCore,
		type ForgetOutcome
	} from '#lib/api/forget.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';

	interface Turn {
		id: number;
		rendering: TurnRendering;
		provenance: ProvenanceBlock | null;
	}

	/**
	 * II-7g — the per-citation affordances: `[correct]` and `[forget]`.
	 *
	 * Both go through their own core in the SAME order the recall turn does
	 * (authorize → wire → shape), and neither decision is made in this
	 * component: the six `/verify` states and the five `/ump/forget` states are
	 * classified in Rust, and this file only draws what came back. That is the
	 * whole reason these are affordances rather than inline fetches — an inline
	 * `/verify` would put a content-confirmation oracle in the renderer with the
	 * core's bounds nowhere in it.
	 *
	 * State is keyed by CITATION id, so two citations in one turn cannot
	 * overwrite each other's verdicts.
	 */
	interface CitationAction {
		verify: 'idle' | 'busy' | VerifyOutcome;
		forget: 'idle' | 'busy' | ForgetOutcome;
	}

	let actions = $state<Record<number, CitationAction>>({});
	/** The open `[correct]` form, or null. One at a time, so the transcript
	 * cannot fill with expanded forms nobody asked for. */
	let verifying = $state<number | null>(null);
	let claimDrafts = $state<Record<number, string>>({});

	function actionFor(citationId: number): CitationAction {
		return actions[citationId] ?? { verify: 'idle', forget: 'idle' };
	}

	const verifyCore: VerifyCore = {
		authorize: async (request: VerifyRequest): Promise<AuthorizedVerify> => {
			const response = await import('@tauri-apps/api/core').catch(() => null);
			if (response && typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
				return (await response.invoke('verify_claim_command', { request })) as AuthorizedVerify;
			}
			throw new Error('no Tauri runtime: the core is only reachable inside the shell');
		},
		shape: async (
			exchange: { status: number; body: string },
			doc: AuthorizedVerify['doc']
		): Promise<unknown> => {
			const response = await import('@tauri-apps/api/core').catch(() => null);
			if (response && typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
				return response.invoke('shape_verify_command', { exchange, requested: doc });
			}
			throw new Error('no Tauri runtime: the core is only reachable inside the shell');
		}
	};

	const forgetCore: ForgetCore = {
		// `unknown`, not a request type: the CORE takes `hard` in order to refuse
		// it, so a narrower type here would hide the field this affordance must
		// be unable to express. `forgetRecord` is given the id and nothing else.
		authorize: async (request: unknown): Promise<AuthorizedForget> => {
			const response = await import('@tauri-apps/api/core').catch(() => null);
			if (response && typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
				return (await response.invoke('forget_memory', { request })) as AuthorizedForget;
			}
			throw new Error('no Tauri runtime: the core is only reachable inside the shell');
		},
		shape: async (exchange: { status: number; body: string }, doc: { id: string }): Promise<unknown> => {
			const response = await import('@tauri-apps/api/core').catch(() => null);
			if (response && typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
				return response.invoke('shape_forget_command', { exchange, requested: doc });
			}
			throw new Error('no Tauri runtime: the core is only reachable inside the shell');
		}
	};

	function setAction(citationId: number, patch: Partial<CitationAction>): void {
		actions = { ...actions, [citationId]: { ...actionFor(citationId), ...patch } };
	}

	function openVerify(citationId: number, prefill: string): void {
		verifying = citationId;
		claimDrafts = { ...claimDrafts, [citationId]: prefill };
	}

	async function runVerify(citationId: number): Promise<void> {
		const claim = (claimDrafts[citationId] ?? '').trim();
		// An empty claim is the CORE's refusal (`claim_empty`), not a guard
		// here: duplicating the bound in the renderer is how two copies drift.
		// The button is simply not enabled for an empty draft, which is an
		// affordance, not the bound.
		if (claim.length === 0) return;
		setAction(citationId, { verify: 'busy' });
		try {
			const outcome = await verifyClaim(
				{ chunk_id: citationId, claim },
				{ core: verifyCore, transport: typedVerifyTransport }
			);
			setAction(citationId, { verify: outcome });
		} catch (error) {
			// A core that cannot be reached is DEGRADED and STATED. It becomes
			// `unreachable` rather than a silent "not found" — a surface that
			// reported "not found in this source" when it never asked would be the
			// exact conflation the six states exist to prevent.
			setAction(citationId, {
				verify: {
					state: 'unreachable',
					status: 0,
					detail: error instanceof Error ? error.message : String(error)
				}
			});
		}
	}

	async function runForget(citationId: number): Promise<void> {
		setAction(citationId, { forget: 'busy' });
		try {
			// `{id}` and nothing else. There is no `hard` key at this call site,
			// which is the affordance's half of the core's boundary.
			const outcome = await forgetRecord(
				{ id: String(citationId) },
				{ core: forgetCore, transport: typedForgetTransport }
			);
			setAction(citationId, { forget: outcome });
		} catch (error) {
			setAction(citationId, {
				forget: {
					state: 'unavailable',
					status: 0,
					detail: error instanceof Error ? error.message : String(error)
				}
			});
		}
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
								{@const action = actionFor(citation.id)}
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

									<!-- II-7g — the per-citation affordances.
									     Every state carries its own `data-*` token, because the
									     copy is five locales deep and a translated string is not
									     a stable hook. Six verify arms and five forget arms are
									     each DISTINCT tokens: a renderer that collapsed a refusal
									     into an unsupported verdict would still produce one of
									     these tokens, and the pins compare the token SET. -->
									<div class="flex flex-wrap items-center gap-2 pt-1">
										<Button
											type="button"
											variant="ghost"
											size="sm"
											data-testid="chat-correct"
											onclick={() =>
												openVerify(
													citation.id,
													citation.title.kind === 'present' ? citation.title.value : citation.text
												)}
										>
											{m['chat.correct.open']()}
											<span class="sr-only"> — {m['chat.correct.claim.label']()}</span>
										</Button>
										<Button
											type="button"
											variant="ghost"
											size="sm"
											data-testid="chat-forget"
											onclick={() => runForget(citation.id)}
											disabled={action.forget === 'busy'}
										>
											{m['chat.forget.open']()}
											<span class="sr-only"> — {m['chat.forget.submit']()}</span>
										</Button>
									</div>

									{#if verifying === citation.id}
										<!-- The claim surface. A real `<form>` with a real
										     `<label>`, so the flow is keyboard-complete and the
										     input is announced; Enter submits without a click. -->
										<form
											class="grid gap-2 pt-1"
											onsubmit={(event) => {
												event.preventDefault();
												void runVerify(citation.id);
											}}
										>
											<label class="sr-only" for="chat-claim-{citation.id}">
												{m['chat.correct.claim.label']()}
											</label>
											<Input
												id="chat-claim-{citation.id}"
												bind:value={claimDrafts[citation.id]}
												placeholder={m['chat.correct.claim.placeholder']()}
												data-testid="chat-claim"
											/>
											<div>
												<Button
													type="submit"
													size="sm"
													data-testid="chat-verify-submit"
													disabled={(claimDrafts[citation.id] ?? '').trim().length === 0 ||
														action.verify === 'busy'}
												>
													{m['chat.correct.submit']()}
												</Button>
											</div>
										</form>
									{/if}

									{#if action.verify === 'busy'}
										<p class="text-xs text-muted-foreground" data-verify-state="busy">
											{m['chat.working']()}
										</p>
									{:else if action.verify !== 'idle'}
										<!-- `role="status"` so a verdict announces without
										     interrupting. Each arm is its own token AND its own
										     copy: `refused` and `no_such_chunk` are deliberately
										     different sentences, because a refusal must never read
										     as "the claim is not in that source". -->
										<p
											role="status"
											class="text-xs"
											data-testid="chat-verify-result"
											data-verify-state={action.verify.state}
										>
											{#if action.verify.state === 'supported'}
												{m['chat.correct.supported']()}
											{:else if action.verify.state === 'unsupported'}
												{m['chat.correct.unsupported']()}
											{:else if action.verify.state === 'refused'}
												{m['chat.correct.refused']()}
											{:else if action.verify.state === 'no_such_chunk'}
												{m['chat.correct.no_such_chunk']()}
											{:else if action.verify.state === 'unreachable'}
												{m['chat.correct.unreachable']()}
											{:else}
												{m['chat.correct.violation']()}
											{/if}
										</p>
									{/if}

									{#if action.forget !== 'idle' && action.forget !== 'busy'}
										<p
											role="status"
											class="text-xs"
											data-testid="chat-forget-result"
											data-forget-state={action.forget.state}
										>
											{#if action.forget.state === 'tombstoned'}
												{m['chat.forget.tombstoned']()}
											{:else if action.forget.state === 'refused'}
												{m['chat.forget.refused']()}
											{:else if action.forget.state === 'no_such_row'}
												{m['chat.forget.no_such_row']()}
											{:else if action.forget.state === 'unavailable'}
												{m['chat.forget.unavailable']()}
											{:else}
												{m['chat.forget.violation']()}
											{/if}
										</p>
										<!-- The human gate, stated on the affordance itself.
										     II-6's receipt carries `humanGated: true` as a
										     VALUE; this is the same fact where the action
										     begins. There is no approve path to disable,
										     because there is none. -->
										<p class="text-xs text-muted-foreground" data-testid="chat-forget-gate">
											{m['chat.forget.human_gate']()}
										</p>
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