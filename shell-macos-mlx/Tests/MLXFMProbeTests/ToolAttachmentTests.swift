import Foundation
import Testing

#if canImport(FoundationModels)
import FoundationModels
#endif

import MLXLMCommon
@testable import MLXFMProbe

/// A/B: the SAME model, the SAME session construction, varying only the prompt and
/// whether a tool is attached.
///
/// This exists because two tests failed two DIFFERENT ways on the same candidate,
/// and every reading I tried first was wrong:
///
///   - harness (no tools): 0 snapshots, no error → I blamed model residency, then
///     tools-on-session, then `@MainActor` — which made it worse
///   - end-to-end (with a tool): `.runtime("No user query found in messages.")`
///
/// `Measure.swift` also builds `LanguageModelSession(model:)` and streams the same
/// way, and it answers at 52.81 tok/s. So the model is not the variable and the
/// session construction is not the variable. That leaves the PROMPT, and the TEST
/// CONTEXT — and this runs all four combinations in one process with the weights
/// already resident, so there is nothing left to guess at.
@Suite("Tool attachment", .serialized)
struct ToolAttachmentTests {

    @Test("Qwen3.5-9B: prompt x tool-attachment, all four arms",
          .enabled(if: ProcessInfo.processInfo.environment["BRAIN_FM_MEASURE"] == "1"),
          .timeLimit(.minutes(25)))
    func promptByToolAttachment() async throws {
        guard #available(macOS 27.0, *) else { return }

        // Five prompts spanning the axis: short/long, closed/open, same topic/different.
        let prompts = [
            "What is the capital of France?",
            "Name three benefits of green tea.",
            "What are the benefits of green tea?",
        ]
        // Same weights, one field apart: `.thinkTagsWithEnableThinking` vs an
        // explicit off. The A/B that matters.
        let configs: [(String, ModelConfiguration)] = [
            ("as-shipped", MLXFMProbe.reasoningQwen),
        ]

        for (label, config) in configs {
        let cfgModel = try await MLXFMProbe.model(
            configuration: config,
            capabilities: [.toolCalling, .guidedGeneration, .reasoning])

        for prompt in prompts {
            for withTool in [true] {
                let tag = "[\(label)] tool=\(withTool)"
                do {
                    let session = withTool
                        ? LanguageModelSession(model: cfgModel, tools: [GreenTeaFacts()])
                        : LanguageModelSession(model: cfgModel)
                    var chunks = 0
                    var text = ""
                    let stream = session.streamResponse(
                        to: prompt,
                        options: GenerationOptions(samplingMode: .greedy,
                                                   maximumResponseTokens: 2048))
                    for try await snap in stream {
                        chunks += 1
                        text = snap.content.description
                    }
                    let deg = Degeneracy.inspect(text)
                    print("-- \(tag) -> chunks=\(chunks) chars=\(text.count) \(deg.verdict.rawValue)")
                    print("     \(text.prefix(200).replacingOccurrences(of: "\n", with: " "))")
                } catch {
                    print("-- \(tag) -> THREW \(error)")
                }
            }
        }
        }
    }
}