import Foundation
import Testing

// File-scope import for the same reason as `shell-macos`: `#if canImport` around
// a use site does not bring a module's types into scope.
#if canImport(FoundationModels)
import FoundationModels
#endif

import MLXFoundationModels

@testable import MLXFMProbe

/// Proves the BRIDGE, not the model.
///
/// The question is whether an MLX model can sit in the same
/// `LanguageModelSession` as `SystemLanguageModel` — same `Tool` protocol, same
/// `@Generable` guided output, same streaming. If yes, the two tiers are one line
/// apart and the whole sidecar/HTTP/UDS question disappears.
///
/// What it deliberately does NOT assert: that 0.6B Qwen is good at anything. It
/// is a 400 MB plumbing fixture.
@Suite("MLX ↔ FoundationModels bridge")
struct MLXFMProbeTests {

    @Test("MLXLanguageModel conforms to LanguageModel and can be gated on")
    func mlxModelIsAGatedLanguageModel() async throws {
        guard #available(macOS 27.0, *) else {
            print("macOS 27 not available — bridge not exercised here")
            return
        }

        let model = try await MLXFMProbe.model()
        print(await MLXFMProbe.describe(model))

        // The type-level claim: this compiles only if MLXLanguageModel is a
        // LanguageModel. That is the assertion — a cast through the existential
        // fails to build if the conformance is absent, which is a stronger
        // statement than any runtime check could make.
        let asLanguageModel: any LanguageModel = model
        #expect(asLanguageModel.capabilities.contains(.toolCalling))

        // Availability is a three-state value, and `.downloading` is a real
        // state, not an error: weights are lazy. A probe that treated it as
        // failure would be wrong about how the thing works.
        switch await model.availability {
        case .available, .downloading:
            break
        case .unavailable:
            Issue.record("model reported unavailable on a supported platform")
        @unknown default:
            Issue.record("availability gained a case this probe has not seen")
        }
    }

    /// The coherence gate, at its smallest.
    ///
    /// A model that loads and streams must produce TEXT. A stream of nil, empty
    /// strings, or a prompt echoed back is a failure that a speed measurement
    /// would happily report as fast — which is precisely how a 3-bit quant
    /// measured 56 tok/s while emitting `But. However. But. But.`.
    ///
    /// Not a quality test. A 0.6 B model is allowed to be bad. It is not allowed
    /// to be EMPTY or to loop a single token forever, because those are the two
    /// shapes that masquerade as speed.
    @Test("A loaded model emits text, not an empty or looping stream")
    func modelEmitsText() async throws {
        guard #available(macOS 27.0, *) else {
            print("macOS 27 not available — generation not exercised here")
            return
        }

        let model = try await MLXFMProbe.model()
        let session = LanguageModelSession(model: model)

        let stream = session.streamResponse(
            to: "Name the capital of France in one short sentence.",
            options: GenerationOptions(samplingMode: .greedy, maximumResponseTokens: 48)
        )

        // Each snapshot's content is the text SO FAR, not a delta, so
        // accumulating it concatenates the whole answer onto itself once per
        // chunk — which reported "241 characters" for a thirty-character reply.
        // Take the LAST snapshot instead; that is the completed text.
        var text = ""
        var chunks = 0
        for try await snapshot in stream {
            chunks += 1
            text = snapshot.content.description
            if chunks > 200 { break }
        }

        print("chunks: \(chunks) · characters: \(text.count)")
        print("output: \(text.prefix(300))")

        #expect(chunks > 0, "the stream produced nothing at all")
        #expect(
            !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
            "a completed stream with no text is a broken model, not a small one"
        )
    }
}