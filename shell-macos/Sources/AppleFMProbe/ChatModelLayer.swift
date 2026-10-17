import Foundation

#if canImport(FoundationModels)
import FoundationModels
#endif

// MARK: - Model choice

/// The model id the chat layer is allowed to run on.
///
/// This is a STRING, and that is the point. A model that is merely "not wired
/// up" re-enters the next time someone adds a picker, and the failure is silent
/// — it simply starts throwing on turn 2, which looks like a memory bug and is
/// chased as one. So the exclusion is a value, a check, and a pin, rather than a
/// convention.
///
/// ## THE ONE PLACE QWEN3.5'S CAUSE IS STATED.
///
/// Everything else in the repo refers here rather than restating it.
///
/// **Measured.** Qwen3.5-9B-4bit throws `runtime("No user query found in
/// messages.")` on turn 2 of any conversation, on BOTH `LanguageModelSession`
/// and the mlx-native `ChatSession`. Qwen3-0.6B-4bit passes the same
/// multi-turn probe with a cold control.
///
/// **Hypothesis, and previously overstated.** An earlier version of this comment
/// asserted the defect "is in the MLX executor's prefix matching, not the
/// weights", reasoning that the identical failure across two session types
/// rules out a call-shape problem. The symmetry is real; the conclusion drawn
/// from it was not licensed. mlx-native `ChatSession` does not go through a
/// `LanguageModelExecutor` at all, so a defect in that executor cannot explain a
/// throw on BOTH surfaces — the common factor is the chat-template path they
/// share, which is also where upstream's own note about conversation-aware Qwen
/// templates points.
///
/// So: the weights are not implicated, and the cause is **not established**.
/// It is a rendering/template-layer question, and whether it is fixable locally
/// is open. What is NOT open is that the model cannot serve chat today, which
/// is what this enum actually enforces.
public enum ChatModelId: String, Sendable, Equatable, CaseIterable {
    /// The measured tier. 4,096 combined tokens, and the only one that has
    /// completed a turn on the development machine.
    case onDeviceAFM = "afm3-core"

    /// Excluded, with the reason attached to the case rather than to a comment
    /// that a refactor can leave behind.
    ///
    /// Retained as a case — not deleted — because `allCases` is what lets a pin
    /// enumerate the whole set and fail if an id that is not allowed ever
    /// appears. Deleting it would make the exclusion unenforceable and leave
    /// only the memory of it.
    case qwen3_5 = "qwen3.5-9b-4bit"

    /// Whether this model may serve a CHAT path — a multi-turn conversation
    /// with history carried across turns.
    public var mayServeChat: Bool {
        switch self {
        case .onDeviceAFM: true
        case .qwen3_5: false
        }
    }

    /// Why not, for the error a caller gets.
    public var chatRefusalReason: String? {
        switch self {
        case .onDeviceAFM: nil
        case .qwen3_5:
            "Qwen3.5 throws \"No user query found in messages.\" on turn 2 of any "
                + "conversation, on both LanguageModelSession and mlx-native ChatSession. The "
                + "weights are not implicated and the cause is not established — see ChatModelId "
                + "for the evidence and the two hypotheses."
        }
    }

    /// Refuses a model that may not serve chat.
    ///
    /// Returns the model rather than throwing, so a caller that wants to probe
    /// can; the refusal is carried in the enum's own vocabulary rather than in
    /// an error type that would need its own case for every future exclusion.
    public static func forChat(_ id: ChatModelId) throws -> ChatModelId {
        guard id.mayServeChat else {
            throw ChatModelError.excludedFromChat(id: id, reason: id.chatRefusalReason ?? "")
        }
        return id
    }
}

/// A model that cannot serve a chat path. Small on purpose: one case, and it
/// names the id so the message says which model was refused.
public enum ChatModelError: Error, Equatable {
    case excludedFromChat(id: ChatModelId, reason: String)
}

// MARK: - The window, read through ONE path

