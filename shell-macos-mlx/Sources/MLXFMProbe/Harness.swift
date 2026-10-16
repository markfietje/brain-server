import Foundation

#if canImport(FoundationModels)
import FoundationModels
#endif

import MLXFoundationModels
import MLXLLM
import MLXLMCommon

/// A candidate model, described well enough to be run and reported on.
///
/// `@available(macOS 27.0)` because `LanguageModelCapabilities.Capability` is a
/// 27 surface — a stored property of it cannot live in an unguarded type.
@available(macOS 27.0, *)
public struct Candidate: Sendable {
    public let name: String
    public let configuration: ModelConfiguration
    public let capabilities: [LanguageModelCapabilities.Capability]
    public let notes: String

    public init(
        name: String,
        configuration: ModelConfiguration,
        capabilities: [LanguageModelCapabilities.Capability] = [.toolCalling, .guidedGeneration],
        notes: String = ""
    ) {
        self.name = name
        self.configuration = configuration
        self.capabilities = capabilities
        self.notes = notes
    }

    /// The small CI plumbing candidate.
    public static var ci: Candidate {
        Candidate(name: "qwen3-0.6b-4bit", configuration: LLMRegistry.qwen3_0_6b_4bit)
    }

    /// The measured Plan 03 candidate.
    public static var qwen35_9b: Candidate {
        Candidate(
            name: "qwen3.5-9b-4bit",
            configuration: MLXFMProbe.reasoningQwen,
            capabilities: [.toolCalling, .guidedGeneration, .reasoning],
            notes: "thinking mode on by default; tokenizer from Qwen/Qwen3.5-9B")
    }
}

/// One probe's result.
///
/// `.unreachable` is NOT a pass and NOT a failure. It is its own verdict, because
/// collapsing it into either is how a real constraint disappears: the Qwen3.5
/// multi-turn probe throws `.runtime("No user query found in messages.")`, and
/// reporting that as "the loop works" would be a lie, while reporting it as a
/// failure would hide that single-turn generation is fine.
public struct ProbeResult: Sendable {
    public enum Verdict: String, Sendable {
        /// Met the expectation.
        case pass
        /// Ran, and did not.
        case fail
        /// Could not run — a transport or template limitation, not the model.
        case unreachable
        /// The stream COMPLETED with zero snapshots and no error.
        ///
        /// NOT degeneracy and NOT a model failure. Nothing was generated because
        /// the runtime never ran — an empty answer and a stalled runtime look
        /// identical from outside, and reporting the first as the second would
        /// condemn a model for the harness's own environment. This is how the 9B
        /// reported itself before the distinction existed.
        case stalled
    }

    public let probe: String
    public let verdict: Verdict
    public let detail: String
    public let degeneracy: Degeneracy.Verdict
    public let firstTokenSeconds: Double?
    public let decodeTokensPerSecond: Double?

    public var isDegenerate: Bool { degeneracy != .coherent }
}

/// Runs a fixed probe suite against any candidate and returns a report.
///
/// Every probe is DIAGNOSTIC: a throw is caught, recorded as `.unreachable`, and
/// the suite continues. A coherence harness that aborts on the first exception
/// loses every measurement after the failure — which is how a 56 tok/s number
/// reached a log with no text beside it.
@available(macOS 27.0, *)
public struct Harness {

    public struct Report: Sendable {
        public let candidate: String
        public let results: [ProbeResult]

        public var degenerate: Bool { results.contains { $0.isDegenerate } }
        public var failed: [ProbeResult] { results.filter { $0.verdict == .fail } }
        public var stalled: [ProbeResult] { results.filter { $0.verdict == .stalled } }
        public var unreachable: [ProbeResult] { results.filter { $0.verdict == .unreachable } }

        /// The headline: is this model trustworthy at all?
        /// Usable requires a CLEAN run: not degenerate, not failed, not stalled.
        /// A stall is not a pass — it is an absent measurement.
        public var usable: Bool { !degenerate && failed.isEmpty && stalled.isEmpty }

