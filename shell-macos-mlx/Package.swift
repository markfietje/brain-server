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
// same transcript, same streaming as `SystemLanguageModel`. That makes the two
// model tiers one line apart, in one language, with no sidecar, no HTTP
// listener, and no second implementation of the tool schema.
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
        .library(name: "MLXFMProbe", targets: ["MLXFMProbe"])
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
        .testTarget(name: "MLXFMProbeTests", dependencies: ["MLXFMProbe"]),
    ]
)
