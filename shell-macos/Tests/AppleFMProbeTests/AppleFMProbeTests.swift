import Testing
import Foundation

// The import must be at FILE SCOPE, not inside `#if canImport` around the
// use site. A conditional around a call does not bring the module's types
// into scope, and the build fails with `cannot find type 'Tool' in scope`
// rather than "module not available" — a misleading diagnostic that reads
// like a missing dependency when it is really an import placement.
#if canImport(FoundationModels)
import FoundationModels
#endif

@testable import AppleFMProbe

/// These are MEASUREMENT tests, not capability claims.
///
/// Each asserts a number the framework reports on the machine running them.
/// A pin that hardcoded 4096 would rot the first time Apple shipped a
/// different model, and a pin that asserted "available" would pass on a
/// build where `@available` had compiled the whole surface out of existence.
/// So each test asks the framework and then checks that what it said is
/// coherent.
@Suite("Apple FM probe")
struct AppleFMProbeTests {

    @Test("The on-device window is reported, and it is a combined budget")
    func onDeviceWindowIsMeasuredNotAssumed() {
        let caps = AppleFMProbe.capabilities()
        switch caps.onDevice {
        case let .available(contextSize, languages, variant, capabilities):
            #expect(contextSize > 0, "a reported window of zero is not a window")
            #expect(languages > 0, "a model reporting no languages is not usable")
            // Recorded, not asserted to a constant: the value moved from 4096
            // to something else across AFM generations, and a pin on the
            // number would break on hardware we have not seen yet.
            print("on-device window: \(contextSize) tokens; languages: \(languages)")
            // WHICH model answered is the question the window alone cannot
            // answer. A 4096 window is consistent with a 3B Core and with a
            // downgraded Core Advanced, and the two imply different routing.
            #expect(!variant.isEmpty, "an unnamed variant cannot be routed on")
            print("variant: \(variant); capabilities: \(capabilities)")
        case let .unavailable(reason):
            // Legitimate on non-Apple-silicon or pre-26 hosts. Say so rather
            // than failing, so the lane is honest about what it ran on.
            print("on-device model unavailable: \(reason)")
        }
    }

    @Test("Private Cloud eligibility is never reported as capability")
    func privateCloudEligibilityIsNotCapability() {
        let caps = AppleFMProbe.capabilities()
        switch caps.privateCloud {
        case .eligibleButUnentitled:
            // The exact trap, pinned. `isAvailable` returns true on a device
            // that is eligible, and the REQUEST then fails with
            // ModelManagerError 1046 for want of the app entitlement. A caller
            // that read the flag would ship a dead code path.
            #expect(
                caps.contextBudget != nil,
                "the on-device path must stand alone; PCC is never required"
            )
        case let .unavailable(reason):
            print("Private Cloud Compute unavailable: \(reason)")
        case _:
            break
        }
    }

    @Test("The tool schema cost is measurable, and small")
    func toolSchemaCostIsMeasured() async throws {
        #if canImport(FoundationModels)
        if #available(macOS 26.4, *) {
            // Measured, not assumed — and the model may be legitimately ABSENT.
            //
            // Found by the `xcode-27` lane on its first run: the image is macOS
            // 27 on Apple Silicon, so the framework compiles and links and
            // dyld-binds, and `SystemLanguageModel.default.availability` is
            // `.unavailable(deviceNotEligible)`. `tokenCount(for:)` does not
            // return zero there — it throws `ModelManagerError 1046`. That throw
            // is CORRECT (a measurement that cannot be taken must not report a
            // number), so the fix belongs here, not in `measure`: this test has
            // to skip the way its two siblings already do, rather than assume a
            // model exists because the framework compiled.
            //
            // The consequence is recorded honestly: this lane proves the toolchain
            // and the linkage, and the window/schema/latency numbers come from
            // eligible hardware only.
            guard case .available = SystemLanguageModel.default.availability else {
                print("on-device model unavailable — tool schema cost not measured here")
                return
            }

            // The transport is unreachable on this path: the test PRICES the
            // tool's schema and never invokes it. Supplied rather than
            // defaulted, because a tool constructible with no transport is a
            // tool that can still hand back the stub.
            let tools: [any Tool] = [MemoryRecallTool(transport: UnwiredRecallTransport())]
            let instructions = Instructions(
                "You are the memory assistant for a private knowledge kernel."
            )
            let cost = try await ContextBudget.measure(
                instructions: instructions, tools: tools
            )
            print("instructions \(cost.instructions) + tools \(cost.toolSchemas)")
            #expect(cost.toolSchemas > 0, "a tool that costs nothing was not measured")
            if let budget = cost.remaining {
                #expect(
                    budget > 0,
                    "the tool surface consumed the entire window"
                )
                print("remaining for conversation: \(budget) tokens")
            }
        }
        #endif
    }

    /// Latency, measured rather than guessed.
    ///
    /// Everything else in this probe reports a NUMBER THE FRAMEWORK GIVES US
    /// (a window, a language count, a schema cost). Latency is different: it is
    /// the product's feel, and it is the figure every plan in the private set
    /// had to estimate instead of measure. On an M1 Pro the memory-bandwidth
    /// arithmetic gave 12–18 tok/s for a 27B and a wide band for this model —
    /// arithmetic, not evidence.
    ///
    /// Three numbers are taken per turn:
    ///
    /// - **first token** — what the user waits for before anything appears
    /// - **sustained decode** — output tokens ÷ (total − first token). The
    ///   first-token time is EXCLUDED on purpose: folding prefill and session
    ///   setup into a decode rate produces a number that describes nothing.
    /// - **cached input tokens** — `Usage.Input.cachedTokenCount`, so we can
    ///   see the runtime's own prefix reuse rather than inferring it from a
    ///   suspiciously fast second turn.
    ///
    /// NO TIMING CONSTANT IS ASSERTED. This runs on shared CI runners under
    /// variable load; a pin of "TTFT < 1s" would be red on a loaded runner and
    /// green on an idle one, which is a pin that measures the runner. What is
    /// asserted is that the measurement is internally coherent — a turn that
    /// produced no tokens, or whose first token arrived after the turn ended,
    /// is a broken measurement rather than a slow machine. Both are worth a
    /// red build; neither is a latency claim.
    @Test("Latency: cold first token, warm turn, sustained decode")
    func latencyIsMeasured() async throws {
        #if canImport(FoundationModels)
        if #available(macOS 26.0, *) {
            let model = SystemLanguageModel.default
            guard case .available = model.availability else {
                print("on-device model unavailable — latency not measured here")
                return
            }

            // A SHORT, BOUNDED prompt. This runs in CI on a shared runner where
            // the model may download on first use; a long generation turns a
            // measurement into a timeout.
            //
            // The token ceiling is 256 rather than a token or two on purpose.
            // Decode RATE can only be measured over a window long enough that
            // session setup stops dominating it — at 48 tokens the setup is most
            // of the turn, and the rate it reports describes framework
            // overhead rather than the model.
            let probe = "Write a short paragraph about how memory should be verified."
            let options = GenerationOptions(
                samplingMode: .greedy, maximumResponseTokens: 256
            )

            let session = LanguageModelSession(model: model)

            // COLD — fresh session, nothing in any cache.
            let cold = try await Self.measure(session: session, prompt: probe, options: options)

            // WARM — same session, so the prefix is a candidate for reuse. The
            // prompt differs from `cold` only in its trailing line, which is
            // what makes this a cache probe rather than a repeat.
            let warm = try await Self.measure(
                session: session,
                prompt: probe + "\n\nAnd now in one sentence: why provenance matters.",
                options: options
            )

            print("")
            print("── AFM latency on this machine ──")
            print(Self.line("cold", turn: cold))
            print(Self.line("warm", turn: warm))
            print(
                "input cached: cold \(cold.cachedInput)/\(cold.inputTokens) · "
                    + "warm \(warm.cachedInput)/\(warm.inputTokens) tokens"
            )
            print("──────────────────────────────")

            // Coherence, not speed.
            #expect(cold.snapshots > 0, "a turn that streamed nothing was not measured")
            #expect(warm.snapshots > 0, "the warm turn streamed nothing")
            #expect(
                cold.outputTokens > 0,
                "a completed turn reported no output tokens"
            )
            #expect(
                cold.firstToken <= cold.total,
                "the first token cannot arrive after the turn ended"
            )
            #expect(
                cold.cachedInput <= cold.inputTokens,
                "cached tokens cannot exceed the input they are a subset of"
            )
        }
        #endif
    }
}

#if canImport(FoundationModels)
extension AppleFMProbeTests {
    fileprivate struct Turn {
        var firstToken: Double = 0
        var total: Double = 0
        var outputTokens: Int = 0
        var inputTokens: Int = 0
        var cachedInput: Int = 0
        var snapshots: Int = 0
    }

    @available(macOS 26.0, *)
    fileprivate static func measure(
        session: LanguageModelSession,
        prompt: String,
        options: GenerationOptions
    ) async throws -> Turn {
        var turn = Turn()
        let start = ContinuousClock.now

        for try await snapshot in session.streamResponse(to: prompt, options: options) {
            if turn.snapshots == 0 {
                turn.firstToken = Self.seconds(since: start)
            }
            turn.snapshots += 1
            if #available(macOS 27.0, *) {
                // Exact counts from the runtime, not estimated from text
                // length — which would conflate tokens with characters.
                //
                // MAX, not last: `usage` is cumulative per snapshot today, but
                // taking the last value silently reports "1 token" if a future
                // runtime makes it per-chunk. A max degrades to the same answer
                // under either semantics; a last does not.
                turn.outputTokens = max(turn.outputTokens, snapshot.usage.output.totalTokenCount)
                turn.inputTokens = max(turn.inputTokens, snapshot.usage.input.totalTokenCount)
                turn.cachedInput = max(turn.cachedInput, snapshot.usage.input.cachedTokenCount)
            }
        }