        public func render() -> String {
            var out = "\n── coherence report: \(candidate) ──\n"
            for r in results {
                let mark: String = switch r.verdict {
                    case .pass: "✓" ; case .fail: "✗" ; case .stalled: "!?" ; case .unreachable: "–"
                }
                out += String(format: "  %@ %@", mark, r.probe.padding(toLength: 26, withPad: " ", startingAt: 0))
                out += "  \(r.verdict.rawValue)"
                if let f = r.firstTokenSeconds {
                    out += String(format: " · first %.0f ms", f * 1000)
                }
                if let d = r.decodeTokensPerSecond, d > 0 {
                    out += String(format: " · %.1f tok/s", d)
                }
                if r.isDegenerate { out += " · DEGENERATE(\(r.degeneracy.rawValue))" }
                out += "\n      \(r.detail)\n"
            }
            out += "\n  verdict: "
            if degenerate { out += "UNUSABLE — degenerate output" }
            else if !stalled.isEmpty { out += "INCONCLUSIVE — the runtime stalled; not a verdict on the model" }
            else if !failed.isEmpty { out += "COHERENT but failing \(failed.count) probe(s)" }
            else { out += "coherent" }
            if !failed.isEmpty { out += " · \(failed.count) failed" }
            if !stalled.isEmpty { out += " · \(stalled.count) stalled" }
            if !unreachable.isEmpty { out += " · \(unreachable.count) unreachable" }
            return out + "\n"
        }
    }

    public static func run(_ candidate: Candidate) async -> Report {
        var results: [ProbeResult] = []
        let session: LanguageModelSession
        // ONE model for the whole run. Sessions are cheap; a 6 GB MLX model is
        // not. The first version built a model per session and per probe, and on
        // this 16 GB machine TWO of them exhaust unified memory — the stream then
        // yields ZERO snapshots and throws nothing, so a resource failure
        // presented as an empty answer. That is Plan 03's "swap, do not
        // co-reside" law, observed by tripping over it.
        let model: MLXLanguageModel
        do {
            model = try await MLXFMProbe.model(configuration: candidate.configuration,
                                              capabilities: candidate.capabilities)
            session = LanguageModelSession(model: model)
        } catch {
            return Report(candidate: candidate.name, results: [
                ProbeResult(probe: "session", verdict: .unreachable,
                            detail: "could not construct a session: \(error)",
                            degeneracy: .empty, firstTokenSeconds: nil,
                            decodeTokensPerSecond: nil)
            ])
        }

        // P1 — single turn, factual. The baseline every model must clear.
        results.append(await probe(session, "single-turn/fact",
            "Name the capital of France in one sentence.") { out in
                let ok = out.lowercased().contains("paris")
                return (ok, ok ? "named Paris" : "did not name Paris: \(out.prefix(90))")
            })

        // P2 — single turn, reasoning. Expects a specific token, so a fluent
        // non-answer fails rather than passing on shape alone.
        results.append(await probe(session, "single-turn/reason",
            "Is 17 a prime number? Answer yes or no, then one sentence of why.") { out in
                let low = out.lowercased()
                let saidYes = low.contains("yes")
                let justified = low.contains("divisor") || low.contains("factor")
                    || low.contains("only") || low.contains("not divisible")
                return (saidYes && justified,
                        "yes=\(saidYes) justified=\(justified)")
            })

        // P3 — MULTI-TURN LOOP. Known to throw on Qwen3.5.
        results.append(await probe(
            session, "loop/multi-turn",
            "In one sentence: what is a provenance chain?",
            seed: { _ in (false, "seed only") },
            followUp: "Now in one sentence: why does it matter?",
            then: { out in (out.count > 20, "second turn produced \(out.count) chars") }))

        // P4 — TOOL CALLING. The capability the shell actually depends on: every
        // memory tool in the plan set is a `Tool`.
        results.append(await toolCallProbe(candidate, model: model))

        // P5 — GUIDED / STRUCTURED OUTPUT. `@Generable` is what keeps a 3B model
        // from emitting prose where the kernel expects a typed value.
        results.append(await guidedProbe(session))

        return Report(candidate: candidate.name, results: results)
    }

    // MARK: - internals

    private struct Turn {
        var text: String
        var first: Double
        var rate: Double
        var stalled: Bool = false
    }

