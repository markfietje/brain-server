# Local judgment vs rented judgment: why the System-1 port is Laya, not Jev

*2026-09-22, v1.28.92. The 1.32.8 System-One lane is opener-gated: Phase 0 pure modules landed, inference and the classifier consume ship only with operator-labeled proof.*

Every governed loop eventually needs cheap judgment. Not the deep kind — the shallow, high-volume kind: which class does this ticket belong to, which language is this, is this message safe to show, which of these twenty options fits. An agent that pays frontier-model prices for those decisions is burning money; an agent that makes them with no calibration is burning trust. There are two ways to buy cheap judgment: rent it from a hosted endpoint, or run it locally. This post is the decision record for why our lane is local — and why the hosted alternative stays out of prod by design.

## What Jev is

Jev is the hosted path: send the text out, get a judgment back. The numbers cited for it (from the Laya author's demo videos — cited, not measured by us) are strong where it matters: around 73% zero-shot on business classification against 36% for the base local checkpoints, at 236–276ms per call. If your only metric is zero-shot accuracy per millisecond, rent wins.

But accuracy per millisecond is not our metric. Our loop already refuses to let memory leave the box on the retrieval path — no LLM in the retrieval loop, no embedding API, air-gapped profiles that must work with no network at all (operator-configured sinks like webhooks and OIDC fetch exist, pinned at the egress boundary — but recall itself never calls out). A hosted judgment call breaks every one of those properties at the exact moment the loop needs judgment most: on untrusted, possibly PII-bearing, possibly clinical case text. The cost is not just the per-call meter. It is the data-flow contradiction: a privacy posture with a hole in it everywhere triage happens. So the plan states it flatly: no Jev adapter, no outbound network in prod. Not "later" — never, as an architectural position. (`LAYA_RUST_PORT.md` §8 non-goals.)

## What Laya is

Laya is the local family: open-source (`NandhaKishorM/laya` 0.3.4, Apache-2.0), ModernBERT/mmBERT checkpoints, small enough to live on the operator's own machine — primary target MacBook Pro M1 Pro, Jetson stays on the static path. The Rust port lands in four phases, and Phase 0 is already in the tree:

- **Phase 0 (shipped, v1.28.92):** pure modules, no feature. `lang` (script detection, language guessing, depth-6 state flattener), `router` (closed precedence chain, checkpoint alias table, LRU state machine), `sequence` (the closed choice/score/noul vocabulary, budget arithmetic, the hard 20-option ceiling), `calibration` (entropy confidence, temp buckets, hand-computable ECE, integer score units), `presets` (triage/email/guard/moderation/router schemas as pure data). 134 tests, zero Cargo change, zero behavior change — reviewable without weights.
- **Phase 1:** export + `laya-local` skeleton. Weights arrive as pre-exported ONNX, tokenizer as `tokenizer.json`, no runtime HuggingFace fetch. Only two dependencies reused (`ort` + `tokenizers`), both already in the tree.
- **Phase 2:** boot + preload on M1, `/healthz` reports what's loaded.
- **Phase 3:** pilots + fit. Conservative 0.85 thresholds (escalate-heavy at first), ECE fitted on held-out slices, thresholds lowered per-domain only with human sign-off.

## The rules that make local judgment trustworthy

The port carries the same fail-closed DNA as the rest of the loop:

- **Closed vocabularies stay closed.** The model never free-texts a decision; it picks from choice/score/noul schemas, max 20 options, no bypass flag. Unknown model strings route to a human (`Routed`), never to a guess.
- **Floats stop at the boundary.** A compile-time scan (`no_f32_in_decide_math_outside_boundary`) keeps `f32` inside `calibration.rs`; everything downstream is integer units. Judgment you can't audit bit-for-bit isn't judgment you can sign off on.
- **Escalation is the default output.** Weak confidence, weak zero-shot, strict-scaling math — all escalate. The `score` primitive is quarantined until its own eval passes because the videos show it weakest on math scaling.
- **Auto-act needs proof, not vibes.** A fine-tuned checkpoint with a pinned SHA plus ECE evidence, or the path stays escalate-only. "Fast base to specialise, not magic judgment" is the ship message, stated in the plan verbatim.

## The honest ceilings

Local judgment as shipped today is weaker zero-shot than rented (36% vs 73% cited on business classification — the gap fine-tuning is supposed to close, and the 1.32.8 lane stamps only when the operator labeling round proves it closed). ModernBERT-large in ONNX may fall back from CoreML to CPU ops (ship CPU-only with a parity gate if it diverges). All three checkpoints resident exceeds M1 comfort with other tiers co-loaded (default max two). ONNX + tokenizer are binary blobs — SHA256SUMS plus pinned HF revision plus audit-green, or they don't load. And the classifier consume — the part that would actually let the loop act on local judgment — is deliberately absent, opener-gated on the labeling round.

That absence is the point of this post. We would rather ship the pure math with 134 tests and no callers than wire a judgment path we cannot yet prove calibrated. Rented judgment would have been faster to demo and impossible to defend: every escalation, every triage call, every red-flag check would cross the network boundary the rest of the system treats as sacred. Local judgment is weaker today, improvable by fine-tune, auditable to the bit, and air-gappable. For a memory system whose whole thesis is "your data never leaves," that is not a close call.
