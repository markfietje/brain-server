import Foundation

#if canImport(FoundationModels)
import FoundationModels
#endif

// MARK: - Capability report

/// What this build can actually do, decided at runtime rather than assumed.
///
/// Every value here is a MEASUREMENT, not a capability claim. The whole
/// point of the macOS lane is that a green compile means nothing on its own —
/// a `@available` guard that never fires compiles the same code out of
/// existence and exits 0. So the surfaces below report what the framework
/// said, and a caller can tell "not entitled" from "not compiled".
public struct AppleFMCapabilities: Sendable, Equatable {
    /// The on-device model, or the reason it is absent.
    public enum OnDevice: Sendable, Equatable {
        case available(contextSize: Int, languages: Int, variant: String, capabilities: [String])
        case unavailable(reason: String)
    }

    /// Private Cloud Compute. Device-eligibility is NOT entitlement.
    public enum PrivateCloud: Sendable, Equatable {
        case eligibleButUnentitled
        case unavailable(reason: String)
    }

    public let onDevice: OnDevice
    public let privateCloud: PrivateCloud
    public let supportsReasoningLevels: Bool
    public let supportsToolChoice: Bool

    /// Combined input+output window the on-device model will honour.
    public var contextBudget: Int? {
        if case let .available(contextSize, _, _, _) = onDevice { return contextSize }
        return nil
    }
}

// MARK: - Probe

/// Reads the framework's real answers.
///
/// The private-framework calls a 4096-token model does not reveal: PCC's
/// `isAvailable` reports DEVICE eligibility and the request then fails on the
/// APP entitlement with `ModelManagerError 1046`. Both facts are surfaced
/// separately so no caller can read the flag as a capability.
public enum AppleFMProbe {
    public static func capabilities() -> AppleFMCapabilities {
        #if canImport(FoundationModels)
        if #available(macOS 26.0, *) {
            let model = SystemLanguageModel.default
            let onDevice: AppleFMCapabilities.OnDevice
            switch model.availability {
            case .available:
                // `variant` and `capabilities` are macOS 27 surfaces. Before 27
                // `contextSize` is back-deployed to a hard 4096 and the whole
                // variant/capability vocabulary does not exist, so the pre-27
                // branch reports the window and nothing it cannot know.
                var variant = "unreported (pre-macOS 27)"
                var capabilities: [String] = []
                if #available(macOS 27.0, *) {
                    variant = model.variant.displayName
                    capabilities = Self.capabilityNames(model.capabilities)
                }
                onDevice = .available(
                    contextSize: ChatWindow.onDevice().tokens ?? 0,
                    languages: model.supportedLanguages.count,
                    variant: variant,
                    capabilities: capabilities
                )
            case let .unavailable(reason):
                onDevice = .unavailable(reason: String(describing: reason))
            }

