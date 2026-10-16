import Foundation

/// Detects output that is PRESENT but not MEANING.
///
/// The failure this exists for is specific and it has bitten this repository
/// twice. A rotated-basis or mis-quantised model does not crash, does not return
/// nil, and does not fail to load. It streams at full speed and emits text that
/// is fluent-shaped and contentless:
///
///     "But. However. But. But. But. …"                    (LFM2.5 DWQ 3-bit, 56 tok/s)
///     "The chain is the proof. The proof is the chain. …"   (same run, later prompt)
///
/// Every speed figure measured against those runs was void. So the detector runs
/// BEFORE any performance claim is trusted — not as a style preference, but
/// because a number produced by a broken model is worse than no number, because
/// it looks like evidence.
///
/// WHAT IT IS NOT. This cannot tell a CORRECT answer from a WRONG one. It tells
/// a model from a loop. Anything stronger is a task-specific oracle — which is
/// what `Probe` is for.
public enum Degeneracy {

    /// What a detector concluded about one piece of output.
    public enum Verdict: String, Sendable, Equatable {
        /// Text arrived and is shaped like an answer.
        case coherent
        /// Nothing arrived at all.
        case empty
        /// Too few words to be an answer — "Paris." is one, which is fine for one
        /// word of a prompt but is also what noise looks like.
        case tooShort
        /// A small vocabulary repeated — the DWQ signature.
        case repetition
        /// The same phrase, slightly varied, over and over — the rotated-basis
        /// signature. Distinct from `repetition`: every word can be new while the
        /// whole is a loop.
        case cyclical
        /// Recycling rather than composing: few distinct words across many.
        /// The prompt-echo signature, which no single-word or fixed-window arm
        /// sees.
        case lowDiversity
    }

    public struct Finding: Sendable, Equatable {
        public let verdict: Verdict
        public let wordCount: Int
        public let distinctWords: Int
        public let topWord: String
        public let topWordCount: Int
        /// Set when the detector wants to show its working rather than just rule.
        public let evidence: String

        public var isDegenerate: Bool { verdict != .coherent }

        public var summary: String {
            switch verdict {
            case .coherent:
                return "coherent (\(wordCount) words, \(distinctWords) distinct)"
            case .empty:      return "EMPTY — a completed stream with no text"
            case .tooShort:   return "too short (\(wordCount) words)"
            case .repetition:
                return "repetition — \"\(topWord)\" x\(topWordCount) of \(wordCount)"
            case .cyclical:
                return "cyclical — \(evidence)"
            case .lowDiversity:
                return "low diversity — \(distinctWords) distinct over \(wordCount) words (\(evidence))"
            }
        }
    }

    /// Thresholds, as named constants rather than inline literals.
    ///
    /// CALIBRATED AGAINST KNOWN-BROKEN AND KNOWN-GOOD OUTPUT, not chosen for
    /// plausibility. The first version of this check demanded 40 characters and
    /// 8 words and failed the correct answer to "name the capital in one short
    /// sentence" — which is 30 characters and 6 words. The numbers below are the
    /// ones that separate the cases we have actually observed.
    public enum Limits {
        /// Below this a model is producing noise, not an answer. "Paris." is 1.
        public static let minimumWords = 4

        /// A word may appear this many times before it counts as a repetition
        /// loop. "The capital of France is Paris." has "the" once.
        public static let absoluteRepetitionCeiling = 3

        /// …or this share of the output, whichever is larger. A short correct
        /// answer may legitimately repeat "the"; a long one may not.
        public static let repetitionShareDenominator = 3

        /// A repeating window at this size, found this many times, is `cyclical`.
        /// The rotated-basis garbage rotates through several words, so it never
        /// trips a single-word ceiling — it trips this.
        public static let cyclicalWindow = 4
        public static let cyclicalRepeats = 3

