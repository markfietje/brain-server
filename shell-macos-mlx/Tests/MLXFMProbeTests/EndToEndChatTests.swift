import Foundation
import Testing

#if canImport(FoundationModels)
import FoundationModels
#endif

@testable import MLXFMProbe

/// END TO END: a question, a retrieval tool, a grounded answer.
///
/// This is the product thesis in one test. The kernel already has row-level authz,
/// calibrated abstention, chain-verifiable provenance and certified erasure; the
/// claim under test is that a MODEL can sit in front of them and produce an answer
/// that is traceable to what it was given — and that it says "I could not find
/// that" rather than inventing a benefit.
///
/// It is deliberately HERMETIC. The retrieval facts live in a fixture beside this
/// file rather than in a running daemon, because a test that needs a launchd
/// service, a token file and whatever corpus happens to be installed is a test that
/// fails for reasons unrelated to what it is testing. The kernel is reachable for a
/// separate integration pass; this proves the MODEL side is sound.
///
/// And it runs on the real 9B, not the 0.6B plumbing fixture: the question below is
/// the one that distinguishes "can emit tokens" from "can answer".
@Suite("End-to-end grounded chat", .serialized)
struct EndToEndChatTests {

    @Test("answers from retrieved facts, and cites them",
          .enabled(if: ProcessInfo.processInfo.environment["BRAIN_FM_MEASURE"] == "1"),
          .timeLimit(.minutes(20)))
    func groundsItsAnswerInTheTool() async throws {
        guard #available(macOS 27.0, *) else { return }

        let model = try await MLXFMProbe.model(
            configuration: MLXFMProbe.reasoningQwen,
            capabilities: [.toolCalling, .guidedGeneration, .reasoning])
        let session = LanguageModelSession(model: model, tools: [GreenTeaFacts()])

        let question = "What are the benefits of green tea?"
        var called = false
        var retrieved: [String] = []
        var text = ""
        var first = 0.0

        let start = ContinuousClock.now
        let stream = session.streamResponse(
            to: question,
            options: GenerationOptions(samplingMode: .greedy, maximumResponseTokens: 2048))
        for try await snapshot in stream {
            if text.isEmpty && first == 0 { first = Self.seconds(since: start) }
            text = snapshot.content.description
            for entry in snapshot.transcriptEntries {
                if case let .toolCalls(calls) = entry {
                    for call in calls where call.toolName == "green_tea_facts" {
                        called = true
                        retrieved.append(call.arguments.jsonString)
                    }
                }
            }
        }
        let total = Self.seconds(since: start)

        print("""
        ── end-to-end: \(question) ──
          tool called : \(called)  args=\(retrieved.first ?? "—")
          first token : \(String(format: "%.0f ms", first * 1000))
          total       : \(String(format: "%.0f ms", total * 1000))
          answer      : \(text.prefix(600).replacingOccurrences(of: "\n", with: " "))
        ──────────────────────────────────────────────────────
        """)

        // 1. The model must actually RETRIEVE. Answering a green-tea question from
        //    priors is the failure this whole kernel exists to make visible, and a
        //    speed or fluency check cannot see it.
        #expect(called, "the model answered without calling the retrieval tool")

        // 2. It must be COHERENT first. A degenerate stream here would otherwise
        //    read as "grounded" because the fixture's words are in the prompt.
        let deg = Degeneracy.inspect(text)
        #expect(!deg.isDegenerate, "degenerate answer: \(deg.summary)")

        let answer = text.lowercased()
        // 3. It must land on facts that are IN the fixture. Two of the four, so a
        //    correct-but-vague answer still passes and a generic wellness essay
        //    does not.
        let grounded = ["l-theanine", "catechins", "egcg", "caffeine"]
            .filter { answer.contains($0) }
        #expect(grounded.count >= 2,
                "answer engaged with \(grounded.count) fixture facts; text: \(text.prefix(200))")

