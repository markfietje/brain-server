// brain-afm — the AFM bridge executable (the chat spine, Round 1).
//
// PROTOCOL (JSONL over stdio; one request per line in, one event per line out;
// exit on EOF). The kernel's bridge client is the only intended caller.
//
//   {"id":"c1","op":"capabilities"}
//     → {"id":"c1","ok":true,"window":4096,"budget":2710,
//        "languages":["en",…],"pccWindow":32768,"pccEligible":false,
//        "pccGeneration":"refused","escalationPermitted":false}
//
//   {"id":"t1","op":"turn","session":"s1","prompt":"…",
//    "instructions":"…"(first turn of a session only),"maxResponseTokens":512}
//     → zero or more {"id":"t1","event":"delta","text":"…"}
//       then exactly one terminal event:
//       {"id":"t1","event":"final","text":"…","abstain":false,
//        "claims":[{"text":"…","cites":[0,2]}],"inputTokens":N,"outputTokens":N,
//        "tier":"on_device","turn":N,"escalationRefused":false}
//       {"id":"t1","event":"error","code":…,"message":…,"shortfallTokens":N?}
//
// The `final` event is TYPED as of LL-04 (§1.1): the answer arrives through
// guided generation as a GroundedAnswer — abstain first, then the prose,
// then the sentence-level claims with the hit indices they assert. The
// kernel, not this process, judges those indices (no model in the gate).
//
//   {"id":"x1","op":"cancel","target":"t1"}   → abandons that turn's task
//
// WHY SESSIONS LIVE HERE. The conversation's accumulated transcript — the
// thing HistoryBudget trims — can only exist inside a live
// `LanguageModelSession` (macOS 27 additionally allows seeding one from
// history; the rebuild path is a named Round-2 item, not shipped here). The
// kernel keeps its own audit copy (agent_session_events) and sends each
// turn's NEW prompt only; continuity is this process's state, keyed by the
// kernel's session id.
//
// macOS 27 IS REQUIRED for sessions: the trimmer rides `historyTransform`,
// which is a DynamicProfile method (27-only). On 26 the bridge answers
// capabilities and refuses turns with `model_unavailable` — named, not
// halfway-working. (Without the trimmer attached, an over-window session
// dies mid-generation with `contextSizeExceeded` — the exact failure the
// ChatModelLayer exists to prevent — so degrading silently is worse than
// refusing loudly.)
//
// WHAT THIS DOES NOT DO: no tools (Round 1 chat has none), no network except
// Apple's own framework (PCC), no keys, no tokens, no database. Errors are
// named codes, never prose parsed by the caller: model_unavailable ·
// conversation_does_not_fit · generation_failed · instructions_locked ·
// bad_request.
//
// PCC, measured not assumed: capabilities PROBES Private Cloud Compute with a
// one-line generation. On this machine (2026-10-25, macOS 27.0.1) the request
// is refused (`ModelManagerError 1046`) — NOT a hardware fact: PCC needs an
// APP-LEVEL entitlement (Apple developer-program enrollment; per-user daily
// quotas apply), so an unentitled process is refused while the model still
// reports itself "available". That split is why `pccEligible` and
// `pccGeneration` are separate fields: routing on eligibility alone produces
// a path that fails on first use. With the refusal measured,
// escalationPermitted is false and `.deep` stays nominal — exactly the
// posture ChatRouter was built for. The probe answer is cached for the
// process lifetime.

import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

import AppleFMProbe

// ─────────────────────────────────────────────────────────────────────────
// Output
// ─────────────────────────────────────────────────────────────────────────

/// One writer for stdout: JSONL lines, never interleaved. Every reply the
/// bridge emits crosses here. JSONSerialization with sorted keys keeps the
/// wire deterministic for the Rust side's fixtures.
final class LineOut: @unchecked Sendable {
    private let lock = NSLock()
    private let handle = FileHandle.standardOutput

    /// Optional values cross as JSON null, not as absent keys: the reader's
    /// contract is "every key is always present".
    func null(_ value: Any?) -> Any {
        value ?? NSNull()
    }

