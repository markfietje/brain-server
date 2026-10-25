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
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import {
		recallMemory,
		typedRecallTransport,
		type AuthorizedRecall,
		type RecallCore,
		type RecallRequest
	} from '#lib/api/recall.js';
	import { renderTurn, renderingToken, type TurnRendering } from '#lib/chat/turn.js';
	import { provenanceBlock, type ProvenanceBlock } from '#lib/chat/provenance.js';
	import { wireFaultMessage } from '#lib/api/wire-fault.js';
	import type { WireFault } from '#lib/api/client.js';
	import {
		classifyWireFault,
		clearWireFault,
		currentWireFault
	} from '#lib/chat/wire-status.svelte.js';
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
	import ArrowUpIcon from '@lucide/svelte/icons/arrow-up';
	import { currentConversationEpoch } from '#lib/shell/frame.svelte.js';

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

	// "New chat" in the sidebar bumps the conversation epoch; this surface
	// answers by dropping its transcript and every per-citation state with
	// it, then handing focus back to the composer. The first run only
	// records the epoch it mounted under — a mount is not a reset.
	let seenEpoch = currentConversationEpoch();
	$effect(() => {
		const epoch = currentConversationEpoch();
		if (epoch === seenEpoch) return;
		seenEpoch = epoch;
		turns = [];
		actions = {};
		verifying = null;
		claimDrafts = {};
		evidenceOpen = {};
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
			// M3-b: an `unreachable` that CAME BACK from the pipeline is the
			// transport arm — the wire itself failed while the core answered —
			// so it is the one classified state that buys the banner's
			// sentence. A refusal is the kernel SPEAKING and never a fault;
			// the catch arm below is the app's own core failing, which a
			// "restart the kernel" banner would misname.
			if (rendering.kind === 'unreachable') void classifyWireFaultByProbe();
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

	// ── M3-b: the fault banner (exception chrome, plan 21 §1) ─────────────
	//
	// The renderer cannot name a wire failure — the browser engine reports a
	// blocked response, a rejected bearer and a dead port identically — so
	// classification asks the core ONCE, at the fault moment (`diagnoseWire`'s
	// vocabulary, kept local because client.ts's copy is not exported; the
	// list is the six `WireFault` literals and drift is impossible without
	// the type changing too). One bounded probe per fault event: no polling,
	// no interval, no watchdog (plan 22 R2).
	const NAMED_FAULTS: readonly WireFault[] = [
		'unreachable',
		'origin_not_allowed',
		'no_credential',
		'unauthorized',
		'unexpected'
	];

	async function classifyWireFaultByProbe(): Promise<void> {
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			const dto = await invoke<{ kind: string }>('kernel_wire_status');
			const named = dto?.kind as WireFault;
			classifyWireFault(NAMED_FAULTS.includes(named) ? named : 'unknown');
		} catch {
			// No core to ask (the PWA posture): the honest answer is `unknown`,
			// whose sentence names the situation without inventing a cause.
			classifyWireFault('unknown');
		}
	}

	const bannerFault = $derived(currentWireFault());

	function inTauri(): boolean {
		return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
	}

	let restarting = $state(false);

	/**
	 * Restart re-runs the EXISTING gate flow — `kernel_status`, then
	 * `kernel_start` from `notRunning` only, then the gate's own health read.
	 * The consent marker and the data root already exist from the first run,
	 * so there is no second consent dialog; a probe that answers `ready` is
	 * the one thing that dismisses the banner, and any other outcome leaves
	 * the fault on the screen because it is still the live fact.
	 */
	async function restartKernel(): Promise<void> {
		if (restarting) return;
		restarting = true;
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			const status = await invoke<{ state: string }>('kernel_status');
			if (status.state === 'notRunning') {
				await invoke('kernel_start', { consent: true });
			}
			const probe = await invoke<{ state: string }>('kernel_status');
			if (probe.state === 'ready') clearWireFault();
		} catch {
			// The banner stays: a restart that could not even ask the core has
			// not changed the fact the banner names.
		} finally {
			restarting = false;
		}
	}

	// ── M3-b: the collapsed evidence disclosure (plan 21 §3) ──────────────
	//
	// Collapsed is the DEFAULT (absent key = closed); the affordances render
	// INSIDE the opened disclosure, and `receipt` is untouched — it reads the
	// provenance block, never the DOM's expansion state.
	let evidenceOpen = $state<Record<number, boolean>>({});

	function evidenceIsOpen(turnId: number): boolean {
		return evidenceOpen[turnId] ?? false;
	}

	function toggleEvidence(turnId: number): void {
		evidenceOpen = { ...evidenceOpen, [turnId]: !evidenceIsOpen(turnId) };
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

<div class="flex min-h-full flex-1 flex-col gap-4" data-testid="chat-surface">
	<h1 class="sr-only">{m['chat.title']()}</h1>

	<!-- M3-b — the empty state IS the first paint (plan 21 §1/§3.8): a
	     one-line greeting and three chips that ACT. The chips are the
	     onboarding — each does the thing it names, none explains a page —
	     and the first turn replaces them, because a transcript with chips
	     below it is clutter. The composer keeps its mount focus: the chips
	     render beside it, they never take focus for themselves. It sits in
	     the column's middle, the way an empty conversation does, with the
	     composer waiting below. -->
	{#if turns.length === 0}
		<div class="my-auto grid gap-4 py-10 text-center" data-testid="chat-empty">
			<p class="text-xl font-semibold tracking-tight">{m['chat.empty.greeting']()}</p>
			<div class="flex flex-wrap justify-center gap-2">
				<Button
					type="button"
					variant="outline"
					size="sm"
					data-testid="chat-empty-ask"
					onclick={() => {
						// The composer's id — the same identity its label binds
						// to — not `composerEl`: the component-bound ref is the
						// mount effect's channel and is not populated under
						// jsdom, and a chip whose act only works where the ref
						// binds would be unpinnable where pins run.
						document.getElementById('chat-composer')?.focus();
					}}
				>
					{m['chat.empty.chip.ask']()}
				</Button>
				<Button
					type="button"
					variant="outline"
					size="sm"
					data-testid="chat-empty-trace"
					onclick={() => goto(resolve('/recall'))}
				>
					{m['chat.empty.chip.trace']()}
				</Button>
				<Button
					type="button"
					variant="outline"
					size="sm"
					data-testid="chat-empty-review"
					onclick={() => goto(resolve('/review'))}
				>
					{m['chat.empty.chip.review']()}
				</Button>
			</div>
		</div>
	{/if}

	<!-- The transcript. `aria-live="polite"` so a streaming or slow turn
	     announces without interrupting (Plan 02 §5). Each turn is the
	     person's line as a bubble on the right and the answer as prose on
	     the left; a refusal and an abstention keep a bordered card, because
	     those two are the states a reader must not glide past. -->
	<div
		class="flex flex-col gap-6"
		role="log"
		aria-live="polite"
		aria-label={m['chat.transcript']()}
		data-testid="chat-transcript"
	>
		{#each turns as turn (turn.id)}
			<article
				data-rendering={renderingToken(turn.rendering)}
				data-testid="chat-turn"
				class="flex flex-col gap-3"
			>
				<p
					class="max-w-[85%] self-end rounded-2xl rounded-br-md bg-surface-2 px-4 py-2.5 text-sm whitespace-pre-wrap"
				>
					{turn.rendering.query}
				</p>

				<div
					class="grid max-w-[92%] gap-2 self-start
						{turn.rendering.kind === 'refused'
						? 'rounded-xl border border-destructive/60 p-4'
						: turn.rendering.kind === 'abstained'
							? 'rounded-xl border border-brand/50 p-4'
							: ''}"
				>
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
						<!-- M3-b — the verdict line is the answer; the citations
						     are EVIDENCE, collapsed by default and opened on
						     demand (plan 21 §3). A real disclosure control: in
						     the tab order, operable by keyboard, carrying
						     aria-expanded and aria-controls. The affordances
						     render only inside the OPENED list, so the II-7g
						     pins keep driving the opened state they always
						     drove. -->
						<Button
							type="button"
							variant="ghost"
							size="sm"
							class="w-fit"
							data-testid="chat-evidence"
							aria-expanded={evidenceIsOpen(turn.id)}
							aria-controls="chat-evidence-list-{turn.id}"
							onclick={() => toggleEvidence(turn.id)}
						>
							{#if evidenceIsOpen(turn.id)}
								{m['chat.evidence.collapse']()}
							{:else}
								{m['chat.evidence.count']({ count: turn.provenance.count })}
								<span class="sr-only"> — {m['chat.evidence.expand']()}</span>
							{/if}
						</Button>
						{#if evidenceIsOpen(turn.id)}
							<ul class="grid gap-3" id="chat-evidence-list-{turn.id}">
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
				</div>
			</article>
		{/each}
	</div>

	<!-- M3-b — the fault banner: the ONE interrupt plan 21 §1 allows. It
	     renders only when a wire fault was CLASSIFIED (silent while green is
	     structural — no node, not a hidden one), it names the fault's own
	     sentence via `wireFaultMessage` (never a generic "something went
	     wrong"), and its Restart re-runs the existing gate flow. A refusal
	     never reaches here: it is the kernel speaking, not the wire failing. -->
	{#if bannerFault !== null}
		<div
			role="alert"
			data-testid="chat-fault-banner"
			data-fault={bannerFault}
			class="grid gap-2 rounded-lg border border-destructive/60 p-3"
		>
			<p class="text-sm">{wireFaultMessage(bannerFault)}</p>
			{#if inTauri()}
				<Button
					type="button"
					size="sm"
					class="w-fit"
					data-testid="chat-fault-restart"
					disabled={restarting}
					onclick={() => void restartKernel()}
				>
					{restarting ? m['chat.status.restarting']() : m['chat.status.restart']()}
				</Button>
			{/if}
		</div>
	{/if}

	<!-- The composer. Focused on mount, so the app opens ready to type. It
	     sticks to the bottom of the column while the transcript scrolls
	     above it — the one control that is always within reach. -->
	<form
		class="sticky bottom-0 mt-auto grid gap-2 bg-background pt-3"
		onsubmit={(event) => {
			event.preventDefault();
			void send();
		}}
	>
		<label class="sr-only" for="chat-composer">{m['chat.composer.label']()}</label>
		<div
			class="flex items-end gap-2 rounded-2xl border border-border bg-surface p-2 shadow-xs focus-within:border-ring"
		>
			<Textarea
				id="chat-composer"
				ref={composerEl}
				bind:value={composer}
				onkeydown={onKeydown}
				placeholder={m['chat.composer.placeholder']()}
				rows={2}
				class="min-h-0 resize-none border-0 bg-transparent shadow-none focus-visible:ring-0"
				data-testid="chat-composer"
			/>
			<Button
				type="submit"
				size="icon-sm"
				class="shrink-0 rounded-full"
				disabled={busy || composer.trim().length === 0}
				aria-label={m['chat.send']()}
				data-testid="chat-send"
			>
				<ArrowUpIcon aria-hidden="true" />
			</Button>
		</div>
		<p class="px-2 text-xs text-muted-foreground">{m['chat.composer.hint']()}</p>
	</form>
</div>