        turn.total = Self.seconds(since: start)
        return turn
    }

    fileprivate static func seconds(since start: ContinuousClock.Instant) -> Double {
        let components = (ContinuousClock.now - start).components
        return Double(components.seconds) + Double(components.attoseconds) / 1e18
    }

    fileprivate static func ms(_ seconds: Double) -> String {
        String(format: "%6.1f ms", seconds * 1000)
    }

    fileprivate static func line(_ label: String, turn: Turn) -> String {
        "  \(label)  first token \(ms(turn.firstToken)) · total \(ms(turn.total))"
            + " · decode \(rate(turn)) · \(turn.outputTokens) tok"
            + " in \(turn.snapshots) chunks"
    }

    /// Decode rate with prefill excluded — see the test's doc comment.
    fileprivate static func rate(_ turn: Turn) -> String {
        let decodeSeconds = turn.total - turn.firstToken
        guard turn.outputTokens > 0, decodeSeconds > 0 else { return "     n/a" }
        return String(format: "%6.1f tok/s", Double(turn.outputTokens) / decodeSeconds)
    }
}
#endif
// MULTI-TURN MEMORY ON AFM — the question plan 02 depends on.
//
// The window is 4096 tokens COMBINED, and the tool schema already spends 368
// (instructions 58 + tools 310), leaving 3728 for the conversation. So multi-turn
// is not merely a template question here: it is a BUDGET question. A chat that
// keeps history has ~3.7k tokens to work with, which is roughly 3-5 exchanges
// before something has to be dropped or summarized.
//
// The measurement that matters is not "did it not throw" — it is whether the
// value told to it in turn 1 SURVIVES to turn 2 while a COLD session, which
// never saw it, fails to produce it. A warm answer with a cold positive proves
// nothing; the model would be inventing to be agreeable.
@Test("Multi-turn: AFM carries a turn-1 value to turn 2, and a cold session cannot")
func multiTurnMemoryIsMeasuredNotAssumed() async throws {
    #if canImport(FoundationModels)
    guard #available(macOS 27.0, *) else { return }
    // The model may be legitimately ABSENT: on a macOS 27 image the
    // framework compiles and links and dyld-binds, but
    // `SystemLanguageModel.default.availability` is
    // `.unavailable(deviceNotEligible)` — and the REQUEST then throws
    // `ModelManagerError 1046` rather than returning zero. That throw is
    // CORRECT (a measurement that cannot be taken must not report a
    // number), so the fix belongs here, not in `measure`: this test has to
    // skip the way its three siblings already do, rather than assume a model
    // exists because the framework compiled.
    guard case .available = SystemLanguageModel.default.availability else {
        print("on-device model unavailable — multi-turn memory not measured here")
        return
    }
    let secret = "184/97"

    // Same surface the latency test above already drives successfully, which is
    // itself evidence that SESSION REUSE works on AFM: it measures a cold and a
    // warm turn on one `LanguageModelSession`.
    func ask(_ session: LanguageModelSession, _ p: String) async throws -> String {
        let r = try await session.respond(to: p)
        return r.content.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    let warm = LanguageModelSession(model: SystemLanguageModel.default)
    let t1 = try await ask(warm, "Remember this number for later: \(secret). Reply only 'ok'.")
    let t2 = try await ask(warm, "What was the number I asked you to remember? Numbers only.")

    // CONTROL: a fresh model instance that was never told the number. If it can
    // produce the value on its own, the warm arm proves nothing.
    let cold = LanguageModelSession(model: SystemLanguageModel.default)
    let c = try await ask(cold, "What was the number I asked you to remember? Numbers only.")

    let warmHas = t2.contains(secret)
    let coldHas = c.contains(secret)
    print("""
    ---
    AFM multi-turn ---
      turn 1: \(t1.prefix(80))
      turn 2: \(t2.prefix(80))
      cold  : \(c.prefix(80))
      warm contains the value: \(warmHas)   cold contains it: \(coldHas)
    ---
    """)
    #expect(!coldHas, "a session that was never told the value must not produce it — otherwise the warm arm is unfalsifiable")
    #expect(warmHas, "AFM did not carry the turn-1 value into turn 2")
    #endif
}

// M1 — PCC LATENCY. UNMEASURED when plan 19 was written.
//
// The on-device numbers above describe a LOCAL model: cold first token ~2.0 s,
// warm ~0.7 s, decode 43-56 tok/s. PCC is a NETWORK model. Plan 19 §3.1 says the
// routing policy escalates to it, and §2 names this as the measurement that
// decides WHETHER it escalates by default or selectively — so the number is not
// a nice-to-have, it is the input to a decision that is otherwise a guess
// wearing a citation.
//
// What this test deliberately does NOT do is report a number it did not get.
// The trap it exists to avoid is the one the banner experiment already fell
// into: a probe whose every arm threw printed "no difference" and "PASS". So
// the failure modes are separated and NAMED — device not eligible, eligible
// but the request refused, or measured — and only the last one prints latency.
// An unentitled machine prints the error, and the absence of a number is itself
// the recorded result.
@Test("PCC latency: cold and warm first token — or the named reason there is none")
func privateCloudLatencyIsMeasuredNotAssumed() async throws {
    #if canImport(FoundationModels)
    guard #available(macOS 27.0, *) else {
        print("PCC needs macOS 27 — latency not measured here")
        return
    }

    let pcc = PrivateCloudComputeLanguageModel()

    // ELIGIBILITY, kept apart from capability — the same separation
    // `privateCloudEligibilityIsNotCapability` pins, and the same reason this
    // test does not stop here. `isAvailable` is true on a machine that cannot
    // run the app; the request is what finds out.
    guard pcc.isAvailable else {
        print("PCC device not eligible — latency NOT MEASURED (no number was taken)")
        return
    }

    // TRAP 1, first half: this is `async throws`. The on-device window is a
    // plain property; PCC's is a remote answer about a server-side deployment.
    // Reading it synchronously against `SystemLanguageModel` compiles and fails
    // here — which is exactly what a Dynamic Profile that swaps tiers will do.
    //
    // It is TIMED because the obvious next claim is that `async` means
    // "costs a round trip". MEASURED: it does not — 0.0 ms on three runs,
    // served from a local cache, because the runtime already knows the window
    // before anyone asks. So trap 1 is a COMPILE-TIME asymmetry, not a latency
    // one, and a design that pays for it "because it is a network call" is
    // paying for a cost that was not there. The hypothesis was wrong; the
    // number is kept because being wrong here is the useful part.
    let windowStart = ContinuousClock.now
    let pccWindow: Int
    do {
        pccWindow = try await pcc.contextSize
    } catch {
        print("""
        PCC contextSize: VOID — \(error)
          The window could not be read, so no request was attempted. \
        Latency NOT MEASURED.
        """)
        return
    }
    print("")
    print("── PCC remote window read (TRAP 1, measured) ──")
    print("  window \(pccWindow) tokens in \(AppleFMProbeTests.ms(AppleFMProbeTests.seconds(since: windowStart)))")
    print("  on-device the same read is a local constant — 0 ms by definition")
    print("──────────────────────────────")

    // The window arriving is not the model answering. The REQUEST is the
    // measurement; everything before it is the framework answering a question
    // about itself.
    let options = GenerationOptions(samplingMode: .greedy, maximumResponseTokens: 256)
    let probe = "Write a short paragraph about how memory should be verified."

    // CORRECTION — this test previously carried the claim that there is NO
    // `LanguageModelSession(model: pcc)`, and it is FALSE. macOS 27 adds
    // `init(model: some LanguageModel = SystemLanguageModel.default,
    // dynamicInstructions:history:)`, and `PrivateCloudComputeLanguageModel`
    // conforms to `LanguageModel`, so PCC can be passed straight to it. Verified
    // by compiling that exact call against this SDK, after the claim had already
    // been written down and repeated.
    //
    // The profile path below is still the right construction here, but for
    // DIFFERENT reasons: `historyTransform` is a profile modifier, and switching
    // tier mid-session is what a Dynamic Profile does. Reaching PCC does not
    // require one.
    //
    // The DSL spelling is also not the obvious one. `DynamicProfileBuilder` is a
    // `@_functionBuilder` (the pre-`@resultBuilder` spelling) and its `$0`
    // shorthand does NOT compile in this toolchain — the body takes the
    // expression itself and chains from there. Recorded because the `$0` form
    // is the one every doc example uses, and the diagnostic it produces
    // ("anonymous closure argument not contained in a closure") reads like a
    // malformed closure rather than a wrong DSL.
    let session = LanguageModelSession(
        profile: LanguageModelSession.Profile {
            Instructions("You are the memory assistant for a private knowledge kernel.")
        }
        .model(pcc)
    )

    func attempt(_ label: String, _ prompt: String) async -> AppleFMProbeTests.Turn? {
        var turn = AppleFMProbeTests.Turn()
        let start = ContinuousClock.now
        do {
            for try await snapshot in session.streamResponse(to: prompt, options: options) {
                if turn.snapshots == 0 {
                    turn.firstToken = AppleFMProbeTests.seconds(since: start)
                }
                turn.snapshots += 1
                turn.outputTokens = max(turn.outputTokens, snapshot.usage.output.totalTokenCount)
                turn.inputTokens = max(turn.inputTokens, snapshot.usage.input.totalTokenCount)
                turn.cachedInput = max(turn.cachedInput, snapshot.usage.input.cachedTokenCount)
            }
        } catch {
            print("PCC \(label): VOID — \(error)")
            print("  PCC latency NOT MEASURED. Eligibility is not entitlement; this is that gap, measured.")
            return nil
        }
        turn.total = AppleFMProbeTests.seconds(since: start)
        return turn
    }

    guard let cold = await attempt("cold", probe) else { return }
    let warm = await attempt("warm", probe + "\n\nAnd now in one sentence: why provenance matters.")

    print("")
    print("── PCC latency on this machine ──")
    print("window: \(pccWindow) tokens (async remote read)")
    print(AppleFMProbeTests.line("cold", turn: cold))
    if let warm {
        print(AppleFMProbeTests.line("warm", turn: warm))
    } else {
        print("  warm  : VOID — the cold turn answered and the warm one did not")
    }
    print("──────────────────────────────")

    // The on-device comparison the routing decision actually turns on, printed
    // rather than compared here: this test asserts no timing constant, because
    // a red build on a loaded runner would measure the runner. It asserts the
    // measurement is coherent, which is a different and legitimate claim.
    #expect(cold.snapshots > 0, "PCC reported a turn that streamed nothing")
    #expect(cold.outputTokens > 0, "a completed PCC turn reported no output tokens")
    #expect(cold.firstToken <= cold.total, "the first token cannot arrive after the turn ended")
    #expect(cold.cachedInput <= cold.inputTokens, "cached tokens cannot exceed the input they are a subset of")
    if let warm {
        #expect(warm.snapshots > 0, "the PCC warm turn streamed nothing")
        #expect(warm.firstToken <= warm.total, "the PCC first token cannot arrive after the turn ended")
    }
    #endif
}

// M2 — IS THE BIG WINDOW ACTUALLY USABLE? ("lost in the middle")
//
// A declared window is not a working one. If attention degrades with position,
// a 32,768-token window can be WORSE than a 4,096-token one, and the escalation
// target plan 19 names would be wrong. The check is a needle at known depth:
// bury a fabricated fact at ~25%, ~50% and ~75%, and require all three back.
//
// WHAT THIS RUNS AGAINST, AND WHY THAT IS THE POINT. The 24k prompt the plan
// specifies only fits PCC, and M1 just measured that PCC refuses the request on
// this machine (ModelManagerError 1046, the app entitlement). So M2's literal
// question is UNANSWERABLE here — and saying so is part of the result, because
// the alternative is a plan that keeps citing a number nobody has taken.
//
// So this measures the same PHENOMENON on the window this machine actually has:
// the needle at three depths of a real on-device prompt. That is smaller than
// the plan asked for and it answers a narrower question — does this model show
// depth-dependent recall degradation AT ALL, and at what scale — which is the
// precondition for the 32k claim being worth anything. A model that loses the
// middle at 2.4k is not a model to escalate to at 32k.
//
// Every arm carries a COLD control, and the control differs from the warm arm
// in EXACTLY ONE WAY: the needle sentence is absent. Same document, same
// filler, same question, same token scale.
//
// The first version built the "cold" session on the SAME document and it
// returned the needle verbatim, 3 of 3 — which reads like a catastrophic recall
// result and was in fact a broken control, because a session handed the
// document has seen the needle. A control that does not differ from the
// treatment only ever measures the treatment. Building it as "the document
// minus the one fact under test" is what makes a hit mean recall rather than
// compliance.
//
// A small model asked "what is the vault access code" and told nothing is
// agreeable; without the cold arm a hit proves nothing and a miss is ambiguous
// between "forgot" and "refused to guess".
//
// Depths are MEASURED with `tokenCount(for:)`, not assumed from character
// count — "25% of the prompt" in tokens is the claim, and reporting the
// character fraction as if it were the token fraction would make a
// depth-excuse available for any failure.
@Test("Needle: recall at three depths, each against a cold control")
func needleSurvivesAtThreeDepths() async throws {
    #if canImport(FoundationModels)
    guard #available(macOS 26.4, *) else {
        print("tokenCount needs macOS 26.4 — needle depths not measured here")
        return
    }
    let model = SystemLanguageModel.default
    guard case .available = model.availability else {
        print("on-device model unavailable — needle depths not measured here")
        return
    }

    // Sized to the MEASURED conversation budget (3,728 tokens after instructions
    // and one tool schema), not to a round number. Going past it would measure
    // context-window-exceeded rather than recall, which is the other test.
    let targetTokens = 2400

    struct Arm {
        let label: String
        let depth: Double
        let value: String
    }

    // Three DISTINCT values, one per arm. A single reused value would let one
    // lucky guess satisfy arms it never saw — and "all three back" only means
    // three separate recalls if the targets differ.
    let arms = [
        Arm(label: "25%", depth: 0.25, value: "kestrel-471"),
        Arm(label: "50%", depth: 0.50, value: "marram-802"),
        Arm(label: "75%", depth: 0.75, value: "gannet-336"),
    ]

    print("")
    print("── Needle at depth (on-device window; PCC refused, see M1) ──")

    var results: [(Arm, Bool, Bool, Double)] = []
    var retriedArms: [String] = []

    for arm in arms {
        // MEASURE the depth: build the document, count its tokens, and place the
        // needle where the measured prefix says, not where the character count
        // would put it.
        let (document, controlDocument, measuredDepth) = try await AppleFMProbeTests.needleDocument(
            model: model,
            target: arm.value,
            targetTokens: targetTokens,
            fraction: arm.depth
        )
        // `needleDocument` returns the MEASURED fraction it achieved; if it could
        // not get near the requested depth it says so rather than reporting a
        // depth it did not hit.
        results.append((arm, false, false, measuredDepth))

        let question = "\n\nQuestion: what is the vault access code recorded above? "
            + "Reply with the code only."
        // GREEDY. The default sampler made this test flaky — a miss at a depth
        // the next run recovered at the same measured 24.9%. A pin that is red
        // one run in five teaches its reader to re-run it, which is worse than
        // having no pin.
        let options = GenerationOptions(samplingMode: .greedy, maximumResponseTokens: 32)
        func ask(_ session: LanguageModelSession, _ body: String) async throws -> String {
            let r = try await session.respond(to: body + question, options: options)
            return r.content
        }

        // Attempts are counted and REPORTED, never hidden. Marginal recall is a
        // finding: "needed 2" says this model is not comfortable at this depth,
        // which a bare pass would not. What must not happen is a coin flip
        // turning the build red, so an arm gets a small retry budget and has to
        // exhaust it to fail.
        //
        // THE BUDGET IS 3 BECAUSE TWO WAS NOT ENOUGH — but note the regime that
        // observation belongs to: it was taken under the DEFAULT sampler, where
        // this test was flaky roughly one run in five. Under greedy sampling,
        // five consecutive characterisation runs had every arm clean on the
        // FIRST attempt, so nothing has needed a second attempt since. Three is
        // headroom, not a measured requirement; if this ever goes red, the
        // printed attempt counts say which regime you are back in.
        var needed = 0
        var warmHas = false
        for attempt in 1...3 {
            let warm = LanguageModelSession(model: model)
            let answer = try await ask(warm, document)
            needed = attempt
            warmHas = answer.contains(arm.value)
            if warmHas { break }
        }
        retriedArms.append("\(arm.label):\(needed)")

        // COLD: the same document with the needle line removed. Identical in
        // every other respect. If it produces the value anyway, the warm hit is
        // unfalsifiable and the whole measurement is void.
        let cold = LanguageModelSession(model: model)
        let coldAnswer = try await ask(cold, controlDocument)

        let coldHas = coldAnswer.contains(arm.value)
        results[results.count - 1] = (arm, warmHas, coldHas, measuredDepth)
        print(String(
            format: "  target %@  measured %5.1f%%  warm %@  cold %@",
            arm.label, measuredDepth * 100,
            warmHas ? (needed > 1 ? "HIT(\(needed))" : "HIT ") : "MISS(\(needed))",
            coldHas ? "HIT(!)" : "miss"
        ))
        // The cold answer is PRINTED on a leak, because "the control failed" and
        // "the control failed because the needle is guessable" are different
        // findings and only one of them is actionable.
        if coldHas {
            print("    cold said: \(coldAnswer.trimmingCharacters(in: .whitespacesAndNewlines).prefix(120))")
        }
    }

    let hits = results.filter(\.1).count
    let contaminated = results.filter(\.2).count
    print("  \(hits) of \(results.count) needles recovered; \(contaminated) cold control(s) leaked")
    print("  attempts per arm (1 = clean first try): \(retriedArms.joined(separator: " "))")
    if hits == results.count {
        print("  => no depth-dependent recall loss at ~\(targetTokens) tokens on this model.")
        print("  => This does NOT license a 32k claim: the 24k+ prompt the plan asks")
        print("     about did not run here, because PCC refused the request (M1).")
    }
    print("──────────────────────────────")

    // A cold positive voids the arm it contaminates, so it is asserted rather
    // than reported: a model that invents the code makes every hit meaningless.
    #expect(
        contaminated == 0,
        "a session that never saw the document produced a needle — every warm hit in this run is unfalsifiable"
    )
    // All three, and the failure message NAMES THE DEPTHS that were missed,
    // because "2 of 3" alone sends the reader back to the source to find out
    // which — and the depth IS the finding.
    let missed = results.filter { !$0.1 }.map { "\($0.0.label)" }.joined(separator: ", ")
    #expect(
        hits == results.count,
        "the needle was not recovered at: \(missed.isEmpty ? "n/a" : missed)"
    )
    #endif
}