/// One seam for BOTH tiers' windows, so no caller can read one synchronously.
///
/// TRAP 1, and the reason this type exists. `SystemLanguageModel.contextSize`
/// is a plain `var`; `PrivateCloudComputeLanguageModel.contextSize` is
/// `get async throws`, because it describes a server-side deployment rather than
/// the local binary. Code that reads the window from a call site that has
/// concrete knowledge of the tier compiles against one model and fails against
/// the other — and a Dynamic Profile that swaps between them is exactly where
/// that surfaces.
///
/// ACCURATELY PUT, because an earlier version of this comment said "one async
/// read for both tiers" and that is wrong: `onDevice()` is SYNCHRONOUS. The
/// seam unifies the RETURN TYPE, not the calling convention, and the two halves
/// legitimately differ because the SDK says they do. What the seam buys is that
/// a caller writes one shape and gets either tier back, instead of hard-coding
/// a tier and breaking on the other.
///
/// The remote read is `async` because the TYPE demands it, not because it costs
/// anything: M1 measured the PCC window read at **0.0 ms**, served from a local
/// cache. It is a compile-time asymmetry wearing a runtime signature.
///
/// The availability is split the way the trap is: the on-device half is 26.0
/// (a plain local property, and the tier that actually runs here) while the
/// private-cloud half needs 27.0. A seam that could only be called at 27 would
/// not be usable by the 26 code it exists to protect.
@available(macOS 26.0, *)
public enum ChatWindow {
    /// The combined input+output window for a tier, or the reason there is none.
    public enum Reading: Sendable, Equatable {
        case window(Int)
        case unreadable(reason: String)

        public var tokens: Int? {
            if case let .window(n) = self { return n }
            return nil
        }
    }

    /// The on-device tier. A plain property, wrapped so callers cannot reach
    /// past this type and read it synchronously.
    public static func onDevice() -> Reading {
        let model = SystemLanguageModel.default
        guard case .available = model.availability else {
            return .unreadable(reason: "on-device model unavailable")
        }
        return .window(model.contextSize)
    }

    /// The Private Cloud tier. The SAME return type as `onDevice()`, though the
    /// property underneath is `async throws`.
    ///
    /// A refusal here is expected on an unentitled machine and is NOT an
    /// error to swallow: M1 measured it — the window reads back 32,768 while
    /// every generation request is refused with `ModelManagerError 1046`. So a
    /// readable window is emphatically not evidence that the tier works, and
    /// the routing policy is written on that basis.
    @available(macOS 27.0, *)
    public static func privateCloud() async -> Reading {
        let pcc = PrivateCloudComputeLanguageModel()
        do {
            return .window(try await pcc.contextSize)
        } catch {
            return .unreadable(reason: String(describing: error))
        }
    }
}

// MARK: - Routing policy

/// Why a turn would escalate, in the priority order plan 19 §3.1 sets.
///
/// Plan 19 lists a FIRST trigger above this — "reasoning the small model is
/// measured to get wrong" — and it is deliberately ABSENT rather than carried as
/// an unimplemented case. It was an enum case for a while and it was removed: no
/// code produced it, and a case nothing constructs is a promise with no
/// implementation behind it. The measurement it depends on has not been taken,
/// so the trigger does not exist yet.
public enum EscalationReason: String, Sendable, Equatable, CaseIterable {
    /// Trigger 2 — what the turn asks for exceeds the remaining on-device
    /// budget. The only trigger currently decidable from a measurement, because
    /// the budget IS measured (`ContextBudget`).
    ///
    /// ## RENAMED, because the old name was the bug
    ///
    /// This case was `evidenceExceedsOnDeviceBudget`, and `decide` compared
    /// `evidenceTokens` alone — the recall payload — against the whole budget. So
    /// a turn carrying a 2,443-token trimmed history and a 200-token recall
    /// payload routed as `.onDevice` with no escalation and no record, on a
    /// 2,710-token budget. The router could not see the conversation at all, and
    /// the conversation is most of the demand.
    ///
    /// The case is now named for the SUM, because that is what is compared and
    /// because a caller holding both numbers cannot tell which term crossed —
    /// a reason that claims to localize the cause would be guessing.
    case conversationExceedsOnDeviceBudget

    /// Never on turn 1. The first token is the thing the user feels, and M1
    /// means the cost of the alternative is unmeasured rather than known.
    case neverOnFirstTurn
}

/// The decision, and — more importantly — why the decision was allowed to be
/// made at all.
public struct RoutingDecision: Sendable, Equatable {
    public enum Tier: String, Sendable, Equatable {
        /// Measured. 4,096 combined tokens, and the only tier that has ever
        /// completed a turn on the development machine.
        case onDevice
        /// Never measured here — the request is refused without the app
        /// entitlement (`ModelManagerError 1046`).
        case deep
    }