            var privateCloud: AppleFMCapabilities.PrivateCloud = .unavailable(
                reason: "Private Cloud Compute requires macOS 27"
            )
            if #available(macOS 27.0, *) {
                let pcc = PrivateCloudComputeLanguageModel()
                privateCloud = pcc.isAvailable
                    ? .eligibleButUnentitled
                    : .unavailable(reason: "device not eligible")
            }
            // VERBOSE-ONLY: print the raw PCC facts, including the CONTEXT
            // SIZE, so the 32k claim is measured on this machine rather than
            // taken from a WWDC slide. `isAvailable` is an ELIGIBILITY flag; it
            // is deliberately NOT reported as capability (the row above is
            // pinned to that), so this line is the only place the two are kept
            // apart.
            if #available(macOS 27.0, *),
               ProcessInfo.processInfo.environment["APPLEFM_VERBOSE"] == "1" {
                let pcc = PrivateCloudComputeLanguageModel()
                print("--- AppleFMProbe verbose ---")
                // `contextSize` is async on PCC (it is a NETWORK model, so the
                // window is a property of the server-side deployment, not of the
                // binary). That is itself a fact worth having: the on-device
                // window is a local constant, PCC's is a remote answer.
                //
                // ONE-PATH-EXEMPTION BEGIN — the ONLY reads of `.contextSize`
                // outside `ChatWindow`, and they are here because routing them
                // through the seam would erase the very difference this block
                // exists to print: one is `await`, one is not. `ChatWindow` is
                // the seam for CALLERS; this is the seam's own test fixture.
                // `contextSizeIsReadThroughOnePath` scans for reads outside the
                // exemption and outside `ChatModelLayer.swift`, and fails if
                // this marker is deleted while the reads remain.
                print("PCC isAvailable: \(pcc.isAvailable)")
                Task {
                    let pccSize = try? await pcc.contextSize
                    print("PCC contextSize: " + String(describing: pccSize))
                }
                print("on-device contextSize: \(model.contextSize)")
                // ONE-PATH-EXEMPTION END
            }

            return AppleFMCapabilities(
                onDevice: onDevice,
                privateCloud: privateCloud,
                supportsReasoningLevels: true,
                supportsToolChoice: true
            )
        }
        #endif
        return AppleFMCapabilities(
            onDevice: .unavailable(reason: "FoundationModels unavailable"),
            privateCloud: .unavailable(reason: "FoundationModels unavailable"),
            supportsReasoningLevels: false,
            supportsToolChoice: false
        )
    }

    /// The 27-only capability vocabulary, as names, so a caller can log or
    /// assert on it without importing the framework.
    ///
    /// `Capability` has exactly four cases — `toolCalling`, `guidedGeneration`,
    /// `reasoning`, `vision` — and every one is `@available(macOS 27.0, *)`, so
    /// this helper carries the same guard. The list is deliberately exhaustive:
    /// a new case added by Apple should fail to compile here rather than be
    /// silently reported as absent.
    @available(macOS 27.0, *)
    private static func capabilityNames(_ capabilities: LanguageModelCapabilities) -> [String] {
        var names: [String] = []
        if capabilities.contains(.toolCalling) { names.append("toolCalling") }
        if capabilities.contains(.guidedGeneration) { names.append("guidedGeneration") }
        if capabilities.contains(.reasoning) { names.append("reasoning") }
        if capabilities.contains(.vision) { names.append("vision") }
        return names
    }
}

// MARK: - The memory tool shape

#if canImport(FoundationModels)
/// The shape the shell's memory tools take.
///
/// `Tool` is GENERIC over `Arguments` and `Output` as of the current SDK —
/// the published WWDC25 example (`func call(arguments:) -> ToolOutput`) does
/// not compile, and `ToolOutput` is a `Transcript` entry rather than a return
/// type. That is recorded here as a compile-time fact rather than a comment,
/// so the next person to port an example from the docs finds the real
/// signature where they are already working.
///
/// The tool surface is deliberately FOUR. Measured on an M1 Pro, four tool
/// schemas cost ~442 of a 4096-token window; the same four expressed as the
/// 21-tool surface some on-device MCP servers ship cost ~1405 — a third of
/// the window spent before the user types. Breadth lives behind these
/// routers, not beside them.
@available(macOS 26.0, *)
public struct MemoryRecallTool: Tool {
    @Generable
    public struct Arguments: Sendable {
        @Guide(description: "What to search the user's private memory for.")
        public var query: String

        @Guide(description: "Optional domain scope to narrow the search.")
        public var domain: String?

        public init(query: String, domain: String? = nil) {
            self.query = query
            self.domain = domain
        }
    }

    // The framework models a tool's return as anything `PromptRepresentable`;
    // `String` satisfies it. `ToolOutput` does not — it is a transcript entry.
    public typealias Output = String

    public let name: String
    public let description: String

    public init() {
        self.name = "recall_memory"
        // Shortened, and the MEASUREMENT corrects the expectation: the cut
        // saved 6 tokens (310 → 304), not the ~250 it looked like it should.
        // Almost all of a tool's cost is the `@Generable` schema the macro
        // emits, not the prose — so "shorten the description" is close to a no-op
        // and anyone budgeting a large tool surface by trimming descriptions is
        // optimising the wrong term. Kept anyway because it is free and the
        // guidance that matters ("pass a low-confidence gap through rather than
        // filling it") survives; the lesson recorded is the 6.
        self.description = """
            Recall ranked passages from the user's private memory. Returns \
            confidence; if confidence is low the store has no answer — pass \
            that through rather than filling the gap.
            """
    }

    /// - Returns: the retrieval result, confidence and provenance included.
    ///   The kernel is the only thing that may decide this; the model never does.
    public func call(arguments: Arguments) async throws -> String {
        // Intentionally a stub. The real implementation is a typed-wire call
        // to the brain-server kernel, and it belongs behind the Rust core's
        // bounds and authz — not here, where nothing enforces them.
        "no kernel transport wired yet"
    }
}
#endif