#if canImport(FoundationModels)
@available(macOS 26.4, *)
extension AppleFMProbeTests {
    /// A document of roughly `targetTokens` with `needle` buried at `fraction`
    /// of its MEASURED token length. Returns the ACHIEVED fraction, not the
    /// requested one — the caller reports the achieved value, because a test
    /// that prints the depth it asked for is printing its own intent.
    ///
    /// Paragraphs are counted as they are appended, so the insertion point is a
    /// TOKEN index resolved to a PARAGRAPH index. Treating one as the other
    /// would put the needle at paragraph `fraction` of the way through, which
    /// for this filler is a completely different depth and would still print a
    /// confident number.
    fileprivate static func needleDocument(
        model: SystemLanguageModel,
        target: String,
        targetTokens: Int,
        fraction: Double
    ) async throws -> (document: String, control: String, measuredDepth: Double) {
        // Plain, varied filler. Deliberately NOT random: a seeded PRNG would be
        // one more moving part, and a repeating word list is enough to defeat
        // prefix compression as long as the ORDER varies.
        let vocabulary = [
            "harbour", "lantern", "archive", "sediment", "copper", "willow",
            "granite", "threshold", "ledger", "estuary", "meridian", "scaffold",
            "cobalt", "parcel", "trellis", "quarry", "bracket", "cadence", "filament",
        ]

        func paragraph(_ seed: Int) -> String {
            var line: [String] = []
            // A stride coprime with the vocabulary length walks every word
            // before repeating, so the document has no short repeating period
            // for the runtime to compress away.
            for i in 0..<24 {
                line.append(vocabulary[(seed * 7 + i * 5) % vocabulary.count])
            }
            return line.joined(separator: " ") + "."
        }

        var blocks: [String] = []
        var running = 0
        var index = 0
        while running < targetTokens, index < 200 {
            let next = blocks + [paragraph(index)]
            let text = next.joined(separator: "\n\n")
            let count = try await model.tokenCount(for: text)
            blocks = next
            running = count
            index += 1
        }

        let needle = needleSentence(target)
        let needleCost = try await model.tokenCount(for: needle)
        let bodyTokens = max(running - needleCost, 1)
        let wantedTokenIndex = Int(Double(bodyTokens) * fraction)

        // Walk the blocks accumulating tokens until the wanted index is crossed.
        // A needle placed at the paragraph boundary is within one paragraph of
        // the target, and the MEASURED depth printed below is the real one.
        var placed = 0
        var accumulated = 0
        for i in blocks.indices {
            accumulated = try await model.tokenCount(for: blocks[0...i].joined(separator: "\n\n"))
            if accumulated >= wantedTokenIndex {
                placed = i
                break
            }
            placed = i + 1
        }
        blocks.insert(needle, at: min(placed, blocks.count))

        let document = blocks.joined(separator: "\n\n")
        // The control is the SAME document with the needle line removed — not a
        // shorter document and not an empty one. Anything else changes a second
        // variable and the comparison stops being about recall.
        let control = blocks.enumerated()
            .filter { $0.offset != placed }
            .map(\.element)
            .joined(separator: "\n\n")
        let totalTokens = try await model.tokenCount(for: document)
        let prefixTokens = try await model.tokenCount(
            for: blocks[0...placed].joined(separator: "\n\n")
        )
        return (document, control, Double(prefixTokens) / Double(max(totalTokens, 1)))
    }

    /// The token cost of one transcript entry.
    ///
    /// The SDK has a real overload for this —
    /// `tokenCount(for transcriptEntries: some Collection<Transcript.Entry>)` —
    /// so this counts the entry itself rather than a RECONSTRUCTION of it.
    ///
    /// An earlier version of this helper rebuilt each entry's text by hand and
    /// fed that to `tokenCount(for: some PromptRepresentable)`, on the stated
    /// grounds that entries are not `PromptRepresentable`. The premise was half
    /// right — a single `Transcript.Entry` is not — and the conclusion was wrong:
    /// the overload takes a COLLECTION of entries, and it existed the whole time.
    ///
    /// The reconstruction was not merely inelegant, it was WRONG IN THE
    /// DANGEROUS DIRECTION: it rendered `.toolCalls` as `"tool " + names`,
    /// omitting the arguments JSON. The tool traffic is exactly what the trim
    /// policy drops first as "the bulkiest entries", so the undercount landed
    /// squarely on the content the budget most needed to weigh. Measuring the
    /// entry removes the class of bug rather than this instance of it.
    fileprivate static func cost(
        of entry: Transcript.Entry,
        model: SystemLanguageModel
    ) async -> Int {
        let counted = try? await model.tokenCount(for: [entry])
        return counted ?? 0
    }

    fileprivate static func totalCost(
        of entries: [Transcript.Entry],
        model: SystemLanguageModel
    ) async -> Int {
        var total = 0
        for entry in entries {
            total += await cost(of: entry, model: model)
        }
        return total
    }

    fileprivate static func needleSentence(_ value: String) -> String {
        "Record \(value): the vault access code for the north store, effective immediately."
    }
}
#endif

// HISTORY PRESSURE — trims, never rewrites.
//
// The window is 4,096 combined and the measured conversation budget is 3,728.
// A chat session passes that in a handful of turns, so this is the failure the
// shell WILL hit, not a theoretical one.
//
// The rule being pinned is Apple's, quoted in `HistoryBudget`: appending
// preserves the key-value cache, mutation invalidates it, and rewriting can
// confuse the model. So the budget is spent by trimming a COPY per request
// (`historyTransform`), and the session's own transcript is append-only.
//
// WHY A LENGTH ASSERTION ALONE IS NOT ENOUGH. Both a trimmer and a rewriter
// make the transcript shorter, so "the transcript got smaller" is satisfied by
// the implementation the prompt exists to rule out. The discriminator is
// DIRECTION: a trimmer never changes what the session holds, a rewriter does.
// So this asserts the session's transcript is monotonically NON-DECREASING
// across turns that blow the budget, while the turn still succeeds — which is
// what "trims and continues" actually means.
@Test("History pressure: the session's own transcript is never rewritten")
func historyPressureTrimsAndNeverRewrites() async throws {
    #if canImport(FoundationModels)
    // 27.0, not 26.4: `historyTransform` is a Dynamic Profile modifier, and
    // the profile path — not the availability of the model — is what this pin
    // is about. `LanguageModelSession(model:)` has no transform overload at all.
    guard #available(macOS 27.0, *) else {
        print("historyTransform needs macOS 27 — history pressure not driven here")
        return
    }
    let model = SystemLanguageModel.default
    guard case .available = model.availability else {
        print("on-device model unavailable — history pressure not driven here")
        return
    }

    // The budget is measured, not assumed — same seam the router uses.
    guard let budget = await ContextBudget.measureOnDeviceDefaults(), budget > 0 else {
        print("on-device budget not measurable here — history pressure not driven")
        return
    }
    print("")
    print("── History pressure (budget \(budget) tokens) ──")

    // The transform under test, wired the way the shell will wire it: on the
    // Dynamic Profile, because `historyTransform` is a PROFILE modifier and
    // there is no `LanguageModelSession(model:…)` overload that takes one.
    // (It is NOT the same fact that makes PCC reachable only through a profile
    // — that claim is retracted above and is false. The profile is still used
    // here because `historyTransform` is a profile modifier.)
    // Async measurement, synchronous application — the split the framework's
    // signature forces. The controller re-plans before each turn; the closure
    // it hands back is what `historyTransform` calls per request, and it only
    // filters. The pinned property is that `apply` returns a SUBSET and never
    // mutates: the session keeps everything.
    let controller = HistoryBudget.Controller(budgetTokens: budget)
    let session = LanguageModelSession(
        profile: LanguageModelSession.Profile {
            Instructions("Reply with one short sentence.")
        }
        .model(model)
        .historyTransform(controller.transform)
    )

    // Filler sized so the conversation passes the measured budget in a few
    // turns without needing so many turns that the suite becomes the problem.
    // Sized so FIVE turns blow the measured 3,728-token budget on their own.
    // The earlier version sent ~655 tokens in total and printed "budget not
    // exceeded in this run" — a green test that had demonstrated nothing about
    // pressure at all, which is worse than a red one because it looks like
    // coverage. The pressure is now a precondition of the test, asserted below.
    let filler = String(repeating: "context ", count: 1200)
    var historyCounts: [Int] = []
    var transcriptIds: [[Transcript.Entry.ID]] = []
    var tokensSeen = 0
    var trimmedCounts: [Int] = []
    var seenTokens: [Int] = []

    for turn in 1...5 {
        let prompt = "\(filler)\nTurn \(turn): name one word."
        // Charge the prompt we are ABOUT to send against the budget before the
        // turn, not after it fails.
        let promptCost = try await model.tokenCount(for: prompt)
        await controller.refresh(
            Array(session.transcript), incomingTokens: promptCost
        ) { entry in
            await AppleFMProbeTests.cost(of: entry, model: model)
        }
        let response = try await session.respond(to: prompt)
        tokensSeen += promptCost
        historyCounts.append(session.transcript.count)
        transcriptIds.append(Array(session.transcript).map(\.id))
        // What the REQUEST saw, as opposed to what the session holds. The two
        // diverging is the whole point: the model gets less, the session keeps
        // everything.
        // The request view, measured the way the framework would see it: the
        // plan applied to the history PLUS the turn's own pending prompt.
        //
        // It used to be applied to `session.transcript` AFTER the turn, which
        // quietly included the response that had just been generated — content
        // the pre-turn plan was never sized against, so the sum could exceed the
        // budget for a reason that has nothing to do with whether the trim fits.
        // That measurement started failing once the eligible range moved to
        // "before the last prompt", which is when it stopped being able to hide
        // its own flaw.
        let requestEntries = Array(session.transcript) + [
            .prompt(Transcript.Prompt(
                segments: [.text(Transcript.TextSegment(content: prompt))]
            )),
        ]
        let requestPlan = await HistoryBudget.plan(
            requestEntries,
            budgetTokens: budget - promptCost,
            cost: { entry in await AppleFMProbeTests.cost(of: entry, model: model) },
        )
        let seen = requestPlan.apply(requestEntries)
        trimmedCounts.append(seen.count)
        seenTokens.append(await AppleFMProbeTests.totalCost(of: seen, model: model))

        // The conversation must SURVIVE the budget. A session that trims badly
        // does not fail gracefully — it throws `contextSizeExceeded`, and the
        // user sees a dead chat.
        #expect(
            !response.content.isEmpty,
            "turn \(turn) returned no text — the trim starved the conversation"
        )
    }

    // THE DISCRIMINATOR, and it is ids rather than a LENGTH — because the
    // length check was tried first and it does not work.
    //
    // A rewrite drill (mutate `session.transcript` in place to fit the budget)
    // produced [3, 5, 7, 8, 8]: still monotonically NON-DECREASING, because a
    // rewrite that drops two entries is immediately followed by the framework
    // appending the next prompt and response. The count never shrinks, so the
    // length assertion passed on exactly the implementation this pin exists to
    // reject. A naive length check is decorative; it was removed.
    //
    // The invariant that does distinguish them is APPEND-ONLY ON IDENTITY: every
    // entry the session has ever held must still be there. A trimmer drops
    // nothing, so the id set only ever grows. A rewriter drops the OLDEST
    // entry, and that id never comes back.
    // IDENTITY, and this is the assertion that has teeth. It rests on a contract
    // Apple documents for `Identifiable` on transcript entries but does NOT
    // spell out for THIS type: that an entry's id is stable across requests.
    // Nothing in the SDK says so. The only stability statement the framework
    // makes is about `GenerationID` — stable for the duration of a response,
    // explicitly NOT across responses — which is a caution, not a permission.
    //
    // So the trimmer's correctness rests on an undocumented property, and the
    // honest thing is to PIN it rather than assume it. If ids are not stable,
    // `dropped` silently misses, the trim does nothing, and the session starts
    // throwing `contextSizeExceeded` under load — a failure whose cause is two
    // layers away from its symptom.
    //
    // The rewrite drill is caught by the same data: a rewrite drops an entry the
    // session already recorded, and that id never comes back.
    var seenIds: Set<Transcript.Entry.ID> = []
    for (index, count) in historyCounts.enumerated() {
        let currentIds = Set(transcriptIds[index])
        let lost = seenIds.subtracting(currentIds)
        #expect(
            lost.isEmpty,
            """
            turn \(index + 1): \(lost.count) entry id(s) the session had already \
            recorded are GONE (\(currentIds.count) now, was \
            \(seenIds.count + lost.count)). Either the transcript was rewritten, \
            or Transcript.Entry.id is not stable across requests — and the \
            trimmer's drop-set depends on it being stable.
            """
        )
        seenIds.formUnion(currentIds)
        #expect(
            count == currentIds.count,
            "turn \(index + 1) counted \(count) entries but saw \(currentIds.count) distinct ids — ids are colliding"
        )
    }
    #expect(
        historyCounts.last ?? 0 > 0,
        "the session recorded no history at all, so nothing was actually trimmed"
    )
    print("  session transcript across turns: \(historyCounts)")
    print("  what each request saw:          \(trimmedCounts)")
    print("  tokens the session accumulated: \(tokensSeen) against a \(budget)-token budget")
    print("  tokens each request actually saw: \(seenTokens)")
    // Anti-vacuity: a run that never reached the budget has not tested
    // pressure, so the run is VOID rather than green.
    #expect(
        tokensSeen > budget,
        "the conversation never exceeded the \(budget)-token budget — this run proved nothing about pressure"
    )
    #expect(
        seenTokens.allSatisfy { $0 <= budget },
        "a request saw more than the budget allows — the trim did not fit"
    )
    print("  budget EXCEEDED and the conversation continued")
    print("──────────────────────────────")
    #endif
}