    public let tier: Tier
    public let reason: EscalationReason?
    /// True when `.deep` was requested but refused, so the caller can tell a
    /// working escalation from a silent fallback.
    public let escalationRefused: Bool

    public static func onDevice(reason: EscalationReason? = nil) -> RoutingDecision {
        RoutingDecision(tier: .onDevice, reason: reason, escalationRefused: false)
    }
}

/// Decides which tier serves a turn.
///
/// M1 (2026-10-18) is the reason this type is shaped the way it is. The
/// escalation tier **cannot run on the development machine** — every request is
/// refused with `ModelManagerError 1046` — so there is no measured PCC latency
/// and this policy cannot be tuned against one. Rather than guess, the policy
/// refuses to enter `.deep` at all unless escalation is known to be permitted,
/// and the on-device tier is the one that must always work.
///
/// The consequence is deliberate: **a turn routed here always has somewhere to
/// go.** The escalation branch is an upgrade, not a dependency.
///
/// The availability annotation is on the TYPE, for the reason `ContextBudget`
/// records: `ContextBudget` appears in this API's signature, so guarding only
/// the bodies would leave public surfaces that do not compile at the package's
/// declared deployment target.
@available(macOS 26.0, *)
public struct ChatRouter: Sendable {
    /// Whether a `.deep` turn may be attempted.
    ///
    /// This is deliberately NOT `PrivateCloudComputeLanguageModel().isAvailable`
    /// and never derived from it. That flag is device ELIGIBILITY: it is true on
    /// machines that cannot run the request, which is the whole of M1. Routing
    /// on it produces a path that looks enabled and fails on first use.
    public let escalationPermitted: Bool

    /// The measured on-device conversation budget, tokens remaining after
    /// instructions and tools. Nil when the model could not be read.
    public let onDeviceBudget: Int?

    public init(escalationPermitted: Bool, onDeviceBudget: Int?) {
        self.escalationPermitted = escalationPermitted
        self.onDeviceBudget = onDeviceBudget
    }

    /// Reads the budget for whichever on-device build is running.
    public static func measuringOnDevice(escalationPermitted: Bool) async -> ChatRouter {
        let cost = await ContextBudget.measureOnDeviceDefaults()
        return ChatRouter(escalationPermitted: escalationPermitted, onDeviceBudget: cost)
    }

    /// The routing decision for one turn.
    ///
    /// - Parameters:
    ///   - turn: 1-based turn number. Turn 1 never escalates.
    ///   - evidenceTokens: measured size of the recall payload to inject.
    ///   - historyTokens: measured size of the conversation that will be SENT —
    ///     the post-trim transcript, which is `TrimPlan.keptTokens`. Pass the
    ///     PLAN'S number, not the pre-trim one: the trimmer exists precisely to
    ///     make this smaller, and a router that reads the untrimmed size would
    ///     escalate on conversations the trimmer already fixed.
    ///
    ///     ## WHY IT IS A PARAMETER AND NOT A DEFAULT
    ///
    ///     A defaulted `historyTokens: Int = 0` would compile at every call site
    ///     and preserve the exact blindness this parameter exists to remove: a
    ///     caller who forgets it gets "the conversation costs nothing" and a
    ///     silent overflow. There are no call sites yet, so making it required
    ///     costs nothing and makes the omission impossible.
    public func decide(
        turn: Int,
        evidenceTokens: Int,
        historyTokens: Int
    ) -> RoutingDecision {
        // Trigger 3, first, because it is unconditional and cheapest to state.
        // Turn 1 is the turn where latency is the entire user experience.
        if turn <= 1 {
            return .onDevice(reason: .neverOnFirstTurn)
        }

        // Trigger 2. Compares against the MEASURED budget, and fails closed:
        // an unreadable budget is not "plenty of room", it is an unknown, and
        // escalating on unknown is the one thing this policy avoids.
        guard let budget = onDeviceBudget, budget > 0 else {
            return .onDevice()
        }
        // The whole window is one pool — instructions, tool schemas, transcript,
        // and the answer — so the only honest demand is the SUM. `onDeviceBudget`
        // has already had instructions, tool schemas and the response reserve
        // subtracted, so what is left to compare is what the turn adds:
        //
        //     historyTokens  what `historyTransform` will send
        //   + evidenceTokens  what the recall payload injects
        //
        // Comparing the evidence alone is what let a 2,443-token conversation
        // through a 2,710-token budget on the strength of a small payload.
        let demand = historyTokens + evidenceTokens
        if demand > budget {
            guard escalationPermitted else {
                // The refusal is RECORDED, not silent. A caller that cannot see
                // it will read the fallback as "escalation was not needed".
                return RoutingDecision(
                    tier: .onDevice, reason: .conversationExceedsOnDeviceBudget,
                    escalationRefused: true
                )
            }
            return RoutingDecision(
                tier: .deep, reason: .conversationExceedsOnDeviceBudget,
                escalationRefused: false
            )
        }

        return .onDevice()
    }
}
// MARK: - Where the trim and the route meet

