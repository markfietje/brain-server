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

            let tools: [any Tool] = [MemoryRecallTool()]
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
    guard #available(macOS 26.0, *) else { return }
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
        // The budget was set from measurement, not taste. Characterising this
        // arm across runs: under greedy sampling the first-attempt hit rate is
        // high but not 1.0, and the misses are concentrated rather than random.
        // Two attempts was NOT enough (observed: an arm missing both). The
        // count printed below is what the budget should be re-derived from if
        // this ever goes red again.
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
    // (That is the same fact that makes PCC reachable only through a profile.)
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
        let plan = controller.currentPlan
        let seen = plan.apply(Array(session.transcript))
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
// turn."* Trimming to the MINIMUM every turn — which the first version of
// `HistoryBudget` did, and which its own comment called "the point" — changes
// the kept set on every request and re-prefills the whole history each time.
//
// That cost is not observable in this environment: `usage.input.cachedTokenCount`
// reads 0 on this build even for a plain warm second turn (measured, twice), so
// there is no cache signal to assert against. Asserting TTFT instead would be
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
    let perTurn = 220
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

// HISTORY ACROSS A PROFILE SWITCH — the mechanism escalation depends on.
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