// MARK: - Token budget

/// What the on-device window actually costs, measured rather than budgeted.
///
/// AFM 3 Core reports a COMBINED input+output window, not separate budgets,
/// and the tool schemas live inside it. Getting this wrong is how a chat
/// surface ships and then dies mid-sentence with `context-window-exceeded`.
///
/// The availability annotation is on the TYPE, not just the body: `Instructions`
/// appears in this API's signature, so guarding only the body leaves a public
/// surface that will not compile at the deployment target the package declares.
/// That is the first error this package produced, and it is recorded here
/// because the next bridge will hit the same shape.
@available(macOS 26.0, *)
public enum ContextBudget {
    public struct Cost: Sendable, Equatable {
        public let instructions: Int
        public let toolSchemas: Int
        public var total: Int { instructions + toolSchemas }
        public var remaining: Int? {
            guard let budget = AppleFMProbe.capabilities().contextBudget else { return nil }
            return budget - total
        }
    }

    public static func measure(
        instructions: Instructions,
        tools: [any Tool]
    ) async throws -> Cost {
        #if canImport(FoundationModels)
        if #available(macOS 26.4, *) {
            let model = SystemLanguageModel.default
            return Cost(
                instructions: try await model.tokenCount(for: instructions),
                toolSchemas: try await model.tokenCount(for: tools)
            )
        }
        #endif
        return Cost(instructions: 0, toolSchemas: 0)
    }

    /// Tokens held back for the model's REPLY.
    ///
    /// Apple's context-window article: the window is a single combined pool —
    /// *"All input to the model contributes tokens to the context window … and
    /// the model's responses"* — and `maximumResponseTokens`, when left unset,
    /// means the model is *"allowed to produce the longest answer its context
    /// size supports."* Input and output draw from the same 4,096.
    ///
    /// So a budget that hands the whole remaining window to history is wrong by
    /// construction: it fills every input token and leaves nothing for the
    /// answer, and the failure mode is `contextSizeExceeded` raised MID-GENERATION
    /// rather than at the start of the turn.
    ///
    /// This is a RESERVE, not a cap. Apple is explicit that a tight
    /// `maximumResponseTokens` is the wrong tool — it *"can cause the model to
    /// generate incomplete or grammatically incorrect responses, like 'A cat is
    /// a small.'"* — and that prompt-level constraints ("in three sentences")
    /// are the better length control. The reserve is the floor that makes any
    /// answer possible; the cap is a runaway guard set separately and far above
    /// it (see `responseRunawayCap`).
    public static let responseReserve = 1024

    /// The runaway guard for `maximumResponseTokens`, deliberately far above the
    /// reserve. It exists to stop an unbounded generation, not to shape answers
    /// — a low cap truncates mid-sentence, which is the failure Apple documents.
    public static let responseRunawayCap = 2048

    /// The conversation budget for the shell's ACTUAL surface — its
    /// instructions and its one recall tool — rather than for whatever a caller
    /// happens to pass in.
    ///
    /// This exists because `ChatRouter` needs a budget and a budget is only
    /// meaningful for a real tool set: the number is `contextSize −
    /// instructions − tools − responseReserve`, and the last three are
    /// properties of the PRODUCT, not of a caller. A router constructed from an
    /// arbitrary `Cost` would budget against a tool surface nobody ships.
    ///
    /// Returns nil when the measurement cannot be taken — an unavailable model,
    /// or the entitlement throw the tool-schema test already documents. The
    /// router treats nil as "unknown" and fails closed rather than reading it
    /// as unlimited room.
    @available(macOS 26.0, *)
    public static func measureOnDeviceDefaults() async -> Int? {
        #if canImport(FoundationModels)
        if #available(macOS 26.4, *) {
            guard case .available = SystemLanguageModel.default.availability else {
                return nil
            }
            do {
                let cost = try await measure(
                    instructions: Instructions(
                        "You are the memory assistant for a private knowledge kernel."
                    ),
                    tools: [MemoryRecallTool()]
                )
                guard let window = cost.remaining, window > responseReserve else {
                    return nil
                }
                return window - responseReserve
            } catch {
                return nil
            }
        }
        #endif
        return nil
    }
}