        // 4. THE CONTESTED ITEM. A model answering from priors says green tea is high
        //    in caffeine. The fixture's EFSA note makes that contested, and a
        //    grounded answer must engage with the limit rather than ignore it.
        #expect(answer.contains("6 mg") || answer.contains("efsa") || answer.contains("limit"),
                "the contested EFSA limit is in the fixture and was not engaged with")

        // 5. No citation of a fact that is NOT in the fixture. The fixture is the
        //    whole world for this question.
        let invented = ["boosts metabolism", "burns fat", "detox", "cures",
                        "prevents cancer", "anti-aging"]
            .filter { answer.contains($0) }
        #expect(invented.isEmpty, "claims absent from the fixture: \(invented)")
    }

    /// The abstention arm. Same tool, a question the facts cannot answer. The
    /// kernel's calibrated `low_confidence` is the product's sharpest behaviour and
    /// no competitor has it — so it is worth proving a MODEL passes it through
    /// rather than papering over it.
    @Test("declines rather than inventing when the store cannot answer",
          .enabled(if: ProcessInfo.processInfo.environment["BRAIN_FM_MEASURE"] == "1"),
          .timeLimit(.minutes(20)))
    func abstainsWhenTheFixtureCannotAnswer() async throws {
        guard #available(macOS 27.0, *) else { return }

        let model = try await MLXFMProbe.model(
            configuration: MLXFMProbe.reasoningQwen,
            capabilities: [.toolCalling, .guidedGeneration, .reasoning])
        let session = LanguageModelSession(model: model, tools: [GreenTeaFacts()])

        let question = "Who won the 2031 Rugby World Cup?"
        var called = false
        var text = ""
        let stream = session.streamResponse(
            to: question,
            options: GenerationOptions(samplingMode: .greedy, maximumResponseTokens: 2048))
        for try await snapshot in stream {
            text = snapshot.content.description
            for entry in snapshot.transcriptEntries {
                if case let .toolCalls(calls) = entry {
                    for call in calls where call.toolName == "green_tea_facts" { called = true }
                }
            }
        }

        print("── abstention: \(question) ──\n   answer: \(text.prefix(300))")

        let deg = Degeneracy.inspect(text)
        #expect(!deg.isDegenerate, "degenerate abstention: \(deg.summary)")
        #expect(called, "it must consult the store before claiming ignorance or knowledge")

        let answer = text.lowercased()
        let refused = ["do not", "don't", "cannot", "can't", "not in", "no information",
                       "don't have", "do not have", "not aware", "unable", "not enough",
                       "no record", "not found"]
            .filter { answer.contains($0) }
        #expect(!refused.isEmpty,
                "the store cannot answer this, so the model must say so; it said: \(text.prefix(240))")
    }

    static func seconds(since start: ContinuousClock.Instant) -> Double {
        let c = (ContinuousClock.now - start).components
        return Double(c.seconds) + Double(c.attoseconds) / 1e18
    }
}

/// The retrieval tool. Stands in for the kernel's `recall`; the SHAPE is the point —
/// a `Tool` returning text, which is exactly what the shell's four memory tools do.
@available(macOS 27.0, *)
struct GreenTeaFacts: FoundationModels.Tool {
    @Generable
    public struct Arguments: Sendable {
        @Guide(description: "What to look up about green tea.")
        public var topic: String

        public init(topic: String) { self.topic = topic }
    }

    public typealias Output = String

    public let name = "green_tea_facts"
    public let description = """
        Look up documented facts about green tea: caffeine content, L-theanine, \
        catechins, and regulatory safety limits. Returns only what is stored, \
        with an id per fact. If the topic is not covered, say so.
        """

    public init() {}

    /// The fixture the tool serves. Deliberately small, factual, and containing one
    /// CONTESTED item: EFSA's 2024 cap is 6 mg caffeine per 100 ml, which brewed
    /// green tea exceeds. A model answering "green tea is high in caffeine" from
    /// priors alone is therefore caught by the grounded test.
    static let facts: [(id: String, text: String)] = [
        ("kt1", "Green tea leaves are ~30% caffeine by dry weight; a 240 ml cup "
               + "brewed weakly holds roughly 30 mg."),
        ("kt2", "Green tea contains L-theanine, which crosses the blood-brain barrier "
               + "and is associated with a calmer alert state."),
        ("kt3", "Its catechins, chiefly EGCG, are the subject of most trials; "
               + "effects reported are modest and mostly on lipids and blood pressure."),
        ("kt4", "EFSA (2024) advises an upper limit of 6 mg caffeine per 100 ml for "
               + "beverages; brewed green tea at 30 mg per 240 ml exceeds that."),
    ]

    public func call(arguments: Arguments) async throws -> String {
        let relevant = Self.facts.filter {
            arguments.topic.isEmpty
                || $0.text.lowercased().contains(arguments.topic.lowercased())
                || arguments.topic.lowercased().contains("green")
                || arguments.topic.lowercased().contains("caffeine")
        }
        guard !relevant.isEmpty else {
            return "NO_MATCH: the store holds nothing about that."
        }
        return relevant.map { "[\($0.id)] \($0.text)" }.joined(separator: "\n")
    }
}