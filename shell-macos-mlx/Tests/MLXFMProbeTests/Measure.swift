import Foundation
import Testing

#if canImport(FoundationModels)
import FoundationModels
#endif

@testable import MLXFMProbe

/// LOCAL measurement of the 6 GB model. Not the CI test.
///
/// `.timeLimit(.minutes(20))` and `.enabled(if: ProcessInfo.processInfo
/// .environment["BRAIN_FM_MEASURE"] == "1")` — so `swift test` on a runner and on
/// a laptop stays the 0.6 B plumbing check, and this runs only when asked:
/// `BRAIN_FM_MEASURE=1 swift test --filter measure`.
@Suite("MLX ↔ FoundationModels bridge", .serialized)
struct MLXFMMeasure {

    @Test(
        "Measure Qwen3.5-9B: cold/warm TTFT, decode rate, coherence",
        .enabled(if: ProcessInfo.processInfo.environment["BRAIN_FM_MEASURE"] == "1"),
        .timeLimit(.minutes(20)))
    func measureReasoningQwen() async throws {
        guard #available(macOS 27.0, *) else { return }

        let model = try await MLXFMProbe.model(
            configuration: MLXFMProbe.reasoningQwen,
            capabilities: [.toolCalling, .guidedGeneration, .reasoning])
        let session = LanguageModelSession(model: model)

        func turn(_ prompt: String, _ label: String, _ maxTokens: Int) async throws -> String {
            let start = ContinuousClock.now
            var firstToken: Double = 0
            var text = ""
            var chunks = 0
            let stream = session.streamResponse(
                to: prompt,
                options: GenerationOptions(samplingMode: .greedy, maximumResponseTokens: maxTokens))
            for try await snapshot in stream {
                if chunks == 0 {
                    firstToken = Self.seconds(since: start)
                }
                chunks += 1
                text = snapshot.content.description
            }
            let total = Self.seconds(since: start)
            let decode = total - firstToken
            let rate = (chunks > 0 && decode > 0) ? Double(chunks) / decode : 0
            print(String(
                format: "  [%@] first token %6.1f ms · total %7.1f ms · decode %6.2f tok/s · %d chunks",
                label, firstToken * 1000, total * 1000, rate, chunks))
            return text
        }

        // COLD — nothing resident.
        let cold = try await turn(
            "Name the capital of France in one short sentence.", "cold", 256)

        // WARM — same session, so the prefix is a cache candidate.
        let warm = try await turn(
            "Name the capital of France in one short sentence. Now, in two sentences, "
                + "why a provenance chain makes a stored memory verifiable.", "warm", 256)

        // LONG — a prefix long enough that a cache, if there is one, engages.
        let longPrompt = String(repeating: """
            A provenance chain links every stored memory to the hash of the content it \
            was written from, and to the hash of the chain that preceded it. Any edit, \
            deletion or expiry is itself a chained event, so the history of a fact is \
            reconstructible and a claim about what was stored at a past instant is \
            checkable rather than asserted.

            """, count: 8)
        let long = try await turn(longPrompt + "\nIn two sentences, what does that buy?", "long", 256)

        print("\n── coherence: the gate that caught the DWQ ──")
        for (label, out) in [("cold", cold), ("warm", warm), ("long", long)] {
            print("  [\(label)] \(out.prefix(220).replacingOccurrences(of: "\n", with: " "))")
        }

        // COHERENCE BEFORE SPEED. A broken quant streams fast and says nothing;
        // that is exactly how the DWQ measured 56 tok/s while emitting
        // "But. However. But." Every tok/s figure above is void if these fail.
        for (label, out) in [("cold", cold), ("warm", warm), ("long", long)] {
            let trimmed = out.trimmingCharacters(in: .whitespacesAndNewlines)
            #expect(!trimmed.isEmpty, "\(label): a completed stream with no text is a broken model")
            #expect(
                trimmed.count > 40,
                "\(label): \(trimmed.count) chars is too short to be a considered answer")
            let words = trimmed.lowercased().split(whereSeparator: { !$0.isLetter })
            #expect(
                words.count > 8,
                "\(label): \(words.count) words — degenerate output, not a slow model")
            let (top, repeats) = Dictionary(grouping: words, by: { $0 })
                .map { ($0.key, $0.value.count) }
                .max { $0.1 < $1.1 }!
            #expect(
                repeats <= max(3, words.count / 4),
                "\(label): \"\(top)\" appears \(repeats)/\(words.count) times — a repetition loop")
        }
    }

    static func seconds(since start: ContinuousClock.Instant) -> Double {
        let c = (ContinuousClock.now - start).components
        return Double(c.seconds) + Double(c.attoseconds) / 1e18
    }
}
