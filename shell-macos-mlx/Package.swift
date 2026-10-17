// swift-tools-version: 6.2
//
// A SEPARATE package from `shell-macos`, on purpose.
//
// `shell-macos` is Apple's framework surface and builds in ~0.3 s. Its lane
// asserts the toolchain, so a slow build there would cost every run and teach
// nothing. MLX compiles its C++ backend (`Cmlx`) from vendored source INSIDE
// the SwiftPM graph — that is a many-minutes build, once. Keeping it in its own
// package means the cheap lane stays cheap and the expensive one is explicit.
//
// WHAT THIS PROVES. `MLXFoundationModels` is an MLX adapter conforming to
// Apple's `FoundationModels.LanguageModel`, so an MLX model is usable directly
// with `LanguageModelSession` — same `Tool`s, same `@Generable` guided output,
// same streaming as `SystemLanguageModel`. That makes the two model tiers one
// line apart, in one language, with no sidecar, no HTTP listener, and no second
// implementation of the tool schema.
//
// WHAT THIS DOES NOT PROVE — and the sentence this file used to get wrong.
// An earlier version of this comment claimed the adapter gave "the same
// transcript" as `SystemLanguageModel`. For MULTI-TURN that is FALSE, and it is
// false for a reason that is now measured rather than suspected: with
// Qwen3.5-9B-4bit, turn 2 throws `runtime("No user query found in messages.")`
// on BOTH `LanguageModelSession` AND the mlx-native `ChatSession`. The
// adapter's one-tool-call session works; multi-turn memory does not.
//
// The single-tool-call path is therefore the whole of what has been shown to
// work. Anything that carries history across turns is a DIFFERENT claim and
// currently a negative one. Qwen3-0.6B-4bit does pass multi-turn, with a cold
// control; Qwen3.5-9B does not, on either surface.
//
// The weights are not implicated. The cause is NOT ESTABLISHED — an earlier
// version of this comment asserted it was the MLX executor's prefix matching,
// which the same-symmetry evidence does not license, since mlx-native
// `ChatSession` never goes through a `LanguageModelExecutor` at all. The single
// diagnosis, its evidence and its two hypotheses live in ONE place:
// `shell-macos/Sources/AppleFMProbe/ChatModelLayer.swift`, on `ChatModelId`.
// This file does not restate it, because it was restated four times and the
// copies drifted apart from the evidence.
//
// Until the cause is settled, Qwen3.5 is excluded from any chat path by a pin,
// not by convention.
//
// UPSTREAM, recorded rather than patched here: `ChatSession.swift:129` in
// mlx-swift-lm documents a `loadModelContainer(id:)` overload that does not
// exist. That is upstream's file to fix, so it is a note and not a local edit.
//
// The architectures that were ruled out on memory grounds do not come back here:
// Bonsai and the LFM2.5 DWQ quant need a FORK of MLX for their Hadamard-rotated
// weights, and this is stock MLX. The point of the probe is the Qwen path, which
// is standard weights.
import PackageDescription

let package = Package(
    name: "MLXFMProbe",
    platforms: [
        // Matches `shell-macos`. The FM adapter itself needs the 27.0 SDK and is
        // guarded at the USE SITE — a package cannot raise its floor to 27
        // without making the package unusable everywhere below it.
        .macOS(.v15),
    ],
    products: [
        .library(name: "MLXFMProbe", targets: ["MLXFMProbe"]),
        .executable(name: "banner-experiment", targets: ["banner-experiment"]),
        .executable(name: "multi-turn-probe", targets: ["multi-turn-probe"]),
    ],
    dependencies: [
        // Default-on trait `FoundationModelsIntegration` gates MLXFoundationModels.
        .package(url: "https://github.com/ml-explore/mlx-swift-lm", from: "3.32.3"),
        // Not optional conveniences. `#huggingFaceLanguageModel` expands to code
        // that names `HubClient`/`HubCache` (module `HuggingFace`) and
        // `AutoTokenizer` (module `Tokenizers`). Without these two the macro
        // fails with `cannot find 'HuggingFace' in scope` — a diagnostic that
        // reads like a missing package rather than a missing import, which is
        // why it is recorded here rather than rediscovered.
        .package(url: "https://github.com/huggingface/swift-huggingface", from: "0.1.0"),
        .package(url: "https://github.com/huggingface/swift-transformers", from: "0.1.0"),
    ],
    targets: [
        .target(
            name: "MLXFMProbe",
            // Swift 5 language mode, and NOT a style preference.
            //
            // `#huggingFaceLanguageModel` expands to
            // `struct TokenizerBridge: Sendable { let upstream: any Tokenizer }`
            // (upstream `#adaptHuggingFaceTokenizer`). `any Tokenizer` is not
            // Sendable, so under Swift 6 language mode the macro's own output
            // fails to compile:
            //
            //   stored property 'upstream' of 'Sendable'-conforming struct
            //   'TokenizerBridge' has non-Sendable type 'any Tokenizer'
            //
            // That is a defect in mlx-swift-lm 3.32.3, reachable from a correct
            // consumer, and this is the consumer-side workaround pending an
            // upstream fix. Recorded here so the next reader does not remove it
            // as dead strictness and rediscover the wall.
            dependencies: [
                .product(name: "MLXFoundationModels", package: "mlx-swift-lm"),
                .product(name: "MLXLLM", package: "mlx-swift-lm"),
                .product(name: "MLXLMCommon", package: "mlx-swift-lm"),
                .product(name: "MLXHuggingFace", package: "mlx-swift-lm"),
                .product(name: "HuggingFace", package: "swift-huggingface"),
                // `Tokenizers` is not a standalone product at the resolved 0.1.24 —
                // `Transformers` vends the same target, and SwiftPM makes every
                // target in a product importable. Upstream main does expose it
                // separately; this pins to what resolves today.
                .product(name: "Transformers", package: "swift-transformers"),
            ],
            swiftSettings: [.swiftLanguageMode(.v5)],
        ),
        // The banner experiment is an EXECUTABLE, not a test: it loads a 6 GB
        // model, so putting it behind `swift test` would make the package's test
        // lane download 6 GB and run for minutes on every invocation. It is a
        // measurement instrument, invoked deliberately.
        .executableTarget(
            name: "banner-experiment",
            dependencies: ["MLXFMProbe"],
            swiftSettings: [.swiftLanguageMode(.v5)],
        ),
        // Multi-turn memory probe. An executable like the banner experiment: it
        // loads a 6 GB model, so `swift test` is the wrong home. It asks the CHAT
        // product's question -- can the model carry context across turns -- which
        // the banner experiment did NOT (that used a fresh session per turn,
        // correct for a paired A/B and the wrong shape for chat).
        .executableTarget(
            name: "multi-turn-probe",
            dependencies: ["MLXFMProbe"],
            swiftSettings: [.swiftLanguageMode(.v5)],
        ),
        .testTarget(name: "MLXFMProbeTests", dependencies: ["MLXFMProbe"]),
    ]
)