/// A turn that cannot be sent.
///
/// THROWS rather than returning a decision, deliberately. The alternative — a
/// plan that silently over-fits, a session that sends anyway, and a
/// `LanguageModelError.contextSizeExceeded` raised MID-GENERATION — is the
/// failure this whole layer was built to avoid, and it ends the conversation:
///
/// > *"the framework throws a `LanguageModelError.contextSizeExceeded(_:)`
/// > error and the session stops responding"*
///
/// > *"The session stops responding after this error, so create a new session
/// > in the catch block"*
///
/// Source: <https://developer.apple.com/documentation/foundationmodels/managing-the-context-window>
///
/// The shortfall travels in the case because that is the number a caller needs
/// to choose a recovery, and a bare "it didn't fit" is not actionable.
public enum ChatTurnError: Error, Equatable {
    /// The conversation does not fit the on-device window even after trimming
    /// to the exchange floor. `shortfallTokens` is how far over it is.
    case conversationDoesNotFit(shortfallTokens: Int)
}

/// Joins the trimmer and the router, so a caller has one call to make.
///
/// This type exists because `TrimPlan` and `ChatRouter` were correct and
/// DISCONNECTED: the trimmer computed a shortfall nobody read, and the router
/// compared numbers nobody supplied. Either alone is inert — the field and the
/// method were both exercised only by their own pins. The gap was exactly one
/// argument, and this is it.
@available(macOS 26.0, *)
public struct ChatTurnPlanner: Sendable {
    private let router: ChatRouter
    private let controller: HistoryBudget.Controller

    public init(router: ChatRouter, controller: HistoryBudget.Controller) {
        self.router = router
        self.controller = controller
    }

    /// Refresh the trim, then route on the result. THROWS if the conversation
    /// cannot fit the on-device window.
    ///
    /// The order matters and is the whole point: measure first, route second,
    /// and refuse BEFORE the request leaves. A caller that wants to send a
    /// turn it knows will overflow has to catch this, which is the point.
    ///
    /// - Throws: `ChatTurnError.conversationDoesNotFit` when the trimmed
    ///   conversation still exceeds the window. Not thrown when the decision is
    ///   `.deep`: that tier's window is larger, so a shortfall measured against
    ///   the ON-DEVICE budget says nothing about it — and M1 established that
    ///   the escalation tier refuses every request on this machine anyway, so
    ///   the honest answer there is still "do not send", which is the caller's
    ///   call to make with the decision in hand.
    public func prepare(
        turn: Int,
        evidenceTokens: Int,
        entries: [Transcript.Entry],
        incomingTokens: Int,
        cost: @Sendable (Transcript.Entry) async -> Int
    ) async throws -> RoutingDecision {
        await controller.refresh(entries, incomingTokens: incomingTokens, cost: cost)
        let plan = controller.currentPlan
        let decision = router.decide(
            turn: turn,
            evidenceTokens: evidenceTokens,
            // The POST-TRIM size. The pre-trim number would escalate on
            // conversations the trimmer already fixed, which is the router
            // making a decision about a transcript that will not be sent.
            historyTokens: plan.keptTokens
        )
        if decision.tier == .onDevice, plan.exceedsBudget {
            throw ChatTurnError.conversationDoesNotFit(
                shortfallTokens: plan.unmetTokens ?? 0
            )
        }
        return decision
    }

    /// The closure to hand `historyTransform`, so the caller cannot trim
    /// through one path and route through another.
    public var transform: ([Transcript.Entry]) -> [Transcript.Entry] {
        controller.transform
    }
}

// MARK: - Window budgeting by historyTransform