    func send(_ object: [String: Any]) {
        guard JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(
                  withJSONObject: object,
                  options: [.sortedKeys, .withoutEscapingSlashes]
              )
        else { return }
        var line = data
        line.append(0x0A)
        lock.withLock {
            try? handle.write(contentsOf: line)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// The in-flight registry (cancel support)
// ─────────────────────────────────────────────────────────────────────────

/// `op:cancel` names the request id of a running turn. The registry maps it
/// to that turn's Task; a cancel for an unknown/finished id is a no-op,
/// because "already done" and "never existed" are the same fact to a caller
/// holding a dropped stream.
final class InFlight: @unchecked Sendable {
    private let lock = NSLock()
    private var tasks: [String: Task<Void, Never>] = [:]

    func register(_ id: String, _ task: Task<Void, Never>) {
        lock.withLock { tasks[id] = task }
    }

    func cancel(_ id: String) {
        lock.withLock { tasks.removeValue(forKey: id) }?.cancel()
    }

    func finish(_ id: String) {
        lock.withLock { _ = tasks.removeValue(forKey: id) }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Errors — named codes on the wire, never prose a caller parses
// ─────────────────────────────────────────────────────────────────────────

enum BridgeError: Error {
    case modelUnavailable(String)
    case conversationDoesNotFit(shortfall: Int)
    case generationFailed(String)
    case instructionsLocked
    case badRequest(String)

    var code: String {
        switch self {
        case .modelUnavailable: "model_unavailable"
        case .conversationDoesNotFit: "conversation_does_not_fit"
        case .generationFailed: "generation_failed"
        case .instructionsLocked: "instructions_locked"
        case .badRequest: "bad_request"
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// The typed answer — the @Generable schema lives in the AppleFMProbe
// library (`Sources/AppleFMProbe/GroundedAnswer.swift`), where the protocol
// pins can reach it: the bridge generates against it, the pins round-trip
// it, and the kernel's Rust mirror (`bridge_afm.rs`) reads its wire shape.
// ─────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────
// Capabilities (measured once, cached for the process lifetime)
// ─────────────────────────────────────────────────────────────────────────

#if canImport(FoundationModels)
struct Capabilities: Sendable {
    let window: Int
    let budget: Int?
    let languages: [String]
    let pccWindow: Int?
    let pccEligible: Bool
    /// "ok" · "refused" · "unavailable" — a MEASURED generation probe, not
    /// `isAvailable` (eligibility is true on machines that still refuse).
    let pccGeneration: String

    var escalationPermitted: Bool { pccGeneration == "ok" }

    @available(macOS 26.0, *)
    static func measure() async -> Capabilities {
        let model = SystemLanguageModel.default
        guard case .available = model.availability else {
            return Capabilities(
                window: 0, budget: nil, languages: [], pccWindow: nil,
                pccEligible: false, pccGeneration: "unavailable"
            )
        }
        let window = ChatWindow.onDevice().tokens ?? 0
        let budget = await BudgetCache.shared.measured()
        let languages = model.supportedLanguages.map {
            $0.minimalIdentifier.lowercased()
        }

        // PCC: window read (fast, local) + a one-line generation probe. A
        // refusal is an ANSWER, not a failure of this method.
        var pccWindow: Int?
        var pccEligible = false
        var pccGeneration = "unavailable"
        if #available(macOS 27.0, *) {
            let pcc = PrivateCloudComputeLanguageModel()
            pccEligible = pcc.isAvailable
            if case let .window(n) = await ChatWindow.privateCloud() {
                pccWindow = n
            }
            if await Self.probePCC(pcc) == true {
                pccGeneration = "ok"
            } else {
                pccGeneration = pccEligible ? "refused" : "unavailable"
            }
        }
        return Capabilities(
            window: window, budget: budget, languages: languages,
            pccWindow: pccWindow, pccEligible: pccEligible,
            pccGeneration: pccGeneration
        )
    }

    /// One bounded generation on PCC: "ok" in at most 8 tokens, greedy.
    /// Nil means the attempt could not be made or did not settle in time.
    @available(macOS 27.0, *)
    private static func probePCC(
        _ model: PrivateCloudComputeLanguageModel
    ) async -> Bool? {
        guard model.isAvailable else { return false }
        return await withTaskGroup(of: Bool?.self) { group in
            group.addTask {
                do {
                    let session = LanguageModelSession(
                        profile: LanguageModelSession.Profile {}
                            .model(model)
                    )
                    let options = GenerationOptions(
                        samplingMode: .greedy, maximumResponseTokens: 8
                    )
                    var sawText = false
                    for try await snapshot in session.streamResponse(
                        to: "Reply with exactly: ok", options: options
                    ) {
                        if !snapshot.content.isEmpty { sawText = true }
                    }
                    return sawText
                } catch {
                    return false
                }
            }
            group.addTask {
                // Bounded: a hung network path must not hang capabilities.
                // 20 s is generous against the one-word probe's expected
                // sub-second cost.
                try? await Task.sleep(nanoseconds: 20_000_000_000)
                return nil
            }
            let first = await group.next() ?? nil
            group.cancelAll()
            return first
        }
    }
}

/// The process-lifetime cache of the measured on-device budget — the same
/// number capabilities reports, so a session's trimmer and the router's
/// budget agree by construction. Measured once (async side), read anywhere.
final class BudgetCache: @unchecked Sendable {
    static let shared = BudgetCache()
    private let lock = NSLock()
    private var value: Int?

    @available(macOS 26.0, *)
    func measured() async -> Int? {
        if let cached = lock.withLock({ value }) {
            return cached
        }
        let measured = await ContextBudget.measureOnDeviceDefaults()
        lock.withLock { value = measured }
        return measured
    }
}
#endif

// ─────────────────────────────────────────────────────────────────────────
// Sessions
// ─────────────────────────────────────────────────────────────────────────

#if canImport(FoundationModels)
/// `@unchecked Sendable` with a stated invariant: the only mutable field
/// (`turn`) and every session API call happen INSIDE the per-session
/// `TurnGate`, which serializes turns by construction. Nothing else mutates.
@available(macOS 26.0, *)
final class BridgeSession: @unchecked Sendable {
    let session: LanguageModelSession
    let controller: HistoryBudget.Controller
    let planner: ChatTurnPlanner
    let instructions: String?
    private(set) var turn: Int = 0

    /// Builds the session with the trimmer ATTACHED — a session without
    /// `historyTransform` is not a degraded session but a doomed one (the
    /// planner would decide against a budget nothing enforces). The budget
    /// is handed in (measured on the async side, cached process-wide) so
    /// session construction stays sync under the sessions lock.
    init(instructions: String?, budget: Int, escalationPermitted: Bool) throws {
        let model = SystemLanguageModel.default
        guard case .available = model.availability else {
            throw BridgeError.modelUnavailable(
                "the on-device model is not available on this machine"
            )
        }
        // macOS 27 IS the session requirement: `historyTransform` is a
        // DynamicProfile method (27-only). Below it the turn is refused by
        // name rather than sent into a session whose window nothing guards.
        guard #available(macOS 27.0, *) else {
            throw BridgeError.modelUnavailable(
                "sessions require macOS 27 (historyTransform is a DynamicProfile method)"
            )
        }

        let controller = HistoryBudget.Controller(budgetTokens: budget)
        self.controller = controller
        self.instructions = instructions

        // Instructions ride the Profile's DynamicInstructions builder —
        // `Instructions` conforms, and a String is InstructionsRepresentable.
        let base = instructions.map { text in
            LanguageModelSession.Profile { Instructions(text) }
        } ?? LanguageModelSession.Profile {}
        self.session = LanguageModelSession(
            profile: base
                .model(model)
                .historyTransform(controller.transform)
        )

        self.planner = ChatTurnPlanner(
            router: ChatRouter(
                escalationPermitted: escalationPermitted,
                onDeviceBudget: budget
            ),
            controller: controller
        )
    }

    func nextTurn() -> Int {
        turn += 1
        return turn
    }
}

/// All live sessions. Turns for one session serialize through a per-session
/// actor gate; the kernel drives one turn at a time, but the bridge does not
/// trust its caller with its own state.
@available(macOS 26.0, *)
final class Sessions: @unchecked Sendable {
    /// A session's PROTOCOL identity, distinct from its model-backed half:
    /// the instructions and the turn gate exist the moment the id is first
    /// used, whether or not a `BridgeSession` can be built on this machine.
    /// The instructions law lives on the slot, so the lock refuses a changed
    /// instruction even where no model can ever answer.
    final class Slot {
        let instructions: String?
        let gate = TurnGate()
        var bridge: BridgeSession?
        init(instructions: String?) { self.instructions = instructions }
    }

    private let lock = NSLock()
    private var slots: [String: Slot] = [:]

    /// The one place a session id is created, and the one place the
    /// instructions law is decided: changing instructions later invalidates
    /// the entire cached prefix (Apple's own contract), so a later
    /// instruction is REFUSED, not merged. Pure registry work — no model,
    /// no budget — so a refusal's ORDER does not depend on the machine.
    func resolve(id: String, instructions: String?) throws -> Slot {
        try lock.withLock {
            if let slot = slots[id] {
                if let wanted = instructions, wanted != slot.instructions {
                    throw BridgeError.instructionsLocked
                }
                return slot
            }
            let slot = Slot(instructions: instructions)
            slots[id] = slot
            return slot
        }
    }

    /// The model-backed half, built once per slot. This is the step that
    /// may answer `model_unavailable`; the slot it attaches to already
    /// carries the session's identity and its instructions lock.
    func materialize(
        _ slot: Slot, budget: Int, escalationPermitted: Bool
    ) throws -> BridgeSession {
        try lock.withLock {
            if let bridge = slot.bridge { return bridge }
            let fresh = try BridgeSession(
                instructions: slot.instructions, budget: budget,
                escalationPermitted: escalationPermitted
            )
            slot.bridge = fresh
            return fresh
        }
    }
}

/// An empty actor: one method, so calls serialize by construction.
actor TurnGate {
    func run(_ body: @Sendable () async throws -> Void) async rethrows {
        try await body()
    }
}
#endif

// ─────────────────────────────────────────────────────────────────────────
// main
// ─────────────────────────────────────────────────────────────────────────

@main
struct BrainAfmBridge {
    static func main() async {
        let out = LineOut()
        let inflight = InFlight()

        #if canImport(FoundationModels)
        guard #available(macOS 26.0, *) else {
            out.send([
                "event": "fatal", "code": "model_unavailable",
                "message": "FoundationModels requires macOS 26"
            ])
            return
        }
        let sessions = Sessions()
        var capabilities: Capabilities?

        // A stream error (not EOF) ends the bridge: the kernel's provider
        // treats child exit as a named failure and re-spawns on the next
        // turn, so a half-alive loop is worse than a dead one.
        do {
        for try await line in FileHandle.standardInput.bytes.lines {
            guard let data = line.data(using: .utf8),
                  let object = (try? JSONSerialization.jsonObject(with: data))
                    as? [String: Any],
                  let id = object["id"] as? String,
                  let op = object["op"] as? String
            else {
                out.send([
                    "event": "error", "code": "bad_request",
                    "message": "each line must be {id, op, …} JSON"
                ])
                continue
            }

            switch op {
            case "capabilities":
                if capabilities == nil {
                    capabilities = await Capabilities.measure()
                }
                guard let caps = capabilities else { fatalError("unreachable") }
                out.send([
                    "id": id, "ok": true,
                    "window": caps.window,
                    "budget": out.null(caps.budget),
                    "languages": caps.languages,
                    "pccWindow": out.null(caps.pccWindow),
                    "pccEligible": caps.pccEligible,
                    "pccGeneration": caps.pccGeneration,
                    "escalationPermitted": caps.escalationPermitted
                ])

            case "turn":
                guard let session = object["session"] as? String,
                      let prompt = object["prompt"] as? String
                else {
                    out.send([
                        "id": id, "event": "error", "code": "bad_request",
                        "message": "turn requires session and prompt"
                    ])
                    continue
                }
                let instructions = object["instructions"] as? String
                let maxResponse = object["maxResponseTokens"] as? Int
                let requestCap = min(max(maxResponse ?? 512, 16), 1024)

                let task = Task {
                    defer { inflight.finish(id) }
                    do {
                        guard let caps = capabilities else {
                            throw BridgeError.badRequest(
                                "capabilities must be requested before the first turn"
                            )
                        }
                        // The lock is a PROTOCOL decision, so it is resolved
                        // FIRST — before the budget measurement and the model
                        // itself, either of which answers `model_unavailable`
                        // on a machine without the model. A refusal whose
                        // ORDER depends on the machine's posture is not a
                        // contract; resolve() decides the instructions law
                        // against the registry alone, and materialize() is
                        // the only step that needs the model.
                        let slot = try sessions.resolve(
                            id: session, instructions: instructions
                        )
                        guard let budget = await BudgetCache.shared.measured() else {
                            throw BridgeError.modelUnavailable(
                                "the on-device budget could not be measured"
                            )
                        }
                        let bridgeSession = try sessions.materialize(
                            slot, budget: budget,
                            escalationPermitted: caps.escalationPermitted
                        )
                        try await slot.gate.run {
                            let turnNo = bridgeSession.nextTurn()
                            try await Self.runTurn(
                                id: id, prompt: prompt, turnNo: turnNo,
                                cap: requestCap,
                                bridgeSession: bridgeSession,
                                out: out
                            )
                        }
                    } catch let error as BridgeError {
                        var event: [String: Any] = [
                            "id": id, "event": "error", "code": error.code
                        ]
                        if case let .conversationDoesNotFit(shortfall) = error {
                            event["shortfallTokens"] = shortfall
                        }
                        if case let .generationFailed(message) = error {
                            event["message"] = String(message.prefix(300))
                        }
                        if case let .modelUnavailable(message) = error {
                            event["message"] = message
                        }
                        if case let .badRequest(message) = error {
                            event["message"] = message
                        }
                        out.send(event)
                    } catch {
                        out.send([
                            "id": id, "event": "error",
                            "code": "generation_failed",
                            "message": String(String(describing: error).prefix(300))
                        ])
                    }
                }
                inflight.register(id, task)
                await task.value

            case "cancel":
                if let target = object["target"] as? String {
                    inflight.cancel(target)
                }

            default:
                out.send([
                    "id": id, "event": "error", "code": "bad_request",
                    "message": "unknown op \(op)"
                ])
            }
        }
        } catch {
            out.send([
                "event": "fatal", "code": "generation_failed",
                "message": String(String(describing: error).prefix(300))
            ])
        }
        #else
        out.send([
            "event": "fatal", "code": "model_unavailable",
            "message": "built without FoundationModels"
        ])
        #endif
    }

    #if canImport(FoundationModels)
    /// One turn on an existing session: plan (which may refuse BEFORE the
    /// request leaves), stream snapshots as incremental deltas, emit exactly
    /// one terminal event. Runs inside the session's gate — serialized by
    /// construction.
    @available(macOS 26.0, *)
    static func runTurn(
        id: String, prompt: String, turnNo: Int, cap: Int,
        bridgeSession: BridgeSession, out: LineOut
    ) async throws {
        let model = SystemLanguageModel.default
        let session = bridgeSession.session

        // Measurement is the floor the whole planner stands on:
        // `tokenCount` needs macOS 26.4, and a turn that cannot be measured
        // cannot be routed — it is refused by name rather than sent blind.
        guard #available(macOS 26.4, *) else {
            throw BridgeError.modelUnavailable(
                "turn measurement requires macOS 26.4 (tokenCount)"
            )
        }

        // Measure then route, per the planner's own law. The whole prompt —
        // fenced recall grounding included — is the turn's demand, so it is
        // measured as one unit and `evidenceTokens` is 0 by construction,
        // not by omission. An unmeasurable prompt fails closed (0 would
        // certify an unknown size as a fitting one).
        let entries = Array(session.transcript)
        let promptTokens = try await model.tokenCount(for: prompt)
        do {
            let decision = try await bridgeSession.planner.prepare(
                turn: turnNo,
                evidenceTokens: 0,
                entries: entries,
                incomingTokens: promptTokens,
                cost: { entry in
                    // The layer's cost closure is non-throwing BY DESIGN —
                    // the failure policy belongs to this caller. An entry
                    // that cannot be measured maps to an enormous cost, so
                    // the plan cannot fit it and the turn fails closed as
                    // conversation_does_not_fit: an unmeasured entry is a
                    // cost nobody knows, never a free one.
                    (try? await model.tokenCount(for: [entry])) ?? (1 << 30)
                }
            )
            // Round 1: `.deep` is nominal (PCC refused on every machine
            // measured so far), so a deep decision cannot occur with
            // escalationPermitted false — and a state that cannot be reached
            // is refused, not faked.
            if decision.tier == .deep {
                throw BridgeError.generationFailed(
                    "escalated tier is not available in this build"
                )
            }
        } catch let error as ChatTurnError {
            if case let .conversationDoesNotFit(shortfall) = error {
                throw BridgeError.conversationDoesNotFit(shortfall: shortfall)
            }
            throw BridgeError.generationFailed(String(describing: error))
        }

        let options = GenerationOptions(
            samplingMode: .greedy, maximumResponseTokens: cap
        )
        // THE TYPED CONTRACT (LL-04 §1.1): guided generation asks the model
        // for a GroundedAnswer, not prose — the cite-or-abstain shape is the
        // measured `guidedGeneration` capability, a SCHEMA rather than a
        // regex over the text. Deltas ride the partial `answer` exactly as
        // they rode the raw string before (suffix emit; wholesale-replace on
        // a revision); `abstain` and `claims` are read only from the SETTLED
        // value, never from a partial snapshot, so no reader ever sees a
        // half-decided abstention.
        var full = ""
        var settled: GeneratedContent?
        var inputTokens = 0
        var outputTokens = 0
        do {
            for try await snapshot in session.streamResponse(
                to: prompt, generating: GroundedAnswer.self, options: options
            ) {
                if Task.isCancelled { break }
                let content = snapshot.content.answer ?? ""
                if content.count > full.count {
                    // Snapshots are cumulative; emit only the suffix.
                    let delta = String(content.dropFirst(full.count))
                    full = content
                    out.send(["id": id, "event": "delta", "text": delta])
                } else if content != full {
                    // A revision (not append): replace wholesale, marked, so
                    // no reader reconstructs a Frankenstein from suffixes.
                    full = content
                    out.send([
                        "id": id, "event": "delta", "text": content,
                        "replace": true
                    ])
                }
                settled = snapshot.rawContent
                if #available(macOS 27.0, *) {
                    inputTokens = max(
                        inputTokens, snapshot.usage.input.totalTokenCount
                    )
                    outputTokens = max(
                        outputTokens, snapshot.usage.output.totalTokenCount
                    )
                }
            }
        } catch {
            throw BridgeError.generationFailed(String(describing: error))
        }

        if Task.isCancelled {
            // The reader is gone; the terminal event goes nowhere, but the
            // session stays usable — cancel kills the TURN, not the
            // conversation.
            return
        }

        // The COMPLETE typed answer decodes from the last snapshot's raw
        // content — the one place the framework guarantees every field is
        // settled. A stream that produced no content at all IS an
        // abstention: the model said nothing, and silence is never a
        // supported answer. A present-but-undecodable answer is a named
        // failure, not a prose fallback — falling back to the raw string
        // would re-open the unverified-citation hole this schema closes.
        let typed: GroundedAnswer
        if let settled {
            do {
                typed = try GroundedAnswer(settled)
            } catch {
                throw BridgeError.generationFailed(
                    "the typed answer did not decode: \(String(describing: error))"
                )
            }
        } else {
            typed = GroundedAnswer(abstain: true, answer: "", claims: [])
        }

        out.send([
            "id": id, "event": "final", "text": typed.answer,
            "abstain": typed.abstain,
            "claims": typed.claims.map { claim in
                ["text": claim.text, "cites": claim.cites] as [String: Any]
            },
            "inputTokens": inputTokens, "outputTokens": outputTokens,
            "tier": "on_device", "turn": turnNo,
            "escalationRefused": false
        ])
    }
    #endif
}