// THE WATERMARK IS A POLICY ASSERTION, and it cannot be pinned behaviourally
// here — so it is pinned on the pure function instead.
//
// Apple's key-value-caching article: *"Defer removing entries from the
// transcript until the context window is nearly full, then consolidate the
// context in a single operation rather than trimming incrementally after each
// turn."*
//
// Source: <https://developer.apple.com/documentation/foundationmodels/optimizing-key-value-caching-in-language-model-sessions>
// Trimming to the MINIMUM every turn — which the first version of
// `HistoryBudget` did, and which its own comment called "the point" — changes
// the kept set on every request and re-prefills the whole history each time.
//
// That cost is not observable in this environment: `usage.input.cachedTokenCount`
// does NOT reliably report cache reuse on this build — it has been observed at
// both 0 and 132/164 for the same shape of warm second turn, on the same
// machine. An intermittent counter is worse than a dead one for a pin: a dead
// one fails obviously, an intermittent one produces a test whose verdict
// depends on when it ran. So the defensible claim is "no RELIABLE signal", and
// nothing here asserts on cache reuse. Asserting TTFT instead would be
// measuring the runner.
//
// So the property is pinned where it IS decidable — on `plan`, which is a pure
// function of (entries, costs, budget). The claim is "a trim leaves headroom, so
// several turns can append onto a stable prefix", and a trim-to-the-minimum
// implementation violates it while still passing every behavioural pin above.
// Drilled: setting the watermark to 1.0 leaves all of them green.
@Test("A trim consolidates ONCE to a watermark, so appends land on a stable prefix")
func trimConsolidatesOnceRatherThanEveryTurn() {
    #if canImport(FoundationModels)
    guard #available(macOS 27.0, *) else { return }

    func promptEntry(_ text: String) -> Transcript.Entry {
        .prompt(Transcript.Prompt(segments: [.text(Transcript.TextSegment(content: text))]))
    }
    func responseEntry(_ text: String) -> Transcript.Entry {
        .response(Transcript.Response(segments: [.text(Transcript.TextSegment(content: text))]))
    }

    // Simulated conversation: one exchange costs 220 tokens against a 900-token
    // budget, and every turn appends one more exchange.
    let budget = 900
    var entries: [Transcript.Entry] = []
    var costs: [Int] = []
    for i in 0..<12 {
        entries.append(promptEntry("q\(i)")); costs.append(200)
        entries.append(responseEntry("a\(i)")); costs.append(20)
    }

    // UNDER budget, nothing is touched at all — "nearly full" means nearly full.
    let under = HistoryBudget.plan(entries, costs: costs, budgetTokens: budget * 4)
    #expect(
        under.droppedTokens == 0,
        "a conversation inside its window had entries dropped"
    )

    // THE PROPERTY: consolidation CADENCE.
    //
    // Two earlier instruments were tried and both were wrong, which is worth
    // recording because each looked reasonable:
    //
    //  - "how many turns needed a trim" answered 10 of 10. The session
    //    transcript is APPEND-ONLY and grows without bound, so it is over
    //    budget on every turn forever. Correct behaviour, wrong instrument.
    //  - "how many distinct kept-sets" also answered 10. The newest exchange is
    //    always in the kept set and is always new, so consecutive sets can
    //    never be equal.
    //
    // What Apple actually describes is the RATE of consolidation events:
    // trimming every turn invalidates the cache every turn, whereas trimming
    // once to a watermark leaves the prefix alone until it genuinely fills.
    // So the model is: the request carries `carried` tokens; each turn appends;
    // when the request WOULD overflow, consolidate once and carry the smaller
    // number forward.
    // Small exchanges, so there is room for the watermark to buy several turns
    // of stability. With 220-token exchanges against a 900-token budget the
    // headroom is barely one turn and the two policies cannot be told apart.
    let cadenceTurn = 60
    var carried = 5 * cadenceTurn
    var simEntries: [Transcript.Entry] = []
    var simCosts: [Int] = []
    for i in 0..<5 {
        simEntries.append(promptEntry("q\(i)")); simCosts.append(cadenceTurn / 3)
        simEntries.append(responseEntry("a\(i)")); simCosts.append(cadenceTurn - cadenceTurn / 3)
    }
    #expect(carried <= budget, "the fixture must start inside the window")

    var consolidations = 0
    for turn in 0..<20 {
        simEntries.append(promptEntry("new q\(turn)")); simCosts.append(cadenceTurn / 3)
        simEntries.append(responseEntry("new a\(turn)")); simCosts.append(cadenceTurn - cadenceTurn / 3)
        carried += cadenceTurn
        guard carried > budget else { continue }
        let plan = HistoryBudget.plan(simEntries, costs: simCosts, budgetTokens: budget)
        #expect(plan.keptTokens <= budget, "turn \(turn): consolidation did not fit the budget")
        carried = plan.keptTokens
        consolidations += 1
    }
    #expect(consolidations > 0, "the fixture never needed consolidation — it proves nothing")
    #expect(
        consolidations <= 3,
        """
        consolidated \(consolidations) times in 20 turns — that is trimming \
        incrementally, which is the behaviour Apple says to avoid. Cut to the \
        watermark so the prefix holds still between consolidations.
        """
    )

    // And the suffix after the latest response is never touched, which is what
    // makes "the floor is the last exchange" real rather than aspirational.
    let plan = HistoryBudget.plan(entries, costs: costs, budgetTokens: budget)
    let kept = plan.apply(entries)
    #expect(kept.count >= HistoryBudget.keepAtLeast, "the trim left fewer entries than the last exchange")
    #expect(
        kept.last?.id == entries.last?.id,
        "the final entry was dropped — the in-flight exchange must survive intact"
    )
    #endif
}