/// Spends the window by TRIMMING A COPY, never by rewriting the session.
///
/// Apple's key-value-caching article is the rule this exists to follow, and the
/// first version of this code followed it in the wrong direction:
///
/// > *"A stateless transform that drops entries, like truncating to recent
/// > history, invalidates parts of the cache for the entries it removes."*
///
/// > *"Defer removing entries from the transcript until the context window is
/// > nearly full, then consolidate the context in a single operation rather
/// > than trimming incrementally after each turn."*
///
/// Source: <https://developer.apple.com/documentation/foundationmodels/optimizing-key-value-caching-in-language-model-sessions>
/// (`historyTransform` and the cache-invalidation sentence are also on
/// <https://developer.apple.com/documentation/foundationmodels/managing-the-context-window>.)
///
/// So the policy is a WATERMARK, not a minimum: under pressure the trimmer cuts
/// down to `watermark` of the budget and then leaves it alone, letting several
/// turns append onto a stable prefix. Trimming to the minimum every turn — which
/// is what the first version did, and what its own comment called "the point" —
/// changes the kept set on every turn and re-prefills the whole history each
/// time, which is the cache invalidation Apple is warning about, caused by the
/// very mechanism meant to avoid it.
///
/// What `historyTransform` preserves is the TRANSCRIPT, not the cache. Apple:
/// a history transform *"doesn't affect the global state of the transcript."*
/// An earlier version of this comment claimed it "preserves the prefix cache";
/// that is a different claim and it is not this one.
@available(macOS 26.0, *)
public enum HistoryBudget {
    /// A trim, DECIDED before the request and APPLIED by a synchronous closure.
    ///
    /// This split is forced by the framework. `historyTransform` is
    /// `([Transcript.Entry]) -> [Transcript.Entry]` — synchronous, non-async,
    /// non-throwing — while the measurement that makes a trim honest,
    /// `tokenCount(for:)`, is `async throws`. The two cannot meet inside the
    /// callback, so they are split in time: `refresh` runs async before the
    /// turn, and `apply` runs sync inside the request.
    public struct TrimPlan: Sendable {
        private let dropped: Set<Transcript.Entry.ID>
        public let keptTokens: Int
        public let droppedTokens: Int

        /// Tokens the trim COULD NOT remove, because the exchange floor held.
        ///
        /// **Why this exists.** A trim can fail to meet its budget, and until this
        /// field existed it failed SILENTLY: the plan dropped what it could, the
        /// caller applied it, and the only evidence was a
        /// `LanguageModelError.contextSizeExceeded` raised MID-GENERATION. Apple is
        /// explicit about what that costs:
        ///
        /// > *"the framework throws a `LanguageModelError.contextSizeExceeded(_:)`
        /// > error and the session stops responding"*
        ///
        /// > *"The session stops responding after this error, so create a new
        /// > session in the catch block"*
        ///
        /// Source: <https://developer.apple.com/documentation/foundationmodels/managing-the-context-window>
        ///
        /// So the failure is not one lost turn, it is the END of the conversation,
        /// discovered after the request went out. The floor that causes it —
        /// never break the last exchange — is the right policy; what was wrong was
        /// that the policy could not TELL anyone it had bound.
        ///
        /// **The three values, and none of them is the default.**
        ///
        /// ## THE FIELD IS ABOUT THE BUDGET, NOT THE TARGET
        ///
        /// This was documented as "met the target, or there was nothing to
        /// trim" and **both clauses were false**. The floor binds whenever the
        /// last exchange is heavy, which stops the loop before the WATERMARK
        /// while still landing inside the budget. Measured, budget 1000 (target
        /// 700), costs `100,100,100,100,700,100`:
        ///
        ///     target=700  kept=1000  unmet=0  exceeds=false
        ///
        /// The plan missed its own target, had droppable entries left, and
        /// reported `0` — matching neither clause. So the accurate statement:
        ///
        /// - `0` — the kept tokens are WITHIN `budgetTokens`. This is the whole
        ///   meaning. It does NOT mean the watermark target was reached: the
        ///   floor binds between the two routinely, and that is fine, because a
        ///   trim that cannot reach its own target and still fits is a WORKING
        ///   trim. Do not read `0` as "met the target".
        /// - positive — the kept tokens EXCEED `budgetTokens` by this many, so
        ///   the request will overflow the window. The floor held with entries
        ///   still to drop. A caller that routes should treat this as an
        ///   escalation input, not as a trim that worked.
        /// - `nil` — the plan could not be MEASURED (costs that do not match the
        ///   transcript). Unknown is not zero: reporting `0` there would certify
        ///   an unmeasured plan as a fitting one, which is the same silence this
        ///   field was added to remove. `nil` makes the caller fail closed, the
        ///   way an unreadable budget already does in `ChatRouter`.
        ///
        /// ## WHAT IS STILL OWED, and is not implemented here
        ///
        /// Apple names the recovery: *"consider summarizing the conversation and
        /// starting a fresh session with that summary as context"*, and again in
        /// the caching article — *"If you need to remove older entries, consider
        /// summarizing the conversation and starting a fresh session."*
        ///
        /// Source: <https://developer.apple.com/documentation/foundationmodels/optimizing-key-value-caching-in-language-model-sessions>
        ///
        /// That is a model round-trip whose latency and fidelity are UNMEASURED
        /// here, and M1 already established that the alternative tier refuses
        /// every request on this machine. So today the floor-bound turn sends an
        /// over-budget request and the framework ends the session. This field
        /// makes that turn VISIBLE and ROUTABLE before the request goes out; it
        /// does not make it survivable, and it is named rather than implied.
        public let unmetTokens: Int?

