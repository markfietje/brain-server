<script lang="ts">
	// ADVANCED — the operator affordances folded (the operator's UX call,
	// 2026-10-30: "the items we don't use need to be folded"). The kernel
	// token needs NO user action — the app bootstraps the credential from
	// the keychain automatically (R130); the section exists for manual
	// ROTATION (R129) and the loopback override, which are operator/posture
	// acts, not onboarding steps. Native <details>/<summary>: collapsed by
	// default, keyboard-accessible, zero new dependencies, and the summary
	// names what is folded so nothing is hidden SILENTLY.
	import OriginSection from './origin-section.svelte';
	import TokenSection from './token-section.svelte';
</script>

<div class="grid gap-2">
	<details class="group rounded-lg border border-border px-3 py-2">
		<summary
			class="cursor-pointer list-none text-sm font-semibold text-muted-foreground select-none [&::-webkit-details-marker]:hidden"
		>
			<span class="inline-flex items-center gap-1">
				<span
					class="transition-transform group-open:rotate-90 text-xs"
					aria-hidden="true">▶</span
				>
				Advanced
			</span>
		</summary>
		<p class="mt-1 text-xs text-muted-foreground">
			Kernel overrides and credential rotation — configured automatically;
			only touch these when you know why.
		</p>
		<div class="mt-4 grid gap-5">
			<OriginSection />
			<TokenSection />
		</div>
	</details>
</div>