        /// Type-token ratio — distinct words over total — below which the output
        /// is recycling rather than composing.
        ///
        /// Added because a PINNED FIXTURE FOUND THE GAP. The observed prompt echo
        /// is "Answer yes or no and justify briefly. Answer yes or no and justify. …"
        /// — the phrase repeats, but with `briefly` inserted in two of the four
        /// places, so the window arm's fixed-width consecutive match never fires,
        /// and no single word crosses a repetition ceiling (answer 4 of 26). It
        /// passed as COHERENT. Distinct/total is 7/26 = 0.27.
        ///
        /// Calibrated against every fixture in this repository:
        ///   broken   prompt echo 0.27 · DWQ repetition 0.17 · DWQ cyclical 0.13
        ///   passing  capital 1.00 · long answer ~0.73 · prose w/ "the" ~0.75
        ///                        <think> preamble ~0.6
        /// A 0.40 floor sits in the middle of a wide gap, not near an edge.
        public static let minimumTypeTokenRatio = 0.40

        /// Below this length the ratio is too noisy to judge. "The capital of
        /// France is Paris." is 6 words over 6 distinct and says nothing.
        public static let ratioMinimumWords = 12
    }

    /// Classify one piece of generated text.
    public static func inspect(_ text: String) -> Finding {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let words = trimmed.lowercased()
            .split(whereSeparator: { !$0.isLetter })
            .map(String.init)

        guard !trimmed.isEmpty else {
            return Finding(verdict: .empty, wordCount: 0, distinctWords: 0,
                           topWord: "", topWordCount: 0, evidence: "")
        }
        guard words.count >= Limits.minimumWords else {
            return Finding(verdict: .tooShort, wordCount: words.count,
                           distinctWords: Set(words).count, topWord: words.first ?? "",
                           topWordCount: 1, evidence: trimmed.prefix(120).description)
        }

        let counts = Dictionary(grouping: words, by: { $0 }).mapValues(\.count)
        let (topWord, topWordCount) = counts.max { $0.value < $1.value }!
        let ceiling = max(Limits.absoluteRepetitionCeiling,
                          words.count / Limits.repetitionShareDenominator)
        if topWordCount > ceiling {
            return Finding(verdict: .repetition, wordCount: words.count,
                           distinctWords: counts.count, topWord: topWord,
                           topWordCount: topWordCount,
                           evidence: "\"\(topWord)\" x\(topWordCount)")
        }

        if let cycle = cyclicWindow(words) {
            return Finding(verdict: .cyclical, wordCount: words.count,
                           distinctWords: counts.count, topWord: topWord,
                           topWordCount: topWordCount, evidence: cycle)
        }

        // Last, because it is the broadest arm and the one most likely to be
        // wrong on a short or unusual answer. A repeating-window loop is better
        // described by the window than by a ratio.
        if words.count >= Limits.ratioMinimumWords {
            let ratio = Double(counts.count) / Double(words.count)
            if ratio < Limits.minimumTypeTokenRatio {
                return Finding(verdict: .lowDiversity, wordCount: words.count,
                               distinctWords: counts.count, topWord: topWord,
                               topWordCount: topWordCount,
                               evidence: String(format: "ratio %.2f", ratio))
            }
        }

        return Finding(verdict: .coherent, wordCount: words.count,
                       distinctWords: counts.count, topWord: topWord,
                       topWordCount: topWordCount, evidence: "")
    }

    /// Find a window of `width` words repeated `repeats` times consecutively.
    ///
    /// This is the arm that catches rotated-basis garbage. Its vocabulary keeps
    /// changing — "the chain is the proof", "the proof is the chain" — so no single
    /// word ever crosses the repetition ceiling, and only a window does.
    private static func cyclicWindow(_ words: [String]) -> String? {
        let width = Limits.cyclicalWindow
        let needed = Limits.cyclicalRepeats
        guard words.count >= width * needed else { return nil }
        var i = 0
        while i + width * needed <= words.count {
            let head = Array(words[i..<(i + width)])
            var matched = true
            for k in 1..<needed {
                let offset = i + k * width
                if Array(words[offset..<(offset + width)]) != head { matched = false; break }
            }
            if matched { return "\"\(head.joined(separator: " "))\" x\(needed)" }
            i += 1
        }
        return nil
    }
}