    private static func oneTurn(
        _ session: LanguageModelSession, _ prompt: String, maxTokens: Int = 256
    ) async throws -> Turn {
        let start = ContinuousClock.now
        var first = 0.0
        var text = ""
        var n = 0
        let stream = session.streamResponse(
            to: prompt,
            options: GenerationOptions(samplingMode: .greedy, maximumResponseTokens: maxTokens))
        for try await snapshot in stream {
            if n == 0 { first = Self.seconds(since: start) }
            n += 1
            text = snapshot.content.description
        }
        let total = Self.seconds(since: start)
        if n == 0 {
            // Distinguish before anything downstream can mistake this for an
            // answer. `Degeneracy.inspect("")` would say `.empty`, and that is
            // true of the TEXT and false of the CAUSE.
            return Turn(text: "", first: 0, rate: 0, stalled: true)
        }
        let d = total - first
        // A decode RATE needs a window worth dividing. The first version reported
        // 143143 tok/s for a short second turn, because total-minus-first was
        // near zero and the chunk count small — a number that looks like a
        // measurement and is arithmetic on noise.
        let rate: Double
        if n >= 5 && d > 0.15 {
            rate = Double(n) / d
        } else {
            rate = 0
        }
        return Turn(text: text, first: first, rate: rate)
    }

    private static func probe(
        _ session: LanguageModelSession, _ name: String, _ prompt: String,
        judge: @escaping (String) -> (Bool, String)
    ) async -> ProbeResult {
        do {
            let t = try await oneTurn(session, prompt)
            let deg = Degeneracy.inspect(t.text)
            let (ok, why) = judge(t.text)
            return ProbeResult(
                probe: name,
                verdict: deg.isDegenerate ? .fail : (ok ? .pass : .fail),
                detail: deg.isDegenerate ? deg.summary : why,
                degeneracy: deg.verdict,
                firstTokenSeconds: t.first, decodeTokensPerSecond: t.rate)
        } catch {
            return ProbeResult(probe: name, verdict: .unreachable,
                detail: "\(error)", degeneracy: .empty,
                firstTokenSeconds: nil, decodeTokensPerSecond: nil)
        }
    }

    private static func probe(
        _ session: LanguageModelSession, _ name: String, _ first_: String,
        seed: @escaping (String) -> (Bool, String),
        followUp: String, then judge: @escaping (String) -> (Bool, String)
    ) async -> ProbeResult {
        do {
            let seedTurn = try await oneTurn(session, first_)
            if seedTurn.stalled {
                return ProbeResult(probe: name, verdict: .stalled,
                    detail: "seed turn stalled", degeneracy: .empty,
                    firstTokenSeconds: nil, decodeTokensPerSecond: nil)
            }
            let t = try await oneTurn(session, followUp)
            if t.stalled {
                return ProbeResult(probe: name, verdict: .stalled,
                    detail: "second turn produced ZERO snapshots and no error",
                    degeneracy: .empty, firstTokenSeconds: nil,
                    decodeTokensPerSecond: nil)
            }
            let deg = Degeneracy.inspect(t.text)
            let (ok, why) = judge(t.text)
            return ProbeResult(
                probe: name,
                verdict: deg.isDegenerate ? .fail : (ok ? .pass : .fail),
                detail: deg.isDegenerate ? deg.summary : why,
                degeneracy: deg.verdict,
                firstTokenSeconds: t.first, decodeTokensPerSecond: t.rate)
        } catch {
            return ProbeResult(probe: name, verdict: .unreachable,
                detail: "\(error)", degeneracy: .empty,
                firstTokenSeconds: nil, decodeTokensPerSecond: nil)
        }
    }

