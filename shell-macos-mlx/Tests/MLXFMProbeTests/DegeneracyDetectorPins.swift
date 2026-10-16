import Foundation
import Testing

@testable import MLXFMProbe

/// PINS THE DETECTOR, using the outputs this repository has actually observed.
///
/// A coherence gate nobody has pinned is a gate nobody can tune. Every threshold
/// in `Degeneracy` has a fixture here that either demands it or forbids it, and
/// the BROKEN fixtures are verbatim captures from real runs on this machine —
/// not invented strings.
///
/// The asymmetric priority: a detector that misses garbage is far worse than one
/// that over-flags, because over-flagging is visible (a red test) and missing is
/// invisible (a confident tok/s number). Where the two conflict, flag.
@Suite("Degeneracy detector")
struct DegeneracyDetectorPins {

    // MARK: - Must FLAG. Verbatim from real broken runs.

    /// LFM2.5-24B-A2B DWQ 3-bit, measured 56 tok/s on an M1 Pro, 2026-10-15.
    @Test("flags the DWQ repetition loop verbatim")
    func flagsDWQRepetition() {
        let out = "But. However. But. But. But. However. "
            + "But. But. But. But. However."
        let f = Degeneracy.inspect(out)
        #expect(f.isDegenerate, "a measured 56 tok/s run emitting this must NOT read as coherent")
        #expect(f.verdict == .repetition, "got \(f.verdict): \(f.summary)")
    }

    /// Same run, a different prompt: the loop rotates through several words, so
    /// no single word crosses a ceiling. Only a WINDOW catches this, which is why
    /// the window arm exists and why it has its own fixture.
    @Test("flags the DWQ cyclical loop verbatim")
    func flagsDWQCyclical() {
        let out = """
            The chain is the proof. The proof is the chain. The chain is the proof. \
            The proof is the chain. The chain is the proof. The proof is the chain.
            """
        let f = Degeneracy.inspect(out)
        #expect(f.isDegenerate, "got \(f.verdict): \(f.summary)")
        // Assert DEGENERATE, not `.cyclical`. This text has "the" twelve times in
        // thirty words, so the cheaper repetition arm fires FIRST and the window
        // arm never runs — which is the cheaper arm doing its job. The window arm
        // has its own fixture below, built so no single word can reach a ceiling.
        #expect(f.verdict == .repetition || f.verdict == .cyclical,
                "got \(f.verdict): \(f.summary)")
    }

    /// A model that loads and answers with the PROMPT echoed back — the third
    /// observed shape, and the one the two above would both miss.
    @Test("flags prompt echo")
    func flagsPromptEcho() {
        let out = "Answer yes or no and justify briefly. "
            + "Answer yes or no and justify. "
            + "Answer yes or no and justify. "
            + "Answer yes or no and justify briefly."
        let f = Degeneracy.inspect(out)
        #expect(f.isDegenerate, "prompt echo must not read as coherent")
        // Named explicitly: this is the fixture that forced the lowDiversity arm.
        #expect(f.verdict == .lowDiversity,
                "the prompt-echo fixture is the one the ratio arm exists for; got \(f.verdict): \(f.summary)")
    }

    @Test("flags empty and near-empty output")
    func flagsEmpty() {
        #expect(Degeneracy.inspect("").verdict == .empty)
        #expect(Degeneracy.inspect("   \n\t ").verdict == .empty)
        #expect(Degeneracy.inspect("But.").verdict == .tooShort)
        #expect(Degeneracy.inspect("But. However.").verdict == .tooShort)
    }

    // MARK: - Must PASS. Real outputs from the working Qwen3.5-9B run.

    /// This is the one that mis-calibrated the first version of the check.
    /// It is 6 words and 30 characters, and it is the CORRECT answer.
    @Test("does NOT flag a correct short answer")
    func passesCorrectShortAnswer() {
        let out = "The capital of France is Paris."
        let f = Degeneracy.inspect(out)
        #expect(!f.isDegenerate, "a correct one-sentence answer must pass; got \(f.summary)")
        #expect(f.wordCount == 6)
    }

    @Test("does NOT flag a correct long answer that repeats 'the'")
    func passesCorrectLongAnswer() {
        let out = """
            The chain is what makes a stored memory verifiable, because it records the \
            hash of the content at the moment it was written and the hash of the chain \
            that came before it. That is what lets someone reconstruct the history of a \
            fact instead of taking the current row on trust.
            """
        let f = Degeneracy.inspect(out)
        #expect(!f.isDegenerate, "got \(f.summary)")
    }

    @Test("does NOT flag a legitimately repeated word")
    func passesLegitimateRepeat() {
        // "the" appears 5 times in 20 words: over the absolute ceiling of 3, but
        // under the share ceiling of 20/3=6. This is the case the share rule
        // exists for, and it must not be caught.
        let out = "the model stores the memory and the chain proves the memory was "
            + "never altered so the reader can trust the the provenance"
        let f = Degeneracy.inspect(out)
        #expect(!f.isDegenerate, "a common word in prose is not a loop; got \(f.summary)")
    }

    /// Thinking-mode output opens with a deliberation preamble. It is long, it
    /// repeats, and it is legitimate — a detector that flags it would make the
    /// reasoning tier unusable, which is the opposite of what it is for.
    @Test("does NOT flag a <think> preamble")
    func passesThinkingPreamble() {
        let out = """
            <think>
            The user asks for the capital of France. That is a factual lookup. I recall \
            that Paris has been the capital since the tenth century, though the modern \
            administrative region is the Île-de-France. The answer is short and \
            unambiguous, so I should answer directly and not over-explain.
            </think>
            The capital of France is Paris.
            """
        let f = Degeneracy.inspect(out)
        #expect(!f.isDegenerate, "a thinking preamble is not degeneracy; got \(f.summary)")
    }

    // MARK: - The property the windows arm actually buys.

    @Test("the window arm catches rotation the word arm cannot")
    func windowArmBeatsWordArm() {
        // Every word here appears twice. No word crosses any repetition ceiling.
        // Only the repeated WINDOW identifies this.
        let out = "alpha beta gamma delta alpha beta gamma delta alpha beta gamma delta"
        let f = Degeneracy.inspect(out)
        #expect(f.verdict == .cyclical, "got \(f.verdict): \(f.summary)")
    }

    @Test("finding reports its working")
    func findingIsExplainable() {
        let f = Degeneracy.inspect("But. However. But. But. But. However. But. But.")
        #expect(f.summary.contains("repetition"), "\(f.summary)")
        #expect(f.topWord == "but")
        #expect(f.topWordCount >= 4)
        #expect(f.evidence.contains("but"))
    }
}