// WHICH END SURVIVES A TRIM. This is the pin the `.reversed()` inversion
// needed and did not have.
//
// The trimmer's loop said "oldest-first" while walking `.reversed()`, which
// drops the NEWEST eligible entries first and therefore keeps the oldest. A
// drill with six equal-cost exchanges — `q0 a0 … q5 a5` — under a forcing budget
// returned:
//
//     KEPT: q0 a0 a5          <- the opening exchange, and a5 orphaned from q5
//
// Under real pressure the model would recall the first thing anyone said and
// forget everything the user just said — the exact inversion of the policy the
// comment above the loop describes.
//
// TWO REASONS NOTHING CAUGHT IT, both worth knowing:
//
//   - The cadence and budget assertions are TOKEN COUNTS. With equal-cost
//     entries, dropping the oldest three and the newest three produce the same
//     number. A token-count assertion is blind to direction by construction.
//   - The existing suffix check was `kept.last?.id == entries.last?.id`, which
//     `a5` alone satisfies. The trailing response was present; its PROMPT was
//     not.
//
// So the checks are POSITIONAL: the kept entries inside the eligible range must
// form a SUFFIX of it, and no kept response may have lost the prompt that asked
// it. The second was added after the first turned out to be insufficient —
// protecting from the last `.response` rather than the last `.prompt` yields
// `a4 q5 a5`, which satisfies every suffix assertion while orphaning `a4`.
//
// ## WHAT THIS PIN DOES NOT CLAIM
//
// It pins the direction, and the direction is a PRODUCT CHOICE, not an
// Apple-recommended one. Apple says the opposite end is cheaper — "removing
// only the most recent entries is cheaper than modifying earlier ones because
// it invalidates fewer cached values" — because "each cached value in the
// sequence depends on every token that precedes it", so dropping from the front
// invalidates the whole retained history while dropping from the back does not.
//
// Keeping recent turns is right for a conversation and wrong for the cache, and
// Apple pairs that direction with a compensation this package has NOT built:
// "If you need to remove older entries, consider summarizing the conversation
// and starting a fresh session with that summary as context."
//
// So this pin guarantees the trimmer keeps a coherent, non-orphaned SUFFIX of
// the conversation. It does not make that cheap, it does not measure the cost —
// `cachedTokenCount` is unreliable on this build — and it must not be read as
// Apple endorsing the direction. Source:
// <https://developer.apple.com/documentation/foundationmodels/optimizing-key-value-caching-in-language-model-sessions>
//
// A THIRD gap remained after both, and the committed exchange-wise fix closed
// it: the fixture below ends in a response, so the in-flight pair is protected
// by construction, and the orphan rule only ever saw cuts that landed on
// exchange boundaries. The runtime shape — `historyTransform` is handed the
// history WITH the pending prompt, so the transcript ends in a `.prompt` —
// orphaned `a4` under budgets 600–700 while passing every assertion here. The
// pending-prompt arm covers it, and the budget for that arm comes from a sweep
// rather than a guess, because a single lucky budget is how its first version
// passed on the broken code.
@Test("A trim drops the OLDEST history and keeps the most recent exchange whole")
func trimKeepsTheMostRecentAndDropsOldestFirst() {
    #if canImport(FoundationModels)
    guard #available(macOS 27.0, *) else { return }

    func prompt(_ t: String) -> Transcript.Entry {
        .prompt(Transcript.Prompt(segments: [.text(Transcript.TextSegment(content: t))]))
    }
    func response(_ t: String) -> Transcript.Entry {
        .response(Transcript.Response(segments: [.text(Transcript.TextSegment(content: t))]))
    }

    // EQUAL costs on purpose: with unequal costs a token total can hide which
    // end was dropped, which is how the bug survived.
    var entries: [Transcript.Entry] = []
    var costs: [Int] = []
    for i in 0..<6 {
        entries.append(prompt("q\(i)")); costs.append(100)
        entries.append(response("a\(i)")); costs.append(100)
    }

    let plan = HistoryBudget.plan(entries, costs: costs, budgetTokens: 400)
    let kept = plan.apply(entries)
    let keptIds = Set(kept.map(\.id))

    func text(_ e: Transcript.Entry) -> String {
        func flat(_ s: [Transcript.Segment]) -> String {
            s.compactMap { seg -> String? in
                if case let .text(t) = seg { return t.content }
                return nil
            }.joined()
        }
        switch e {
        case let .prompt(p): return flat(p.segments)
        case let .response(r): return flat(r.segments)
        default: return "?"
        }
    }

    // The eligible range is everything before the latest prompt.
    let promptStart = entries.lastIndex(where: { HistoryBudget.isPrompt($0) }) ?? 0
    let eligibleIdx = Set(0..<promptStart)
    let keptEligible = eligibleIdx.filter { keptIds.contains(entries[$0].id) }
    let isSuffix = keptEligible == Set((keptEligible.min() ?? 0)..<promptStart)
    print("KEPT: \(kept.map(text).joined(separator: " "))")

    #expect(
        isSuffix,
        "a trim kept \(keptEligible.count) eligible entries that are NOT a suffix of the eligible range — it dropped the wrong end of the conversation"
    )
    let lastKept = kept.suffix(2).map(text)
    #expect(
        lastKept == ["q5", "a5"],
        "the final exchange did not survive whole — kept \(lastKept), expected q5 then a5"
    )
    #expect(
        !kept.contains { text($0) == "q0" || text($0) == "a0" },
        "the oldest exchange survived a trim that should have dropped it"
    )

    // THE ORPHAN RULE: no kept `.response` may have lost the `.prompt` that
    // asked it. This is the general form of "the exchange survives whole" and
    // it covers the in-flight exchange and every older one.
    func orphans(in entries: [Transcript.Entry], kept keptIds: Set<Transcript.Entry.ID>) -> [String] {
        var orphaned: [String] = []
        for (i, entry) in entries.enumerated() {
            guard case .response = entry, keptIds.contains(entry.id), i > 0 else { continue }
            let previous = entries[i - 1]
            if case .prompt = previous, !keptIds.contains(previous.id) {
                orphaned.append(text(entry))
            }
        }
        return orphaned
    }
    let orphaned = orphans(in: entries, kept: keptIds)
    #expect(
        orphaned.isEmpty,
        "kept \(orphaned.count) response(s) whose prompt was dropped — the model is reading answers to questions it cannot see: \(orphaned.joined(separator: "; "))"
    )

    // THE RUNTIME SHAPE, which the arm above never builds. `historyTransform`
    // is handed the history WITH the pending prompt, so the transcript ends in
    // a `.prompt` and the exchange just before it is eligible. The arm above
    // ends in a response, so its in-flight pair is protected by construction
    // and the orphan rule only ever sees cuts that happen to land on exchange
    // boundaries — 400 is an even number of 100-token entries.
    //
    // This arm appends a pending `q6` and uses a budget whose WATERMARK does
    // not land on an exchange boundary, so an entry-wise loop fits
    // mid-exchange. The budget is chosen from a sweep, not a guess: against
    // the entry-wise loop the orphan bands are 575–700 (`a4`), 875–975 (`a3`)
    // and 1150–1275 (`a2`), with clean bands at 400–550, 725–850 and
    // 1000–1125.
    //
    // THE FIRST VERSION OF THIS COMMENT REPORTED ONLY THE FIRST BAND, and
    // generalized the rest away as "750+ cut cleanly again". It did not: 750–850
    // IS clean and 875–975 is not, so a budget picked from "750+" is a coin
    // flip — and the coin lands vacuous more often than not. The full band list
    // is here so the next budget choice cannot be made from a summary of a
    // sweep that stopped early.
    //
    // Why the bands repeat at all: a cut that lands between a prompt and its
    // response orphans, and raising the budget by one entry moves the cut back
    // onto a boundary. The bands are every budget whose watermark lands
    // mid-exchange, which is most of them — 400 was chosen because its
    // watermark does not.
    let pending = entries + [prompt("q6")]
    let pendingCosts = costs + [100]
    let pendingPlan = HistoryBudget.plan(pending, costs: pendingCosts, budgetTokens: 700)
    let pendingKept = pendingPlan.apply(pending)
    let pendingIds = Set(pendingKept.map(\.id))
    print("KEPT (pending prompt): \(pendingKept.map(text).joined(separator: " "))")

    #expect(
        pendingKept.last.map(text) == "q6",
        "the pending prompt — the question being asked right now — was dropped"
    )
    let pendingOrphans = orphans(in: pending, kept: pendingIds)
    #expect(
        pendingOrphans.isEmpty,
        "with a pending prompt, kept \(pendingOrphans.count) orphaned response(s): \(pendingOrphans.joined(separator: "; "))"
    )
    // Every kept exchange is whole in the other direction too: a kept prompt
    // keeps its response. An exchange-wise drop cannot produce a lone prompt;
    // an entry-wise one can.
    for (i, entry) in pending.enumerated() where pendingIds.contains(entry.id) {
        guard case .prompt = entry, i + 1 < pending.count else { continue }
        if case .response = pending[i + 1] {
            #expect(
                pendingIds.contains(pending[i + 1].id),
                "kept prompt \(text(entry)) lost its response — a half exchange"
            )
        }
    }
    #endif
}

// A TRIM THAT COULD NOT MEET ITS BUDGET HAS TO SAY SO.
//
// The exchange floor (`keepAtLeast`) is the right policy and an unbendable one:
// the last exchange is what "continue the conversation" means. But a floor that
// BINDS — a conversation whose newest exchange is larger than the whole budget —
// produces a plan that is over the window, and until `unmetTokens` existed the
// plan said nothing about it. `apply` returned the over-budget transcript, the
// request went out, and the only report was the framework's:
//
//     "the framework throws a LanguageModelError.contextSizeExceeded(_:) error
//      and the session stops responding"
//
//     — https://developer.apple.com/documentation/foundationmodels/managing-the-context-window
//
// Not one lost turn: the END of the session, discovered after the fact. So the
// floor's consequence has to be available BEFORE the request, which is what this
// pins. Three values, and the third is the one a `0` default would have hidden:
//
//   0      the trim met its target (the ordinary case)
//   > 0    the floor held with entries still to drop; the request WILL overflow
//   nil    the plan could not be measured at all — unknown, and read as overflow
//
// The third arm is the fail-closed one, and it is the arm that makes the field
// honest: `costs` that do not describe the transcript mean the conversation's
// size is unknown, and an unknown size is not a fitting one.
@Test("A floor-bound trim REPORTS the tokens it could not drop")
func floorBoundTrimReportsItsUnmetTokens() {
    #if canImport(FoundationModels)
    // 27.0, not 26.0: `Transcript.Response.init` is macOS 27. The PLANNER is
    // 26.0 — this pin's guard is set by the fixtures it has to build, and
    // claiming 26 here would be an availability claim this test cannot honour.
    guard #available(macOS 27.0, *) else { return }

    func prompt(_ t: String) -> Transcript.Entry {
        .prompt(Transcript.Prompt(segments: [.text(Transcript.TextSegment(content: t))]))
    }
    func response(_ t: String) -> Transcript.Entry {
        .response(Transcript.Response(segments: [.text(Transcript.TextSegment(content: t))]))
    }

    // Three exchanges of 400 plus the pending prompt — 2,500 tokens against a
    // 500-token budget, so the floor is forced to bind partway through.
    var entries: [Transcript.Entry] = []
    var costs: [Int] = []
    for i in 0..<3 {
        entries.append(prompt("q\(i)")); costs.append(400)
        entries.append(response("a\(i)")); costs.append(400)
    }
    entries.append(prompt("qP")); costs.append(100)

    let bound = HistoryBudget.plan(entries, costs: costs, budgetTokens: 500)
    print("floor-bound: kept=\(bound.keptTokens) unmet=\(String(describing: bound.unmetTokens))")
    #expect(
        bound.unmetTokens == 400,
        "expected the floor to hold 400 tokens over a 500 budget, got \(String(describing: bound.unmetTokens))"
    )
    #expect(
        bound.exceedsBudget,
        "a plan 400 tokens over the window did not report itself as one"
    )

    // THE CONTROL, and the reason this pin is not a tautology. The SAME
    // transcript with a budget its last exchange fits inside: the loop still
    // stops on the floor — dropping the newest exchange is not on the table —
    // and `unmetTokens` is still `0`, because the floor held nothing over. A pin
    // that only ever saw the first arm could not tell "the loop stopped" from
    // "the plan overflowed", and those are different facts.
    let fits = HistoryBudget.plan(entries, costs: costs, budgetTokens: 1000)
    #expect(
        fits.unmetTokens == 0,
        "a plan that fits the budget reported \(String(describing: fits.unmetTokens)) unmet"
    )
    #expect(
        !fits.exceedsBudget,
        "a plan that fits the budget claimed to exceed it"
    )

    // The other floor exit: nothing droppable at all. One exchange plus the
    // pending prompt is below `keepAtLeast` eligible entries, so the planner
    // returns before any loop — and the conversation is 900 tokens against a
    // 200-token budget.
    let tiny = [prompt("q0"), response("a0"), prompt("qP")]
    let tinyPlan = HistoryBudget.plan(tiny, costs: [400, 400, 100], budgetTokens: 200)
    #expect(
        tinyPlan.unmetTokens == 700,
        "the empty-eligible-range floor reported \(String(describing: tinyPlan.unmetTokens))"
    )

    // UNMEASURED. Costs that do not describe this transcript: the honest answer
    // is `nil`, and `exceedsBudget` reads `nil` as overflow so a caller that
    // forgets to check cannot treat it as a fitting plan.
    let unmeasured = HistoryBudget.plan(entries, costs: [1, 2, 3], budgetTokens: 500)
    #expect(
        unmeasured.unmetTokens == nil,
        "an unmeasurable plan reported \(String(describing: unmeasured.unmetTokens)) — unknown is not zero"
    )
    #expect(
        unmeasured.exceedsBudget,
        "an unmeasured plan did not read as overflow; unknown is not 'it fits'"
    )
    #endif
}