        init(
            dropped: Set<Transcript.Entry.ID>,
            keptTokens: Int,
            droppedTokens: Int,
            unmetTokens: Int?
        ) {
            self.dropped = dropped
            self.keptTokens = keptTokens
            self.droppedTokens = droppedTokens
            self.unmetTokens = unmetTokens
        }

        /// Whether the plan is known to exceed the window. `false` for the
        /// ordinary case; `true` when the floor held, and ALSO when the plan
        /// could not be measured — an unmeasured plan is never a fitting one.
        public var exceedsBudget: Bool {
            (unmetTokens ?? 1) > 0
        }

        /// The synchronous half. Drops a NAMED set rather than keeping a named
        /// one, and that direction is load-bearing.
        ///
        /// The plan is built BEFORE the request, so it cannot know about the
        /// turn's own prompt — which the framework hands to the transform
        /// afterwards and which must survive, or the request has no question in
        /// it. A keep-set built from a pre-turn transcript drops that prompt and
        /// the framework rejects the result: *"Transcript must end with a .prompt
        /// or .toolOutput entry."* Measured, not predicted: the first version
        /// returned ZERO entries on turn one.
        ///
        /// Drop-set semantics means an entry the plan never saw is kept by
        /// default, so a stale plan costs one turn of un-budgeted context rather
        /// than a dead conversation.
        public func apply(_ entries: [Transcript.Entry]) -> [Transcript.Entry] {
            entries.filter { !dropped.contains($0.id) }
        }
    }

    /// How far below the budget a trim cuts, as a fraction.
    ///
    /// Not 1.0 (cut to the minimum, re-prefill every turn — the bug this
    /// replaces) and not something aggressive: the watermark exists so several
    /// turns can append onto a stable prefix before the next consolidation.
    public static let watermark = 0.7

/// The floor on what survives a trim: the last exchange — a question and its
/// answer — which is the minimum from which "continue the conversation" means
/// anything. Everything after the latest response is never touched, which is
/// what makes this floor real rather than aspirational.
    ///
    /// ## THE FLOOR CAN BIND, and a bound floor is a REPORTED outcome
    ///
    /// Two entries is one exchange for a plain question-and-answer transcript,
    /// which is what this constant was written for. It stops being exactly that
    /// the moment an exchange carries tool traffic or reasoning: the loop drops
    /// whole units, so the floor then holds whatever the last unit weighs. That
    /// is measured, not hypothetical — the model-driven history-pressure test
    /// holds at 2,443 tokens against a 2,710 budget with a 1,897 watermark
    /// target, because dropping the last exchange is not on the table.
    ///
    /// A bound floor means the trim CANNOT meet the budget by dropping. It used
    /// to mean that silently. Now `TrimPlan.unmetTokens` carries the shortfall,
    /// so the decision to send an over-budget request — or to route elsewhere —
    /// happens BEFORE the request, where a caller can still make it. The
    /// alternative is the framework's own: send it, and lose the session to
    /// `contextSizeExceeded`.
    static let keepAtLeast = 2

    /// Measures and plans.
    public static func plan(
        _ entries: [Transcript.Entry],
        budgetTokens: Int,
        cost: @Sendable (Transcript.Entry) async -> Int
    ) async -> TrimPlan {
        var costs: [Int] = []
        costs.reserveCapacity(entries.count)
        for entry in entries {
            costs.append(await cost(entry))
        }
        return plan(entries, costs: costs, budgetTokens: budgetTokens)
    }

