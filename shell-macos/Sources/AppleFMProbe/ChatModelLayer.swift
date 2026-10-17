import Foundation

#if canImport(FoundationModels)
import FoundationModels
#endif

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
/// Plan 19 lists a FIRST trigger above these — "reasoning the small model is
/// measured to get wrong" — and it is deliberately ABSENT rather than carried as
/// an unimplemented case. It was an enum case for a while and it was removed: no
/// code produced it, and a case nothing constructs is a promise with no
/// implementation behind it. The measurement it depends on has not been taken,
/// so the trigger does not exist yet.
public enum EscalationReason: String, Sendable, Equatable, CaseIterable {
    /// Trigger 2 — the evidence to inject exceeds the remaining on-device
    /// budget. The only trigger currently decidable from a measurement, because
    /// the budget IS measured (`ContextBudget`).
    case evidenceExceedsOnDeviceBudget

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
    public func decide(turn: Int, evidenceTokens: Int) -> RoutingDecision {
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
        if evidenceTokens > budget {
            guard escalationPermitted else {
                // The refusal is RECORDED, not silent. A caller that cannot see
                // it will read the fallback as "escalation was not needed".
                return RoutingDecision(
                    tier: .onDevice, reason: .evidenceExceedsOnDeviceBudget,
                    escalationRefused: true
                )
            }
            return RoutingDecision(
                tier: .deep, reason: .evidenceExceedsOnDeviceBudget,
                escalationRefused: false
            )
        }

        return .onDevice()
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

        init(dropped: Set<Transcript.Entry.ID>, keptTokens: Int, droppedTokens: Int) {
            self.dropped = dropped
            self.keptTokens = keptTokens
            self.droppedTokens = droppedTokens
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
        guard budgetTokens > 0, !entries.isEmpty, costs.count == entries.count else {
            return TrimPlan(dropped: [], keptTokens: 0, droppedTokens: 0)
        }
        let total = costs.reduce(0, +)
        let target = Int(Double(budgetTokens) * watermark)
        guard total > budgetTokens else {
            // Under budget: drop nothing. Apple's "defer removing entries until
            // the window is nearly full" taken literally — the common case
            // touches nothing at all.
            return TrimPlan(dropped: [], keptTokens: total, droppedTokens: 0)
        }

        // Only entries BEFORE the latest response are eligible. The suffix from
        // there on is the exchange in flight: the model's most recent answer and
        // the prompt being answered now. Dropping from there produced orphans,
        // because a `.toolCalls` entry and its `.toolOutput` sit adjacent and a
        // token-exact cut lands between them as often as not. Apple's own
        // transform shape filters tool traffic within `history[0..<latestResponse]`
        // and keeps the suffix whole; this mirrors it.
        guard let responseStart = entries.lastIndex(where: { isResponse($0) }) else {
            return TrimPlan(dropped: [], keptTokens: total, droppedTokens: 0)
        }
        let eligible = Array(entries[..<responseStart])
        guard eligible.count > keepAtLeast else {
            return TrimPlan(dropped: [], keptTokens: total, droppedTokens: 0)
        }
        let eligibleCosts = Array(costs[..<responseStart])

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

        // Then oldest-first down to the watermark, never past the floor.
        for i in eligible.indices.reversed() where remaining > target {
            guard !dropped.contains(eligible[i].id) else { continue }
            let alive = eligible.indices.filter { !dropped.contains(eligible[$0].id) }.count
            guard alive > keepAtLeast else { break }
            dropped.insert(eligible[i].id)
            remaining -= eligibleCosts[i]
        }

        let kept = entries.indices.filter { !dropped.contains(entries[$0].id) }
        return TrimPlan(
            dropped: dropped,
            keptTokens: kept.reduce(0) { $0 + costs[$1] },
            droppedTokens: total - remaining
        )
    }

    static func isResponse(_ entry: Transcript.Entry) -> Bool {
        if case .response = entry { return true }
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
        private var plan = TrimPlan(dropped: [], keptTokens: 0, droppedTokens: 0)
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