// THE ROUTER HAS TO SEE THE CONVERSATION IT IS ROUTING.
//
// `ChatRouter.decide` compared `evidenceTokens` — the recall payload — against
// the WHOLE budget, and the budget has already had instructions, tool schemas
// and the response reserve subtracted from it. So the conversation itself was
// invisible: the measured history-pressure case routes a 2,600-token trimmed
// history plus a 200-token payload against a 2,710 budget as `.onDevice`, with
// no escalation and no record, and the turn goes out over the window.
//
// The window is one pool — instructions, tools, transcript and answer draw from
// the same 4,096 — so the demand is the SUM. This policy had no pin at all
// before now: `decide`, `EscalationReason`, `escalationRefused` and the
// fail-closed-on-unknown-budget rule were all unpinned, in a file where the
// neighbouring policies are pinned several times over.
//
// The arms are ordered so the pin can fail in only one direction: the blindness
// arm first, then the refusal record, then the two controls that stop a fixed
// comparison from reading as correct.
@Test("Routing compares the CONVERSATION against the budget, not the evidence alone")
func routerRoutesOnTheConversationNotTheEvidenceAlone() {
    #if canImport(FoundationModels)
    guard #available(macOS 26.0, *) else { return }

    let budget = 2710  // the measured conversation budget: 4096 − 58 − 304 − 1024
    let router = ChatRouter(escalationPermitted: true, onDeviceBudget: budget)

    // THE BLINDNESS ARM. 200 alone is 7% of the budget; 2,800 together is over
    // it. A router that reads only the evidence calls this `.onDevice`.
    let overConversation = router.decide(turn: 4, evidenceTokens: 200, historyTokens: 2600)
    #expect(
        overConversation.tier == .deep,
        "a turn asking for 2,800 tokens against a \(budget) budget routed on-device"
    )
    #expect(
        overConversation.reason == .conversationExceedsOnDeviceBudget,
        "the escalation was recorded as \(String(describing: overConversation.reason))"
    )
    #expect(
        !overConversation.escalationRefused,
        "escalation was permitted here, so a refusal flag is false information"
    )

    // THE SAME TURN WITH ESCALATION REFUSED — which is every turn on this
    // machine, M1 — must be RECORDED, not silent. A caller that cannot see the
    // refusal reads the fallback as "escalation was not needed".
    let refusing = ChatRouter(escalationPermitted: false, onDeviceBudget: budget)
        .decide(turn: 4, evidenceTokens: 200, historyTokens: 2600)
    #expect(
        refusing.tier == .onDevice,
        "an unpermitted escalation must fall back on-device, not route deep"
    )
    #expect(
        refusing.escalationRefused,
        "the refusal was silent — the caller reads this as 'escalation not needed'"
    )
    #expect(
        refusing.reason == .conversationExceedsOnDeviceBudget,
        "the refusal was recorded as \(String(describing: refusing.reason))"
    )

    // CONTROL 1 — the comparison is `>`, not `>=`. Exactly at the limit fits; an
    // off-by-one here either escalates every full turn or overflows every one.
    let exact = router.decide(turn: 4, evidenceTokens: 200, historyTokens: 2510)
    #expect(
        exact.tier == .onDevice && exact.reason == nil && !exact.escalationRefused,
        "a turn of exactly \(budget) tokens was treated as over budget: \(exact)"
    )

    // CONTROL 2 — the ordinary case still passes through untouched, so the arms
    // above cannot be satisfied by a router that escalates unconditionally.
    let fits = router.decide(turn: 4, evidenceTokens: 200, historyTokens: 400)
    #expect(
        fits.tier == .onDevice && fits.reason == nil && !fits.escalationRefused,
        "a turn well inside the budget was escalated: \(fits)"
    )

    // Turn 1 never escalates, however far over it is — the latency law, and the
    // reason trigger 3 sits above trigger 2.
    let firstTurn = router.decide(turn: 1, evidenceTokens: 200, historyTokens: 9000)
    #expect(
        firstTurn.tier == .onDevice && firstTurn.reason == .neverOnFirstTurn,
        "turn 1 escalated: \(firstTurn)"
    )

    // AN UNREADABLE BUDGET still fails closed — no escalation, no refusal, and
    // therefore no claim that the turn fits. This is the pre-existing law and
    // the new parameter must not have weakened it.
    let unknown = ChatRouter(escalationPermitted: true, onDeviceBudget: nil)
        .decide(turn: 4, evidenceTokens: 200, historyTokens: 9000)
    #expect(
        unknown.tier == .onDevice && !unknown.escalationRefused,
        "an unknown budget escalated: \(unknown)"
    )
    #endif
}
//
// Plan 19 §3.1 rests on one claim: a `DynamicProfile` switches model INSIDE one
// `LanguageModelSession`, preserving conversation history, so the user feels
// nothing. That claim is UNMEASURED for the real escalation — M1 established
// that PCC refuses every request on this machine (`ModelManagerError 1046`), so
// no turn has ever actually switched between AFM and PCC.
//
// So this pins the MECHANISM, not the PCC path: a `ConditionalDynamicProfile`
// that flips between two profile branches inside ONE session, with a
// fabricated value planted in turn 1 and asked for after the switch. If history
// did not survive a profile change, the escalation design would be wrong, and
// the switch here is the same `buildEither` the escalation branch would use.
//
// WHAT THIS IS NOT, stated so the next reader does not over-credit it: it does
// not exercise PCC, does not exercise the 4,096 → 32,768 window change, and
// proves nothing about cross-tier token accounting. It proves one thing — the
// transcript survives a profile change — which is the part that has never been
// observed and is cheapest to break.
//
// Cold control, as everywhere: a session that never heard the value must not
// produce it, or the warm arm is unfalsifiable.
@Test("History: survives a mid-session profile switch, and a cold session cannot fake it")
func historySurvivesAProfileSwitch() async throws {
    #if canImport(FoundationModels)
    guard #available(macOS 27.0, *) else {
        print("DynamicProfile needs macOS 27 — profile-switch history not driven here")
        return
    }
    let model = SystemLanguageModel.default
    guard case .available = model.availability else {
        print("on-device model unavailable — profile-switch history not driven here")
        return
    }
    let secret = "4147/221"

    // The switch. `escalate` is the stand-in for "the policy decided this turn
    // earns the big model" — same `buildEither`, same single session, same
    // transcript.
    let control = SwitchModifier.Control(escalate: false)

    let session = LanguageModelSession(
        profile: LanguageModelSession.Profile {
            Instructions("Reply briefly.")
        }
        .model(model)
        .modifier(SwitchModifier(control: control))
    )

    func ask(_ text: String) async throws -> String {
        (try await session.respond(to: text)).content
    }

    _ = try await ask("Remember this number for later: \(secret). Reply only 'ok'.")
    // Flip the branch. Everything the session has said so far must survive it.
    control.escalate = true
    let after = try await ask("What was the number I asked you to remember? Numbers only.")

    // COLD: a session that never saw the value.
    let cold = LanguageModelSession(model: model)
    let coldAnswer = try await (cold.respond(to: "What was the number I asked you to remember? Numbers only.")).content

    let warmHas = after.contains(secret)
    let coldHas = coldAnswer.contains(secret)
    print("""
    ---
    Profile switch ---
      after switch : \(after.trimmingCharacters(in: .whitespacesAndNewlines).prefix(80))
      cold          : \(coldAnswer.trimmingCharacters(in: .whitespacesAndNewlines).prefix(80))
      warm contains the value: \(warmHas)   cold contains it: \(coldHas)
    ---
    """)
    #expect(
        !coldHas,
        "a session that never heard the value produced it — the warm arm is unfalsifiable"
    )
    #expect(warmHas, "the transcript did NOT survive the profile switch — escalation would lose the conversation")
    #endif
}

/// Swaps the active profile when `escalate` is set. Both branches are the SAME
/// on-device model with different settings, because the question under test is
/// whether the TRANSCRIPT survives a branch change, not what the other branch
/// computes.
@available(macOS 27.0, *)
struct SwitchModifier: LanguageModelSession.DynamicProfileModifier {
    final class Control: @unchecked Sendable {
        var escalate: Bool
        init(escalate: Bool) { self.escalate = escalate }
    }
    let control: Control

    func body(content: Content) -> some LanguageModelSession.DynamicProfile {
        if control.escalate {
            content.temperature(0.2)
        } else {
            content.temperature(0.9)
        }
    }
}

// THE EXCLUSION IS A PIN, NOT A CONVENTION.
//
// Qwen3.5-9B throws `runtime("No user query found in messages.")` on turn 2 of
// any conversation, on BOTH `LanguageModelSession` and the mlx-native
// `ChatSession`.
//
// The DIAGNOSIS lives in exactly one place — the `ChatModelId` doc comment in
// `ChatModelLayer.swift` — and is deliberately not restated here, because it was
// restated in four places and drifted. An earlier version of this comment called
// that symmetry "the diagnosis" and concluded the MLX executor's prefix
// matching, which the same symmetry does not license: mlx-native `ChatSession`
// never goes through a `LanguageModelExecutor`, so it cannot be the common
// factor. Weights are not implicated; the cause is not established.
//
// What this pin holds is narrower and does not depend on settling the cause:
// the id is REFUSED.
//
// "Simply not wired up" is not an exclusion. It re-enters the first time
// someone adds a model picker, and the failure is silent: it starts throwing on
// turn 2, which presents as a memory bug and gets chased as one. So the refusal
// is a value, a throwing entry point, and this pin.
//
// The pin enumerates `allCases` rather than naming the one bad id, so a model
// added LATER is covered by the same rule without this file being edited.
@Test("Qwen3.5 cannot enter a chat path — the id is refused, not merely unused")
func qwen35CannotEnterAChatPath() {
    // The named case, stated plainly so a reader does not have to infer it.
    #expect(
        !ChatModelId.qwen3_5.mayServeChat,
        "Qwen3.5 is marked chat-capable again — it throws on turn 2 of every conversation"
    )
    #expect(
        ChatModelId.qwen3_5.chatRefusalReason?.isEmpty == false,
        "an excluded model must say WHY, or the next person re-adds it to find out"
    )
    #expect(
        ChatModelId.onDeviceAFM.mayServeChat,
        "the measured tier must remain usable — nothing else has ever completed a turn here"
    )

    // The REFUSAL actually happens, rather than the flag merely being false.
    #expect(throws: ChatModelError.self) {
        try ChatModelId.forChat(.qwen3_5)
    }
    #expect(throws: Never.self) {
        try ChatModelId.forChat(.onDeviceAFM)
    }

    // The law, over the whole set: every id is either chat-capable or carries a
    // reason. An id that is neither is a model someone can route to by accident.
    for id in ChatModelId.allCases {
        #expect(
            id.mayServeChat != (id.chatRefusalReason != nil),
            "\(id.rawValue) is neither usable nor explained — one of the two must be true"
        )
    }
}

// TRAP 1's compile-time half, held as a pin.
//
// `SystemLanguageModel.contextSize` is a plain `var`;
// `PrivateCloudComputeLanguageModel.contextSize` is `get async throws`. Code
// that reads the window synchronously compiles against the tier that happens to
// be in scope and FAILS against the other one — and a Dynamic Profile that
// swaps tiers is precisely where that surfaces, at a call site that is correct
// on the branch it was written on.
//
// `ChatWindow` exists to make that impossible by routing both reads through one
// signature. A TYPE pin catches a caller that bypasses it; it cannot catch a new
// synchronous read added elsewhere in the package, so the source scan below does
// that, and names the one sanctioned exception.
//
// The scan is over SOURCE TEXT, so it is a structural pin and not proof of
// behaviour. That is stated here because this package's law is that a
// structural row is a regression lock, never a proof.
@Test("contextSize is read through ONE path — both tiers, one async read")
func contextSizeIsReadThroughOnePath() async throws {
    // The source scan. `ChatWindow` is the only place allowed to touch
    // `.contextSize`; every other read is a call site that will break the day
    // the profile swaps to PCC. It runs first and unguarded, because it is a
    // property of the SOURCE and holds on every host — including hosts where
    // the framework cannot answer a question at all.
    let here = URL(fileURLWithPath: #filePath)
    let sources = here.deletingLastPathComponent()   // Tests/AppleFMProbeTests
        .deletingLastPathComponent()                  // Tests
        .deletingLastPathComponent()                  // shell-macos
        .appendingPathComponent("Sources/AppleFMProbe")
    let files = try FileManager.default.contentsOfDirectory(
        at: sources, includingPropertiesForKeys: nil
    ).filter { $0.pathExtension == "swift" }

    #expect(!files.isEmpty, "the source scan found no files — it is passing vacuously")

    // The ONE sanctioned exemption, and it is bounded rather than ignored.
    // `AppleFMProbe`'s verbose block prints the raw plain-vs-async difference —
    // that is the fact it exists to show, and routing it through `ChatWindow`
    // would delete the evidence. So the exemption is a MARKED REGION, and this
    // pin fails two ways: a read outside a marked region, or a marked region
    // whose marker was deleted while the reads remained. Deleting the comment
    // to "clean up" therefore turns the build red instead of quietly widening
    // the hole.
    let begin = "ONE-PATH-EXEMPTION BEGIN"
    let end = "ONE-PATH-EXEMPTION END"

    var offenders: [String] = []
    for file in files {
        // The sanctioned seam: its own file.
        if file.lastPathComponent == "ChatModelLayer.swift" { continue }
        let text = try String(contentsOf: file, encoding: .utf8)
        var exempted = false
        for (number, line) in text.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
            if line.contains(begin) { exempted = true; continue }
            if line.contains(end) { exempted = false; continue }
            guard line.contains(".contextSize"), !exempted else { continue }
            offenders.append("\(file.lastPathComponent):\(number + 1)")
        }
        // A region opened and never closed would exempt the rest of the file
        // silently, so an unterminated marker is itself a failure.
        #expect(
            !exempted,
            "\(file.lastPathComponent) opened a ONE-PATH-EXEMPTION and never closed it — the exemption would swallow the rest of the file"
        )
    }
    #expect(
        offenders.isEmpty,
        """
        a direct .contextSize read outside ChatWindow at \
        \(offenders.joined(separator: ", ")). It compiles against the on-device \
        model and fails against PCC — read it through ChatWindow instead.
        """
    )

    #if canImport(FoundationModels)
    guard #available(macOS 27.0, *) else {
        print("both window surfaces need macOS 27 — the type half of this pin is structural only here")
        return
    }
    // Both readers return the SAME type, and the remote one is genuinely async:
    // that pairing is what a synchronous call site cannot reproduce.
    let onDevice = ChatWindow.onDevice()
    let privateCloud = await ChatWindow.privateCloud()
    // The shared RETURN TYPE is enforced by the compiler, not by a runtime
    // `type(of:)` comparison — two `type(of:)` expects were removed here
    // because they asserted something the type system had already guaranteed and
    // therefore could never fail. A vacuous pin is worse than no pin: it reads
    // as coverage.
    //
    // What IS worth asserting is that both answers are WELL-FORMED, matched by
    // case. The previous version was
    //     onDevice.tokens != nil || onDevice == .unreadable(reason: "")
    // which is FALSE for every real `.unreadable` case: on the `xcode-27` CI
    // runner the model is `deviceNotEligible`, `onDevice()` returns
    // `.unreadable(reason: "on-device model unavailable")`, and that whole
    // expression is false twice over. The suite went red on the runner while
    // green on a laptop — which is the "a laptop always has the model" defect,
    // not a missing assertion.
    switch onDevice {
    case let .window(n):
        #expect(n > 0, "a reported window of zero is not a window")
    case let .unreadable(reason):
        #expect(!reason.isEmpty, "an unreadable window must say why")
    }
    switch privateCloud {
    case let .window(n):
        #expect(n > 0, "a reported window of zero is not a window")
    case let .unreadable(reason):
        #expect(!reason.isEmpty, "an unreadable window must say why")
    }
    #endif
}