    private static func toolCallProbe(_ c: Candidate, model: MLXLanguageModel) async -> ProbeResult {
        do {
            // Reuses the run's single model; only the SESSION carries tools.
            let s = LanguageModelSession(model: model, tools: [EchoTool()])
            var sawEcho = false
            var args = ""
            let start = ContinuousClock.now

            // The stream element is a `ResponseStream.Snapshot`, NOT a `.Part` —
            // switching on `.toolCall` does not compile, and the call is reached
            // through `snapshot.transcriptEntries`.
            let stream = s.streamResponse(
                to: "Use the echo tool to repeat the word: pineapple",
                options: GenerationOptions(samplingMode: .greedy, maximumResponseTokens: 128))
            for try await snapshot in stream {
                for entry in snapshot.transcriptEntries {
                    // `Transcript.ToolCalls` is itself a RandomAccessCollection of
                    // `ToolCall` — there is no `.action` on THIS one. The
                    // `.action` belongs to the executor-channel ToolCalls, a
                    // different type with the same name.
                    if case let .toolCalls(calls) = entry {
                        // `toolName`, not `name`; arguments render via
                        // `jsonString`. Both taken from mlx-swift-lm's own
                        // TranscriptConverter, which is the working reference —
                        // the SDK exposes several same-named `ToolCall` types and
                        // guessing between them costs a build each time.
                        for call in calls where call.toolName == "echo" {
                            sawEcho = true
                            args = call.arguments.jsonString
                        }
                    }
                }
            }
            let first = Self.seconds(since: start)
            let hasArg = args.lowercased().contains("pineapple")
            let ok = sawEcho && hasArg
            return ProbeResult(
                probe: "tool-calling",
                verdict: ok ? .pass : .fail,
                detail: ok ? "called echo(\(args.prefix(60)))"
                           : "calledEcho=\(sawEcho) correctArg=\(hasArg) args=\(args.prefix(60))",
                degeneracy: .coherent, firstTokenSeconds: first,
                decodeTokensPerSecond: nil)
        } catch {
            return ProbeResult(probe: "tool-calling", verdict: .unreachable,
                detail: "\(error)", degeneracy: .empty,
                firstTokenSeconds: nil, decodeTokensPerSecond: nil)
        }
    }

    private static func guidedProbe(_ session: LanguageModelSession) async -> ProbeResult {
        do {
            let s = session.streamResponse(
                to: "Is the capital of France Paris?",
                generating: CapitalAnswer.self,
                includeSchemaInPrompt: true,
                options: GenerationOptions(samplingMode: .greedy, maximumResponseTokens: 64))
            for try await answer in s { _ = answer; break }
            return ProbeResult(probe: "guided/@Generable", verdict: .pass,
                detail: "schema-conformant value returned",
                degeneracy: .coherent, firstTokenSeconds: nil,
                decodeTokensPerSecond: nil)
        } catch {
            return ProbeResult(probe: "guided/@Generable", verdict: .unreachable,
                detail: "\(error)", degeneracy: .empty,
                firstTokenSeconds: nil, decodeTokensPerSecond: nil)
        }
    }

    static func seconds(since start: ContinuousClock.Instant) -> Double {
        let c = (ContinuousClock.now - start).components
        return Double(c.seconds) + Double(c.attoseconds) / 1e18
    }
}

@available(macOS 27.0, *)
@Generable
struct CapitalAnswer {
    @Guide(description: "The capital city.")
    var city: String
    @Guide(description: "Whether the statement is true.")
    var correct: Bool
}

/// The smallest tool that proves the shell's actual dependency.
///
/// The plan set's four memory tools are `Tool`s, and tool calling is the ONE
/// capability AFM 3 Core reports that a plain generator does not — so a candidate
/// that cannot call this cannot be the brain tier.
///
/// Shaped exactly like `MemoryRecallTool` in `shell-macos`, which is the one that
/// compiles: `@Generable` on a nested `Arguments`, `Output` as a typealias to
/// `String` (the framework models a return as anything `PromptRepresentable`;
/// `ToolOutput` is a transcript entry, not a return type), and a stored `name` and
/// `description` rather than `@Parameter`.
@available(macOS 27.0, *)
public struct EchoTool: FoundationModels.Tool {
    @Generable
    public struct Arguments: Sendable {
        @Guide(description: "The word to repeat back to the caller.")
        public var word: String

        public init(word: String) { self.word = word }
    }

    public typealias Output = String

    public let name = "echo"
    public let description = "Repeat a word back to the caller, unchanged."

    public init() {}

    public func call(arguments: Arguments) async throws -> String {
        "echo: \(arguments.word)"
    }
}
