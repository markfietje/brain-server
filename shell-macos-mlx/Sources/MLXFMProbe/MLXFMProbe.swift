import Foundation

// `#huggingFaceLanguageModel` is a Swift macro from MLXHuggingFace, and
// `FoundationModels` types appear in PUBLIC signatures below — so the import
// has to be at file scope, not wrapped in `#if canImport` around the use site.
// (The same trap is recorded in `shell-macos`: a conditional around a call does
// not bring a module's types into scope, and the diagnostic reads "cannot find
// type" rather than "module not available".)
import FoundationModels
import MLXFoundationModels
import MLXHuggingFace
import MLXLLM
import MLXLMCommon
import HuggingFace

// `@preconcurrency`, not decoration: the target is already pinned to Swift 5
// language mode because `#huggingFaceLanguageModel` expands to a
// `Sendable` struct over a non-Sendable existential (see Package.swift). This
// target is Swift-6-clean for OUR code; the remaining diagnostic is inside
// Tokenizers, and this is the sanctioned way to say so at the import.
@preconcurrency import Tokenizers

/// A local MLX model, bridged into Apple's session API.
///
/// The claim this file exists to test is narrow and falsifiable: **an MLX model
/// can be used with the SAME `LanguageModelSession` as `SystemLanguageModel`,
/// including `Tool`s and `@Generable` guided output.** If that is true, the two
/// model tiers are one line apart in one language, and there is no sidecar, no
/// HTTP listener, and no second implementation of the memory tool schema.
///
/// What is deliberately NOT claimed: that any particular model is good. A model
/// that loads, reports a plausible tok/s and emits nonsense must fail loudly —
/// the DWQ 3-bit quant in the private plan set did exactly that at 56 tok/s.
@available(macOS 27.0, *)
public enum MLXFMProbe {

    /// The smallest Qwen that still exercises the real path.
    ///
    /// 0.6B cannot answer anything worth reading, and is not here to. It is here
    /// because a CI runner must download weights, and a 0.6 B 4-bit pack is ~400 MB
    /// against several GB for anything larger. Plumbing, not quality.
    public static let smallQwen = LLMRegistry.qwen3_0_6b_4bit

    /// Build the model. Loading and any download are LAZY — the returned value
    /// reports `.downloading` until weights are on disk, which is what
    /// `availability` exists to let a caller gate on.
    ///
    /// Capabilities are DECLARED, not inferred: the adapter defaults to
    /// `[.guidedGeneration]` and refuses anything not listed. Declaring
    /// `.toolCalling` is the whole point here — it is the claim under test.
    public static func model() async throws -> MLXLanguageModel {
        #huggingFaceLanguageModel(
            configuration: smallQwen,
            capabilities: [.toolCalling, .guidedGeneration])
    }

    /// One sentence about what this is, for the log.
    public static func describe(_ model: MLXLanguageModel) async -> String {
        switch await model.availability {
        case .available:
            "MLXLanguageModel: available"
        case .downloading:
            "MLXLanguageModel: downloading weights"
        case let .unavailable(reason):
            "MLXLanguageModel: unavailable — \(reason)"
        @unknown default:
            "MLXLanguageModel: unknown availability"
        }
    }
}