// THE CALL SITE. This is the pin for the gap where a correct trimmer and a
// correct router were each exercised only by their own pins, and neither read
// the other.
//
// `TrimPlan.unmetTokens` computed a shortfall nobody consumed. `ChatRouter.decide`
// compared numbers nobody supplied. Both were correct and both were inert, which
// is a failure mode no individual unit test can see: every pin was green and
// the layer still sent over-budget turns that ended the session at
// `contextSizeExceeded`.
//
// The property: a floor-bound conversation is REFUSED BEFORE the request, and
// the refusal carries the shortfall.
@Test("A conversation that cannot fit is REFUSED before the request, with the shortfall")
func chatTurnPlannerRefusesAConversationThatCannotFit() async throws {
    #if canImport(FoundationModels)
    guard #available(macOS 27.0, *) else {
        print("needs macOS 27 — call-site refusal not driven here")
        return
    }

    func prompt(_ t: String) -> Transcript.Entry {
        .prompt(Transcript.Prompt(segments: [.text(Transcript.TextSegment(content: t))]))
    }
    func response(_ t: String) -> Transcript.Entry {
        .response(Transcript.Response(segments: [.text(Transcript.TextSegment(content: t))]))
    }

    // A transcript whose LAST EXCHANGE alone is larger than the budget: the
    // floor binds, dropping cannot help, and the request would overflow.
    var entries: [Transcript.Entry] = []
    for i in 0..<6 {
        entries.append(prompt("q\(i)"))
        entries.append(response("a\(i)"))
    }
    entries.append(prompt("qP"))

    // The budget must be BELOW the floor for this fixture to bind. The exchange
    // floor keeps one whole exchange (200) plus the pending prompt (100), so
    // the smallest possible request is 300 tokens: at a 500-token budget the
    // trim succeeds and nothing is refused. My first version used 500 and the
    // pin went red — correctly, because it had not built a case that needed
    // refusing at all. 150 is below the floor.
    //
    // Escalation DELIBERATELY not permitted: this machine refuses every PCC
    // request (M1), so the on-device path is the only one that can run, and it
    // is the one that must refuse.
    let planner = ChatTurnPlanner(
        router: ChatRouter(escalationPermitted: false, onDeviceBudget: 150),
        controller: HistoryBudget.Controller(budgetTokens: 150)
    )

    let constantCost: @Sendable (Transcript.Entry) async -> Int = { _ in 100 }

    // REFUSED, and the shortfall is reported rather than the turn being sent.
    var caught: ChatTurnError?
    do {
        _ = try await planner.prepare(
            turn: 4, evidenceTokens: 0, entries: entries,
            incomingTokens: 100, cost: constantCost
        )
    } catch let error as ChatTurnError {
        caught = error
    }
    switch caught {
    case let .conversationDoesNotFit(shortfall):
        #expect(shortfall > 0, "the refusal carried a non-positive shortfall — it reports nothing actionable")
    case nil:
        Issue.record("the planner SENT a conversation that cannot fit — this is the silent overflow the layer exists to prevent")
    }

    // ANTI-VACUITY: the same planner must NOT refuse a conversation that fits,
    // or "always throws" would satisfy everything above.
    let roomy = ChatTurnPlanner(
        router: ChatRouter(escalationPermitted: false, onDeviceBudget: 5_000),
        controller: HistoryBudget.Controller(budgetTokens: 5_000)
    )
    let ok = try await roomy.prepare(
        turn: 4, evidenceTokens: 100, entries: entries,
        incomingTokens: 100, cost: constantCost
    )
    #expect(ok.tier == .onDevice, "a conversation inside its window was not routed on-device")
    #expect(
        !ok.escalationRefused,
        "a conversation inside its window recorded a refused escalation"
    )
    // The planner's `transform` must be the SAME plan it routed on. If it were a
    // second, independently-built trimmer, the request could be sent with a
    // history the routing decision never saw — the two halves disagreeing
    // behind a single API.
    _ = try await roomy.prepare(
        turn: 2, evidenceTokens: 100, entries: entries,
        incomingTokens: 100, cost: constantCost
    )
    let viaTransform = Set(roomy.transform(entries).map(\.id))
    #expect(
        !viaTransform.isEmpty,
        "the planner's transform dropped the entire transcript — including the pending prompt"
    )

    // THE ROUTE ARM, and it exists because the first version of this pin was
    // half blind.
    //
    // Drilled both ways and the refusal pin passed under EITHER: routing the
    // conversation on `evidenceTokens` alone, and passing a pre-trim
    // `historyTokens`. Both are wrong, and both were invisible — because a
    // floor-bound turn throws on `plan.exceedsBudget` BEFORE the router's
    // comparison can matter. The throw path and the route path need separate
    // arms, and only the throw path had one.
    //
    // So: a conversation that FITS the window but whose total DEMAND does not.
    // No throw is possible here, so the decision is the only observable, and
    // the decision depends on `historyTokens` being the post-trim number.
    let routing = ChatTurnPlanner(
        router: ChatRouter(escalationPermitted: false, onDeviceBudget: 1_000),
        controller: HistoryBudget.Controller(budgetTokens: 1_000)
    )
    // Measured, not guessed: `refresh` plans against `budget - incoming` = 900
    // with a 630 watermark, so it keeps four eligible entries (400) plus the
    // pending prompt (100) = 500. Evidence of 700 puts the demand at 1,200
    // against 1,000 — over the window, with the conversation itself fitting.
    let demandsRefusal = try await routing.prepare(
        turn: 4, evidenceTokens: 700, entries: entries,
        incomingTokens: 100, cost: constantCost
    )
    #expect(
        demandsRefusal.escalationRefused,
        "a 500-token conversation plus a 700-token payload against a 1,000-token budget did not record a refused escalation — the router is not seeing the conversation"
    )
    #expect(demandsRefusal.tier == .onDevice, "with escalation refused the tier must be onDevice")

    // Same shape, escalation PERMITTED: now it must route deep, and the
    // refusal flag must be false — the two are different outcomes and a pin
    // that only checks one of them passes an implementation that inverts them.
    let escalating = ChatTurnPlanner(
        router: ChatRouter(escalationPermitted: true, onDeviceBudget: 1_000),
        controller: HistoryBudget.Controller(budgetTokens: 1_000)
    )
    let routed = try await escalating.prepare(
        turn: 4, evidenceTokens: 700, entries: entries,
        incomingTokens: 100, cost: constantCost
    )
    #expect(routed.tier == .deep, "an over-budget conversation with escalation permitted did not route deep")
    #expect(!routed.escalationRefused, "a successful escalation recorded itself as refused")
    #expect(
        routed.reason == .conversationExceedsOnDeviceBudget,
        "the routed decision does not name the conversation as the cause"
    )

    // TURN 1 still never escalates, through the call site rather than only
    // through `decide` — the one rule a caller cannot be trusted to remember.
    let first = try await roomy.prepare(
        turn: 1, evidenceTokens: 100, entries: entries,
        incomingTokens: 100, cost: constantCost
    )
    #expect(first.reason == .neverOnFirstTurn, "turn 1 routed without the never-on-first-turn rule")
    #endif
}

// A QUOTE ATTRIBUTED TO APPLE MUST CITE APPLE.
//
// `HistoryBudget` previously block-quoted "Transcripts mutations can invalidate
// key-value caches and raise latency; appending preserves them…" and introduced
// it with "Apple's own wording is the rule this exists to enforce". It is a
// paraphrase of WWDC26's third-party session notes, not a sentence from Apple's
// documentation, and an unciteable quote presented as a citation is exactly what
// makes the next reader stop checking.
//
// The rule is GENERAL rather than a phrase list: any block quote attributed to
// Apple must sit in a file that links Apple's documentation. That catches the
// next paraphrase, which no phrase list would.
@Test("Every Apple-attributed quote is VERBATIM on a cited Apple page")
func appleQuotesAreVerifiedVerbatim() throws {
    // THIS PIN HAS ALREADY FAILED TO DO ITS JOB ONCE, and the way it failed is
    // the reason it is written this way.
    //
    // v1 required a `developer.apple.com` link in the same comment block as a
    // quote. It verified a citation was PRESENT, not that the quote was ON THE
    // PAGE — and a confident fabrication beside a real URL sailed through:
    //
    //   "The session stops responding after this error, so create a new session
    //    in the catch block"
    //
    // That sentence is in NO Apple document. It is a conflation of two real
    // ones, it shipped in c98cbbb3, and v1 of this pin certified it.
    //
    // v2 also scanned only `Sources/`, and matched only the `> "` block-quote
    // form, so it never saw the inline `*"…"` quotations in this file. Both
    // corrected, and both corrections found real uncited quotes on first run.
    //
    // v3 (this one) checks the QUOTE TEXT. Every Apple-attributed sentence must
    // appear verbatim in `apple-verified-quotes.txt`, a committed corpus of
    // sentences fetched from Apple's pages with their URLs and the date read.
    // That is offline, deterministic, and it fails on exactly the thing v1 and
    // v2 could not see: an invented sentence with a real citation attached.
    // Adding a sentence to the corpus means someone went and fetched the page.
    let packageRoot = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
    let corpusURL = packageRoot
        .appendingPathComponent("Tests/AppleFMProbeTests/apple-verified-quotes.txt")
    let corpusRaw = try String(contentsOf: corpusURL, encoding: .utf8)
    // Strip the comment scaffolding, then collapse whitespace so a sentence
    // wrapped across lines still matches.
    let corpus = corpusRaw
        .split(separator: "\n")
        .filter { !$0.hasPrefix("#") }
        .joined(separator: " ")
        .split(whereSeparator: { $0.isWhitespace })
        .joined(separator: " ")

    #expect(corpus.count > 1_000, "the verified-quote corpus looks empty — the check below is vacuous")

    var files: [URL] = []
    for dir in ["Sources/AppleFMProbe", "Tests/AppleFMProbeTests"] {
        files += try FileManager.default.contentsOfDirectory(
            at: packageRoot.appendingPathComponent(dir),
            includingPropertiesForKeys: nil
        ).filter { $0.pathExtension == "swift" }
    }
    #expect(files.count >= 3, "the scan found \(files.count) files — it is passing vacuously")

    /// Pull every Apple-attributed quotation out of one comment block.
    func quotes(in block: [String]) -> [String] {
        let flat = block.joined(separator: " ")
        // Case-INSENSITIVE, and this is a hole that was found by drilling: the
        // gate was `contains("Apple")`, and re-planting the EXACT quote that
        // shipped in c98cbbb3 passed the v3 pin — because the block holding it
        // contains only `developer.apple.com`, lowercase, and was skipped
        // whole. The strongest pin in the file had a gate that let the known
        // fabrication walk past it. Any apple.com citation or the word "apple"
        // in any case makes the block attributable.
        guard flat.lowercased().contains("apple") else { return [] }
        var found: [String] = []
        let markers = ["> *\"", "*\""]
        for marker in markers {
            var rest = flat[flat.startIndex...]
            while let open = rest.range(of: marker) {
                let after = rest[open.upperBound...]
                // The OPENING delimiter is `*"` and the CLOSING is `"*`. Matching
                // the close with `*"` never fires — it matches across the next
                // quote's own opening — so every extracted "quote" ran on into
                // the following sentence and failed the corpus check for a
                // reason that had nothing to do with accuracy.
                guard let close = after.range(of: "\"*") else { break }
                // Block quotes wrap across lines with a leading "> " on each
                // continuation, and the block was joined with spaces, so the
                // marker lands INSIDE the extracted text. Apple sentences do not
                // contain "> ", so it is stripped rather than special-cased.
                let raw = String(after[after.startIndex..<close.lowerBound])
                let text = raw.replacingOccurrences(of: "> ", with: "")
                // A real quotation, not an inline fragment on a code sample.
                if text.count > 30, text.contains(" ") { found.append(text) }
                rest = after[close.upperBound...]
            }
        }
        return found
    }

    var checked = 0
    var offenders: [String] = []
    for file in files {
        let text = try String(contentsOf: file, encoding: .utf8)
        var block: [String] = []
        func flush() {
            defer { block = [] }
            for quote in quotes(in: block) {
                // An elided quote ("… and the rest") is checked fragment by
                // fragment, because the corpus has no elided form of anything.
                let fragments = quote
                    .split(separator: "\u{2026}")
                    .map { $0.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ") }
                    .filter { $0.count > 30 }
                for fragment in fragments {
                    checked += 1
                    if !corpus.contains(fragment) {
                        offenders.append(
                            "\(file.lastPathComponent): \"\(fragment.prefix(70))\"\u{2026}\""
                        )
                    }
                }
            }
        }
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("//") {
                // Drop ALL the slashes, not two: a doc comment has three, and
                // leaving one behind injected a "/" into the middle of every
                // quoted sentence — which the corpus then (correctly) refused.
                block.append(String(trimmed.drop(while: { $0 == "/" })))
            } else {
                flush()
            }
        }
        flush()
    }

    #expect(
        checked > 0,
        "no Apple-attributed quote was found — the extractor matches nothing and is passing vacuously"
    )
    #expect(
        offenders.isEmpty,
        """
        \(offenders.count) Apple-attributed sentence(s) not found verbatim in \
        apple-verified-quotes.txt: \(offenders.joined(separator: " | ")). Either it is a \
        paraphrase, or it was invented. Fetch the page, copy the sentence exactly, \
        and record the URL — do not make it true by editing the corpus to match.
        """
    )
}

