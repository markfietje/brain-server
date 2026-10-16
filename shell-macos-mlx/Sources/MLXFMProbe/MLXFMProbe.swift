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

    /// The CI model: the smallest Qwen that still exercises the real path.
    ///
    /// 0.6B cannot answer anything worth reading, and is not here to. It is here
    /// because a CI runner must download weights, and a 0.6 B 4-bit pack is ~400 MB
    /// against 6 GB for `reasoningQwen`. Plumbing, not quality.
    public static let ciModel = LLMRegistry.qwen3_0_6b_4bit

    /// The MEASURED model: the one Plan 03 §8.6 nominates.
    ///
    /// `mlx-community/Qwen3.5-9B-4bit` — 5.98 GB, standard affine 4-bit weights
    /// and NOT a Hadamard-rotated basis, which is precisely what made Bonsai 2
    /// and the LFM2.5 DWQ quant unloadable on a stock runtime. `qwen3_5`, so
    /// `Qwen35.swift` in mlx-swift-lm reads it; 262,144 declared context.
    ///
    /// NOT a registry entry. `LLMRegistry` offers `qwen3_5_2b_4bit`,
    /// `qwen3_5_27` and `qwen3_5_35` — the 9B is not among them, which is why
    /// this is built from a repo id via `ModelConfiguration(id:)` rather than
    /// guessed at as `qwen3_5_9b_4bit`. That name was written down first and
    /// does not exist.
    ///
    /// `reasoningConfig` is `.thinkTagsWithEnableThinking` — the preset whose
    /// doc comment describes exactly this family: generic <think> delimiters
    /// toggled by the `enable_thinking` chat-template flag, default on. Qwen3.5
    /// "operates in thinking mode by default", and this is the gap AFM 3 Core
    /// leaves: Core reports toolCalling, guidedGeneration and vision, and NO
    /// reasoning.
    ///
    /// `tokenizerSource` is OVERRIDDEN to the official repo, and that is load
    /// bearing rather than belt-and-braces: `mlx-community`'s Qwen3.5 builds ship
    /// `"tokenizer_class": "TokenizersBackend"` in `tokenizer_config.json` —
    /// which is not a tokenizer class but the loader's own backend enum name, so
    /// the runtime looks for a tokenizer by that name, finds none, and throws
    /// `.unsupportedTokenizer("TokenizersBackend")` naming the bad value verbatim.
    /// Both `mlx-community/Qwen3.5-9B-4bit` and `…-2B-4bit` are affected;
    /// `Qwen/Qwen3.5-9B` and `lmstudio-community/Qwen3.5-9B-MLX-4bit` both say
    /// `Qwen2Tokenizer`. Weights from the quantised repo, tokenizer from upstream
    /// — so this survives mlx-community fixing their config, and says why it is
    /// there if they do not.
    public static let reasoningQwen = ModelConfiguration(
        id: "mlx-community/Qwen3.5-9B-4bit",
        tokenizerSource: .id("Qwen/Qwen3.5-9B"),
        // The registry's own Qwen3.5 entries carry this and the first version
        // here omitted it: without it the SECOND turn on a session throws
        // `.runtime("No user query found in messages.")`. The empty string is
        // not a typo — it is the family's EOT marker, and the chat template
        // cannot place a user message without it.
        extraEOSTokens: [""],
        reasoningConfig: .thinkTagsWithEnableThinking)

    /// The SAME weights, thinking switched OFF. One field apart, so the comparison
    /// is worth something.
    ///
    /// Added to settle an A/B that made `reasoningQwen` look unusable for the
    /// product's actual question. With thinking on, "What are the benefits of green
    /// tea?" yields ZERO snapshots bare and throws
    /// `.runtime("No user query found in messages.")` with a tool attached — while
    /// "What is the capital of France?" answers coherently BOTH ways. Same session
    /// construction, same weights, same process, and tools are provably fine
    /// because the short prompt used them successfully.
    ///
    /// So the suspect is the thinking path: reasoning tokens the adapter never
    /// surfaces as content snapshots, and a template that stops placing the user
    /// turn once it starts one. An open-ended question is exactly what triggers it,
    /// which is why the short probes passed and every real use case would not.
    /// `reasoningQwen` with the empty-string EOT token REMOVED. The A/B that says
    /// whether that hack is helping or hurting.
    public static let reasoningQwenNoEOT = ModelConfiguration(
        id: "mlx-community/Qwen3.5-9B-4bit",
        tokenizerSource: .id("Qwen/Qwen3.5-9B"),
        reasoningConfig: .thinkTagsWithEnableThinking)

    /// `reasoningQwen` at all — the registry's own entry, untouched.
    public static let qwenFromRegistry = ModelConfiguration(
        id: "mlx-community/Qwen3.5-9B-4bit")

    public static let reasoningQwenNoThinking = ModelConfiguration(
        id: "mlx-community/Qwen3.5-9B-4bit",
        tokenizerSource: .id("Qwen/Qwen3.5-9B"),
        extraEOSTokens: [""],
        reasoningConfig: ReasoningConfig(
            startDelimiter: "<think>",
            endDelimiter: "</think>",
            promptStrategy: .templateFlag(key: "enable_thinking", defaultOn: false),
            isSpecialToken: false))

    /// Build the model. Loading and any download are LAZY — the returned value
    /// reports `.downloading` until weights are on disk, which is what
    /// `availability` exists to let a caller gate on.
    ///
    /// Capabilities are DECLARED, not inferred: the adapter defaults to
    /// `[.guidedGeneration]` and refuses anything not listed. Declaring
    /// `.toolCalling` is the whole point here — it is the claim under test.
    public static func model(
        configuration: ModelConfiguration = ciModel,
        capabilities: [LanguageModelCapabilities.Capability] = [.toolCalling, .guidedGeneration]
    ) async throws -> MLXLanguageModel {
        #huggingFaceLanguageModel(
            configuration: configuration,
            capabilities: capabilities)
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