    /// The planner proper, over costs the caller has already measured.
    ///
    /// Split out so `Controller` can hand over a CACHED cost table without
    /// re-measuring history it measured last turn, and so the policy is a pure
    /// function of (entries, costs, budget) — testable without a model.
    public static func plan(
        _ entries: [Transcript.Entry],
        costs: [Int],
        budgetTokens: Int
    ) -> TrimPlan {
        // Every exit below reports what it could not drop. The helper exists so
        // no return can forget: a plan that is silent about its own shortfall is
        // the defect `unmetTokens` was added for.
        func unmet(_ kept: Int) -> Int { max(0, kept - budgetTokens) }
        guard budgetTokens > 0, !entries.isEmpty else {
            return TrimPlan(dropped: [], keptTokens: 0, droppedTokens: 0, unmetTokens: 0)
        }
        guard costs.count == entries.count else {
            // UNMEASURED, and reported as such. The costs do not describe this
            // transcript, so the size of the conversation is unknown — and an
            // unknown size is not a fitting one. `nil` rather than `0` because a
            // caller that cannot measure must fail closed, not proceed on a
            // number nobody computed.
            return TrimPlan(dropped: [], keptTokens: 0, droppedTokens: 0, unmetTokens: nil)
        }
        let total = costs.reduce(0, +)
        let target = Int(Double(budgetTokens) * watermark)
        guard total > budgetTokens else {
            // Under budget: drop nothing. Apple's "defer removing entries until
            // the window is nearly full" taken literally — the common case
            // touches nothing at all.
            return TrimPlan(
                dropped: [], keptTokens: total, droppedTokens: 0, unmetTokens: unmet(total)
            )
        }

        // Only entries BEFORE THE LATEST PROMPT are eligible — not before the
        // latest RESPONSE, which is what this said and was wrong by one entry.
        //
        // Protecting from the last `.response` left the final `.prompt` inside
        // the eligible range, so a trim could drop `q5` and keep `a5`, handing
        // the model an answer to a question it can no longer see. Protecting
        // from the last `.prompt` keeps the exchange in flight WHOLE, which is
        // also the runtime case: `historyTransform` is handed the history
        // including the pending prompt, and that prompt is the question being
        // asked right now — it must never be droppable.
        guard let promptStart = entries.lastIndex(where: { isPrompt($0) }) else {
            return TrimPlan(
                dropped: [], keptTokens: total, droppedTokens: 0, unmetTokens: unmet(total)
            )
        }
        let eligible = Array(entries[..<promptStart])
        guard eligible.count > keepAtLeast else {
            // Nothing droppable without breaking the last exchange. This is the
            // floor in its purest form — a conversation whose newest exchange is
            // larger than the budget — and it is the case that used to reach the
            // framework as a silent overflow.
            return TrimPlan(
                dropped: [], keptTokens: total, droppedTokens: 0, unmetTokens: unmet(total)
            )
        }
        let eligibleCosts = Array(costs[..<promptStart])

        var dropped = Set<Transcript.Entry.ID>()
        var remaining = total

        // Tool traffic first, and AS PAIRS. `toolCalls` + `toolOutput` are
        // dropped together or not at all, so the walk walks PAIRS and stops on a
        // pair boundary — which is the thing the token-exact loop could not do.
        var index = 0
        while index < eligible.count, remaining > target {
            guard isToolTraffic(eligible[index]) else { index += 1; continue }
            var pairEnd = index + 1
            while pairEnd < eligible.count, isToolTraffic(eligible[pairEnd]) { pairEnd += 1 }
            for i in index..<pairEnd {
                dropped.insert(eligible[i].id)
                remaining -= eligibleCosts[i]
            }
            index = pairEnd
        }

        // OLDEST FIRST, walking FORWARD, and BY EXCHANGE — a prompt together
        // with everything up to the next prompt (its response, and any tool
        // traffic the pair walk above left in place), dropped as one unit with
        // the budget checked between units.
        //
        // The unit matters as much as the direction. Dropping entries one at a
        // time stops the moment the budget fits, and that moment lands between
        // a prompt and its response as often as not. The runtime shape is the
        // one that exposed it: `historyTransform` receives the history WITH the
        // pending prompt, so the in-flight pair is protected but the exchange
        // just before it is eligible — and the entry-wise loop dropped `q4`,
        // fitted, and kept `a4`. The orphan the pair walk prevents for tools
        // was still being produced for the conversation itself.
        //
        // Both halves were found by drill: `.reversed()` kept `q0 a0 a5`, and
        // the entry-wise loop with a pending `q6` kept `a4 q5 a5 q6`.
        var start = 0
        while start < eligible.count, remaining > target {
            var end = start + 1
            while end < eligible.count, !isPrompt(eligible[end]) { end += 1 }
            let alive = eligible.indices.filter { !dropped.contains(eligible[$0].id) }.count
            let unit = (start..<end).filter { !dropped.contains(eligible[$0].id) }
            // The floor, in the unit the loop now drops by. Stopping here with
            // entries still to drop is not a silent outcome: `unmetTokens`
            // reports the overflow above, so this break is visible to the caller
            // before the request rather than after it.
            guard alive - unit.count >= keepAtLeast else { break }
            for i in unit {
                dropped.insert(eligible[i].id)
                remaining -= eligibleCosts[i]
            }
            start = end
        }

        let kept = entries.indices.filter { !dropped.contains(entries[$0].id) }
        let keptTokens = kept.reduce(0) { $0 + costs[$1] }
        return TrimPlan(
            dropped: dropped,
            keptTokens: keptTokens,
            droppedTokens: total - remaining,
            unmetTokens: unmet(keptTokens)
        )
    }

