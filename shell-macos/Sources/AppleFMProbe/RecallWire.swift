import Foundation

// MARK: - What the Rust core decided

/// The kernel's `decision`, verbatim from the published enum
/// (`openapi.yaml` `/recall` 200: `enum: [ok, low_confidence]`).
///
/// The spelling is the wire's. Plan 04 §2 writes `"answered" |
/// "low_confidence"`, and a client-side third spelling would be a contract lie
/// dressed as a convenience — so `ok` stays `ok`.
public enum RecallDecision: String, Sendable, Equatable {
    case ok
    case lowConfidence = "low_confidence"
}

/// One passage as the core projects it.
///
/// `validFrom`/`validTo` are lifted out of the hit's evidence on the wire and
/// are first-class here. Presenting a fact whose `validTo` has passed as
/// current is the failure this kernel exists to prevent, so validity travels
/// with the passage instead of being something a renderer has to go and dig
/// out.
public struct RecallPassage: Sendable, Equatable {
    public let id: Int
    public let title: String?
    public let text: String
    public let snippet: String?
    public let domain: String?
    public let score: Double
    public let validFrom: String?
    public let validTo: String?
    public let flagged: Bool
    public let ingestKind: String?
    public let memoryKind: String?
    public let authority: Double?

    public init(
        id: Int,
        title: String? = nil,
        text: String,
        snippet: String? = nil,
        domain: String? = nil,
        score: Double = 0,
        validFrom: String? = nil,
        validTo: String? = nil,
        flagged: Bool = false,
        ingestKind: String? = nil,
        memoryKind: String? = nil,
        authority: Double? = nil
    ) {
        self.id = id
        self.title = title
        self.text = text
        self.snippet = snippet
        self.domain = domain
        self.score = score
        self.validFrom = validFrom
        self.validTo = validTo
        self.flagged = flagged
        self.ingestKind = ingestKind
        self.memoryKind = memoryKind
        self.authority = authority
    }
}

/// What came back, as the CORE classified it.
///
/// Six states, mirroring `shell/src-tauri/src/recall.rs`. The count is not
/// decoration: Plan 04 §5 names five while the wire admits a sixth, and the
/// one that matters most is the refusal. "You may not read this" and "there is
/// nothing here" are different facts about the reader, and a UI that draws
/// both as an empty list has leaked the existence of rows the reader was
/// refused.
public enum RecallOutcome: Sendable, Equatable {
    /// `decision: ok` with at least one passage.
    case answered(passages: [RecallPassage], decision: RecallDecision, confidence: Float?)
    /// `decision: ok` with none. An ABSENCE, named as one.
    case absent(decision: RecallDecision, confidence: Float?)
    /// `decision: low_confidence`. An answer that says "I could not find
    /// that" — emphatically NOT an error.
    case abstained(decision: RecallDecision, confidence: Float?)
    /// 401/403. Must never render like `absent`.
    case refused(status: Int, scope: String)
    /// Any other non-2xx, plus an unreadable 2xx.
    case unreachable(status: Int, detail: String)
    /// A 2xx whose body did not match the published contract.
    case contractViolation(status: Int, detail: String)

    /// Did we get passages? Deliberately NOT "was it refused?" — an abstention
    /// also has none, and a caller that cannot tell those two apart has a bug.
    public var hasPassages: Bool {
        if case let .answered(passages, _, _) = self { return !passages.isEmpty }
        return false
    }

    /// True only for the state that must never be drawn as "no results".
    public var isRefusal: Bool {
        if case .refused = self { return true }
        return false
    }
}

// MARK: - The seam

/// The one way this package reaches recall — and note that it does NOT reach
/// the kernel itself.
///
/// The transport is SUPPLIED by the host, which invokes the Rust core. That is
/// the whole of the constraint the stub used to carry in a comment: the real
/// implementation "belongs behind the Rust core's bounds and authz — not here,
/// where nothing enforces them". A protocol seam is how that comment becomes
/// the architecture. Swift holds a typed outcome the core already bounded and
/// already classified; it never opens a socket, never learns a URL, and never
/// holds a token.
///
/// A Swift bypass would be a second, unaudited path to private data — and it
/// is the failure mode that looks like a refactor, because the shorter version
/// is "just fetch it here".
public protocol KernelRecallTransport: Sendable {
    /// - Throws: whatever the host's bridge throws. There is deliberately no
    ///   error type here, because a Swift-side taxonomy of failure would be a
    ///   second classification of the same exchange — and the core already
    ///   made that decision. A thrown error is a transport that did not
    ///   complete, which `MemoryRecallTool` renders as degraded rather than as
    ///   an absence.
    func recall(query: String, domain: String?) async throws -> RecallOutcome
}