// TRAP 4 — the false claim cannot come back.
//
// `shell-macos-mlx/Package.swift` asserted that the MLX adapter gives "the same
// transcript" as `SystemLanguageModel`. For multi-turn that is FALSE: Qwen3.5
// throws `runtime("No user query found in messages.")` on turn 2 of any
// conversation, on BOTH `LanguageModelSession` and the mlx-native `ChatSession`.
// The symmetry is the diagnosis — not a call-shape problem, not the weights —
// and the sentence is exactly the kind that sends the next reader down the same
// path for an hour.
//
// Comments rot silently, so the correction is held by a scan rather than by
// goodwill. It reads the file across the package boundary and fails if the
// claim reappears in any of the wordings that have actually been tried.
@Test("The MLX package header no longer claims the adapter gives 'the same transcript'")
func mlxPackageHeaderDoesNotClaimTheSameTranscript() throws {
    let here = URL(fileURLWithPath: #filePath)
    let package = here.deletingLastPathComponent()   // Tests/AppleFMProbeTests
        .deletingLastPathComponent()                  // Tests
        .deletingLastPathComponent()                  // shell-macos
        .deletingLastPathComponent()                  // repo root
        .appendingPathComponent("shell-macos-mlx/Package.swift")

    let text = try String(contentsOf: package, encoding: .utf8)

    // Whitespace-collapsed, because the claim has been wrapped across lines
    // before and a raw substring check went green on the very lie it was
    // written to catch.
    let flat = (text as String)
        .split(whereSeparator: { $0.isWhitespace })
        .joined(separator: " ")

    for claim in [
        "the same transcript",
        "same transcript",
    ] {
        #expect(
            !flat.contains(claim),
            """
            shell-macos-mlx/Package.swift claims "\(claim)" again. It is FALSE for \
            multi-turn: Qwen3.5 throws on turn 2 of any conversation on both \
            LanguageModelSession and ChatSession.
            """
        )
    }

    // The claim the file SHOULD make, so the correction cannot hollow out into
    // silence — a header that says nothing is how the next reader re-adds it.
    #expect(
        flat.contains("Qwen3.5"),
        "the header must name the model whose multi-turn failure falsified the claim"
    )
}

#if canImport(FoundationModels)
// MARK: - P1: the recall tool's Swift end

/// A transport the test drives. It stands in for the HOST's bridge into the
/// Rust core — never for the core itself, whose bounds and classification are
/// pinned in `shell/src-tauri/src/recall_tests.rs` and reach Swift already
/// classified.
private final class StubTransport: KernelRecallTransport, @unchecked Sendable {
    var outcome: RecallOutcome
    var failure: Error?
    private(set) var seen: [(query: String, domain: String?)] = []

    init(outcome: RecallOutcome, failure: Error? = nil) {
        self.outcome = outcome
        self.failure = failure
    }

    func recall(query: String, domain: String?) async throws -> RecallOutcome {
        seen.append((query, domain))
        if let failure { throw failure }
        return outcome
    }
}

private func passage(
    id: Int,
    text: String = "the passage text",
    domain: String? = "legal",
    validTo: String? = nil
) -> RecallPassage {
    RecallPassage(
        id: id,
        title: "A title",
        text: text,
        domain: domain,
        score: 0.5,
        validFrom: "2024-01-01T00:00:00Z",
        validTo: validTo,
        flagged: false
    )
}

@available(macOS 26.0, *)
@Test("the recall tool renders the core's answer, and the arguments cross intact")
func recallToolReturnsRowsThroughTheCore() async throws {
    let transport = StubTransport(
        outcome: .answered(
            passages: [passage(id: 42, text: "GDPR covers EU residents")],
            decision: .ok,
            confidence: 0.93
        )
    )
    let tool = MemoryRecallTool(transport: transport)
    #expect(tool.name == "recall_memory")

    let rendered = try await tool.call(
        arguments: .init(query: "eu data rules", domain: "legal")
    )

    // The query and domain actually reached the core — positional, because a
    // count of arguments proves nothing about WHICH ones.
    #expect(transport.seen.count == 1)
    #expect(transport.seen.first?.query == "eu data rules")
    #expect(transport.seen.first?.domain == "legal")

    // And the row came back with its provenance, not as prose and not as JSON.
    #expect(rendered.contains("id 42"))
    #expect(rendered.contains("domain legal"))
    #expect(rendered.contains("GDPR covers EU residents"))
    #expect(rendered.contains("confidence 0.93"))
    #expect(rendered.contains("valid from 2024-01-01T00:00:00Z"))
    #expect(!rendered.contains("{"), "tool output renders as citations, never raw JSON")
}

@available(macOS 26.0, *)
@Test("low_confidence renders as an answer that passed a gap through, not as an error")
func abstentionIsNotAnErrorOnTheSwiftSide() async throws {
    let tool = MemoryRecallTool(
        transport: StubTransport(
            outcome: .abstained(decision: .lowConfidence, confidence: 0.11)
        )
    )
    let rendered = try await tool.call(arguments: .init(query: "something absent"))

    #expect(rendered.contains("low_confidence"))
    #expect(rendered.lowercased().contains("no confident answer"))
    // The gap language the tool's own description promises to the model.
    #expect(rendered.contains("pass that through"))
    #expect(!rendered.lowercased().contains("error"))
    #expect(!rendered.lowercased().contains("no memory matched"))
}

@available(macOS 26.0, *)
@Test("a refusal renders differently from an absence, and leaks nothing about the rows")
func refusalIsNotAnAbsenceOnTheSwiftSide() async throws {
    let refusal = MemoryRecallTool(
        transport: StubTransport(outcome: .refused(status: 403, scope: "recall"))
    )
    let absence = MemoryRecallTool(
        transport: StubTransport(outcome: .absent(decision: .ok, confidence: 0.2))
    )

    let refused = try await refusal.call(arguments: .init(query: "q"))
    let absent = try await absence.call(arguments: .init(query: "q"))

    #expect(refused != absent, "a refusal and an absence must not render alike")
    #expect(refused.contains("403"))
    #expect(refused.lowercased().contains("refused"))
    // The three phrases that would collapse the distinction.
    #expect(!refused.contains("No memory matched"))
    #expect(!refused.lowercased().contains("no results"))
    #expect(!refused.lowercased().contains("nothing for it"))
    // And it says nothing about how many rows exist behind the refusal. The
    // marker is `"] id "` — the citation header the renderer emits — and not
    // `"id "`, which matched inside "d-id not complete" in the arm below and
    // made that arm pass by luck. A substring loose enough to hit an ordinary
    // English word is not an assertion.
    #expect(!refused.contains("] id "))
    #expect(!refused.contains("[1]"))
}

@available(macOS 26.0, *)
@Test("a transport that fails is degraded and stated, never an empty answer")
func aFailedTransportIsNotAnAbsence() async throws {
    let tool = MemoryRecallTool(
        transport: StubTransport(
            outcome: .answered(passages: [], decision: .ok, confidence: nil),
            failure: RecallTransportError.notWired
        )
    )
    let rendered = try await tool.call(arguments: .init(query: "q"))

    #expect(rendered.lowercased().contains("unavailable"))
    #expect(rendered.lowercased().contains("degraded"))
    #expect(!rendered.lowercased().contains("no memory matched"))
    #expect(!rendered.contains("] id "))
    #expect(!rendered.contains("[1]"))
}

@available(macOS 26.0, *)
@Test("the stub string is gone and no Swift path reaches the kernel except the core")
func noSwiftPathReachesTheKernelAndTheStubIsGone() throws {
    let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // AppleFMProbeTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // shell-macos
    let sources = root.appendingPathComponent("Sources/AppleFMProbe")

    // STRUCTURAL, and it is the guard for the trap the plan names: a Swift
    // bypass would be a second, unaudited path to private data, and it looks
    // like a refactor because the shorter version is "just fetch it here".
    // Read the source rather than trusting the seam: a protocol can be declared
    // and then quietly bypassed.
    var scanned = 0
    for file in try FileManager.default.contentsOfDirectory(
        at: sources, includingPropertiesForKeys: nil
    ) where file.pathExtension == "swift" {
        let text = try String(contentsOf: file, encoding: .utf8)
        scanned += 1

        // COMMENT LINES ARE EXCLUDED, and the reason is that these files
        // contain prose whose entire job is to state what is ABSENT — the
        // `KernelRecallTransport` doc names `URLSession` and a bearer token in
        // order to say this side has neither. Scanning the prose flagged the
        // guard at itself on its first run.
        //
        // That exclusion cannot hide a bypass, which is what makes it safe
        // rather than convenient: a comment does not execute, so a URL that
        // exists only inside one is not a path to private data. Code and string
        // literals are both still scanned, and a naive `//` strip would have
        // been WORSE than this — `"http://…"` in a string literal contains
        // `//`, so stripping from the first `//` would have deleted the host
        // out of a real URL and made the guard quietly weaker.
        let codeLines = text.split(separator: "\n", omittingEmptySubsequences: false)
            .filter { line in
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                return !trimmed.hasPrefix("//") && !trimmed.hasPrefix("/*")
                    && !trimmed.hasPrefix("*")
            }
        let flat = codeLines.joined(separator: " ")
            .split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")

        #expect(
            !flat.contains("no kernel transport"),
            "\(file.lastPathComponent) still carries the stub string"
        )
        for forbidden in [
            "URLSession", "URLRequest", "URL(string", "http://", "https://", "127.0.0.1",
            "8765", "localhost", "Authorization", "Bearer", "bearer",
        ] {
            #expect(
                !flat.contains(forbidden),
                """
                \(file.lastPathComponent) reaches the network or holds a credential \
                directly ("\(forbidden)"). The kernel is reached through the Rust \
                core's bounds and authz, and only the core's host bridge may do it.
                """
            )
        }
    }
    #expect(scanned >= 3, "the scan must have read the package's sources, found \(scanned)")

    // Anti-vacuity: the scan would pass on an empty directory, so assert the
    // files it DID read are the ones that carry the seam.
    let wire = try String(
        contentsOf: sources.appendingPathComponent("RecallWire.swift"), encoding: .utf8
    )
    #expect(wire.contains("protocol KernelRecallTransport"))
    #expect(wire.contains("case refused"))
    #expect(wire.contains("case abstained"))
}

@available(macOS 26.0, *)
@Test("the measurement path's inert transport fails loudly rather than answering")
func theInertTransportIsNotAStub() async {
    let transport = UnwiredRecallTransport()
    await #expect(throws: RecallTransportError.notWired) {
        try await transport.recall(query: "q", domain: nil)
    }

    // And through the tool it renders as degraded — the property that
    // distinguishes it from the stub string it replaces.
    let tool = MemoryRecallTool(transport: transport)
    let rendered = try? await tool.call(arguments: .init(query: "q"))
    #expect(rendered?.lowercased().contains("unavailable") == true)
    #expect(rendered?.contains("did not complete") == true)
}

#endif