    static func isPrompt(_ entry: Transcript.Entry) -> Bool {
        if case .prompt = entry { return true }
        return false
    }

    static func isToolTraffic(_ entry: Transcript.Entry) -> Bool {
        switch entry {
        case .toolCalls, .toolOutput: true
        default: false
        }
    }

    /// Holds the budget across turns and hands `historyTransform` something
    /// SYNCHRONOUS to call.
    ///
    /// Per-entry costs are CACHED BY ID and EVICTED with the entries they belong
    /// to — an unbounded cache in a long session is a slow leak, and a chat
    /// session is the one place this code runs for hours at a time.
    public final class Controller: @unchecked Sendable {
        private let lock = NSLock()
        private var cachedCosts: [Transcript.Entry.ID: Int] = [:]
        // `unmetTokens: nil` before the first refresh, and that is the honest value:
        // nothing has been measured yet, so whether the conversation fits is
        // unknown — and `exceedsBudget` reads unknown as "assume it does not".
        // A `0` here would be the quiet version of the same bug this field was
        // added to end: certifying a plan nobody measured.
        private var plan = TrimPlan(dropped: [], keptTokens: 0, droppedTokens: 0, unmetTokens: nil)
        public let budgetTokens: Int

        public init(budgetTokens: Int) {
            self.budgetTokens = budgetTokens
        }

        /// Re-measures and re-plans. Call once per turn, before responding.
        ///
        /// - Parameter incomingTokens: the measured cost of the prompt this turn
        ///   is ABOUT TO SEND, charged against the budget up front. Without it
        ///   the plan is one turn stale, and a turn whose own prompt is large is
        ///   admitted against a budget that did not know about it — the session
        ///   then throws `contextSizeExceeded` on exactly the turn meant to prove
        ///   trimming works.
        public func refresh(
            _ entries: [Transcript.Entry],
            incomingTokens: Int = 0,
            cost: @Sendable (Transcript.Entry) async -> Int
        ) async {
            var costs: [Int] = []
            costs.reserveCapacity(entries.count)
            for entry in entries {
                if let known = peekCost(entry.id) {
                    costs.append(known)
                } else {
                    costs.append(await cost(entry))
                }
            }
            let built = HistoryBudget.plan(
                entries,
                costs: costs,
                budgetTokens: max(budgetTokens - max(incomingTokens, 0), 0)
            )
            lock.withLock {
                let live = Set(entries.map(\.id))
                cachedCosts = cachedCosts.filter { live.contains($0.key) }
                for (index, entry) in entries.enumerated() {
                    cachedCosts[entry.id] = costs[index]
                }
                plan = built
            }
        }

        /// The closure handed to `historyTransform`.
        public var transform: ([Transcript.Entry]) -> [Transcript.Entry] {
            { [self] entries in
                lock.withLock { plan }.apply(entries)
            }
        }

        public var currentPlan: TrimPlan {
            lock.withLock { plan }
        }

        private func peekCost(_ id: Transcript.Entry.ID) -> Int? {
            lock.withLock { cachedCosts[id] }
        }
    }
}