// MARK: - The one inert transport, and why it is not a stub

/// Why a transport could not complete.
public enum RecallTransportError: Error, Sendable, Equatable {
    /// No host bridge was supplied.
    case notWired
}

/// A transport for the tool-SCHEMA MEASUREMENT only.
///
/// `AppleFMCost.measureOnDeviceDefaults` prices the tool's `@Generable` schema
/// against the model window; it never invokes the tool, so this transport is
/// never reached on that path.
///
/// It exists rather than a `nil` or an optional because it must be impossible
/// to confuse with a working bridge — and, above all, because it is NOT the
/// stub string. The stub this replaces returned a plausible-looking sentence
/// from a tool call that reached nothing. If this one is ever reached it
/// THROWS, and `MemoryRecallTool.call` turns that into a rendered, named
/// "recall unavailable" — degraded and stated, rather than a fabricated
/// answer. That is the whole difference between the two, and it is the reason
/// the stub string appears nowhere in this package.
public struct UnwiredRecallTransport: KernelRecallTransport {
    public init() {}

    public func recall(query: String, domain: String?) async throws -> RecallOutcome {
        throw RecallTransportError.notWired
    }
}

// MARK: - Rendering


/// Renders an outcome as the citation block a transcript can show.
///
/// Two rules, both from Plan 04 §4: tool output is NOT prose and NOT raw JSON.
/// It is a typed event rendered as citations, because mixing tool JSON into the
/// token stream is how provenance gets lost. And the four non-answered states
/// each get their own wording, because "no results" is the rendering that
/// collapses an abstention, an absence and a refusal into one indistinguishable
/// blank.
public enum RecallRenderer {
    public static func render(_ outcome: RecallOutcome) -> String {
        switch outcome {
        case let .answered(passages, decision, confidence):
            var lines = ["## Recalled memory (\(decision.rawValue)\(confidenceSuffix(confidence)))"]
            for (index, passage) in passages.enumerated() {
                lines.append("")
                lines.append("[\(index + 1)] id \(passage.id)\(domainSuffix(passage.domain))")
                if let title = passage.title, !title.isEmpty { lines.append("    \(title)") }
                lines.append("    \(passage.text)")
                lines.append("    \(validitySuffix(passage))")
                if passage.flagged { lines.append("    quarantined at ingest") }
            }
            return lines.joined(separator: "\n")

        case let .absent(decision, confidence):
            return
                "## No memory matched (\(decision.rawValue)\(confidenceSuffix(confidence))). The search ran and the store holds nothing for it."

        case let .abstained(decision, confidence):
            return
                "## No confident answer (\(decision.rawValue)\(confidenceSuffix(confidence))). The store has no answer for this; pass that through rather than filling the gap."

        case let .refused(status, scope):
            // Named as a refusal, with the status, and deliberately NOT phrased
            // as an absence. It does not say how many rows exist, or whether
            // any do — that is the row the reader was refused.
            return
                "## Access refused — HTTP \(status) on \(scope). You do not have access to this memory. This is not an empty result."

        case let .unreachable(status, detail):
            return
                "## Recall unavailable — HTTP \(status). \(detail) Retrieval is degraded, not empty."

        case let .contractViolation(status, detail):
            return
                "## Recall response unreadable — HTTP \(status). \(detail) This is a contract fault, not an empty result."
        }
    }

    private static func confidenceSuffix(_ confidence: Float?) -> String {
        guard let confidence else { return "" }
        return String(format: ", confidence %.2f", confidence)
    }

    private static func domainSuffix(_ domain: String?) -> String {
        guard let domain, !domain.isEmpty else { return "" }
        return " · domain \(domain)"
    }

    private static func validitySuffix(_ passage: RecallPassage) -> String {
        switch (passage.validFrom, passage.validTo) {
        case let (from?, to?): return "valid \(from) → \(to)"
        case let (from?, nil): return "valid from \(from)"
        case let (nil, to?): return "EXPIRED at \(to) — not current"
        case (nil, nil): return "validity not stated"
        }
    }
}
