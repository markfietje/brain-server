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