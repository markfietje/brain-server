// The typed answer (LL-04 §1.1 — cite-or-abstain as a SCHEMA, not a regex).
//
// This file lives in the LIBRARY target, not the `brain-afm` executable,
// because the schema is shared vocabulary: the bridge generates against it,
// and the protocol pins (`BridgeProtocolTests`) round-trip it. A test
// target cannot import an executable; the library is the surface both
// already speak.
//
// The kernel owns the VERDICT on what the model asserts: `cites` are
// 0-based indices into the hits the kernel SENT, and the support decision
// is pure index arithmetic over them (no model in the gate). This file's
// only law is that the model must ANSWER IN THIS SHAPE — abstain first,
// then prose, then per-claim evidence — so "say plainly when it is not"
// holds as a wire fact, not an instruction the prose may ignore.

import Foundation
#if canImport(FoundationModels)
import FoundationModels

/// One sentence-level claim the model asserts, with the grounding hits it
/// draws from. The bridge carries what the model asserted, verbatim — it
/// never judges the indices.
@available(macOS 26.0, *)
@Generable
public struct GroundedClaim {
    /// The sentence, verbatim as the model asserts it.
    public var text: String
    /// Indices into the turn's grounding hits.
    public var cites: [Int]
    /// The memberwise init stays available to the bridge and the pins; the
    /// macro adds the GeneratedContent conversion in an extension.
    public init(text: String, cites: [Int]) {
        self.text = text
        self.cites = cites
    }
}

/// The typed final (LL-04 §1.1): the model answers OR abstains, and every
/// claim names its evidence. `abstain` is declared FIRST so guided
/// generation commits to the abstention decision before any prose — the
/// instructions' "say plainly when it is not" becomes a schema, not a hope.
@available(macOS 26.0, *)
@Generable
public struct GroundedAnswer {
    /// true = "I don't know from the memory I was given"; claims stay empty.
    public var abstain: Bool
    /// The prose the user reads; may carry [n] markers.
    public var answer: String
    /// Each sentence-level claim the model asserts.
    public var claims: [GroundedClaim]
    public init(abstain: Bool, answer: String, claims: [GroundedClaim]) {
        self.abstain = abstain
        self.answer = answer
        self.claims = claims
    }
}
#endif
