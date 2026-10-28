import Foundation
import Testing

import AppleFMProbe

#if canImport(FoundationModels)
import FoundationModels
#endif

// Protocol pins for the `brain-afm` bridge executable. These drive the REAL
// binary over its JSONL stdio protocol — not an in-process mock — because the
// protocol IS the product surface the kernel depends on, and the failure
// class that matters (a wire shape drift between Swift and Rust) is only
// visible at the process boundary.
//
// Model-touching arms are guarded like the rest of this suite: on a machine
// where the model is unavailable they assert the NAMED refusal instead of
// skipping, because "not measured here" is an answer, not a pass.

@Suite("brain-afm bridge protocol")
struct BrainAfmBridgeTests {
    /// Locate the built executable relative to THIS file, so `swift test`
    /// always drives the binary the same build produced.
    private static func bridgeURL() throws -> URL {
        let here = URL(fileURLWithPath: #filePath, isDirectory: false)
        // …/Tests/AppleFMProbeTests/BridgeProtocolTests.swift → package root
        // (file → AppleFMProbeTests → Tests → the package directory)
        let root = here.deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let candidates = [
                root.appendingPathComponent(
                    ".build/out/Products/Debug/brain-afm"
                ),
                root.appendingPathComponent(
                    ".build/arm64-apple-darwin/debug/brain-afm"
                ),
            ]
        for candidate in candidates where FileManager.default.isExecutableFile(
            atPath: candidate.path
        ) {
            return candidate
        }
        throw CocoaError(.fileNoSuchFile)
    }

    /// Run the bridge over a fixed request script; collect the parsed
    /// JSONL replies. EOF after the script's last line ends the process.
    private static func drive(_ requests: [[String: Any]]) throws -> [[String: Any]] {
        let url = try bridgeURL()
        let process = Process()
        process.executableURL = url
        let stdin = Pipe()
        let stdout = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        try process.run()

        var script = Data()
        for request in requests {
            script.append(
                (try JSONSerialization.data(withJSONObject: request))
                    + Data([0x0A])
            )
        }
        try stdin.fileHandleForWriting.write(contentsOf: script)
        stdin.fileHandleForWriting.closeFile()

        let data = stdout.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()

        var replies: [[String: Any]] = []
        for line in String(decoding: data, as: UTF8.self).split(separator: "\n") {
            if let object = try? JSONSerialization.jsonObject(with: Data(line.utf8))
                as? [String: Any] {
                replies.append(object)
            }
        }
        return replies
    }

    @Test("capabilities: the reply carries exactly the contract keys, measured")
    func capabilitiesShape() async throws {
        let replies = try Self.drive([
            ["id": "c1", "op": "capabilities"]
        ])
        #expect(replies.count == 1, "capabilities answers exactly one line")
        let reply = replies[0]
        #expect(reply["id"] as? String == "c1")
        #expect(reply["ok"] as? Bool == true)
        // The contract keys, exactly: a new key is additive, a MISSING key
        // breaks the kernel's parser — so the set is pinned, not spot-checked.
        #expect(
            Set(reply.keys) == [
                "id", "ok", "window", "budget", "languages",
                "pccWindow", "pccEligible", "pccGeneration",
                "escalationPermitted",
            ],
            "capabilities keys drifted: \(reply.keys.sorted())"
        )
        // escalationPermitted must AGREE with the measured generation probe:
        // permitting escalation while the probe refused is the M1 trap
        // (routing on `isAvailable` produces a path that fails on first use).
        let permitted = reply["escalationPermitted"] as? Bool
        let generation = reply["pccGeneration"] as? String
        #expect(permitted == (generation == "ok"))
    }

    @Test("a turn before capabilities is refused by name, not guessed")
    func turnBeforeCapabilities() throws {
        let replies = try Self.drive([
            ["id": "t0", "op": "turn", "session": "sx", "prompt": "hello"]
        ])
        #expect(replies.count == 1)
        #expect(replies[0]["event"] as? String == "error")
        #expect(replies[0]["code"] as? String == "bad_request")
    }

    @Test("changed instructions on a live session are refused, never merged")
    func instructionsAreLocked() async throws {
        // Two turns on one session, the second carrying DIFFERENT
        // instructions. Model availability shapes what turn 1 may do, but
        // the lock is decided BEFORE any generation, so both shapes assert
        // the same law: the second instruction never takes effect.
        let replies = try Self.drive([
            ["id": "c1", "op": "capabilities"],
            [
                "id": "t1", "op": "turn", "session": "s9", "prompt": "hi",
                "instructions": "first",
            ],
            [
                "id": "t2", "op": "turn", "session": "s9", "prompt": "again",
                "instructions": "second",
            ],
        ])
        let terminal = replies.last
        #expect(terminal?["event"] as? String == "error")
        #expect(terminal?["code"] as? String == "instructions_locked")
    }

    @Test("a malformed line is one error event, and the loop survives it")
    func malformedLineSurvives() throws {
        let url = try Self.bridgeURL()
        let process = Process()
        process.executableURL = url
        let stdin = Pipe()
        let stdout = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        try process.run()
        try stdin.fileHandleForWriting.write(
            contentsOf: Data("this is not json\n".utf8)
        )
        // then a well-formed no-op op the bridge also names:
        try stdin.fileHandleForWriting.write(
            contentsOf: Data("{\"id\":\"z\",\"op\":\"nope\"}\n".utf8)
        )
        stdin.fileHandleForWriting.closeFile()
        let data = stdout.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        let lines = String(decoding: data, as: UTF8.self)
            .split(separator: "\n")
        #expect(lines.count == 2, "one error per bad line, then the unknown op")
        let first = try JSONSerialization.jsonObject(with: Data(lines[0].utf8))
            as! [String: Any]
        #expect(first["event"] as? String == "error")
        #expect(first["code"] as? String == "bad_request")
        let second = try JSONSerialization.jsonObject(with: Data(lines[1].utf8))
            as! [String: Any]
        #expect(second["code"] as? String == "bad_request")
    }

    // ── LL-04 §1.1 — the typed contract ──────────────────────────────────

    /// THE SCHEMA DRIFT PIN: the GroundedAnswer round-trips through
    /// GeneratedContent and the JSON it produces carries EXACTLY the wire
    /// vocabulary the kernel's Rust mirror parses (`abstain`, `answer`,
    /// `claims`, and per claim `text` + `cites`). A renamed property, a new
    /// field, or a re-spelled key drifts the bridge protocol — this pin is
    /// where the drift surfaces, model-free, in every lane.
    #if canImport(FoundationModels)
    @available(macOS 26.0, *)
    @Test("the grounded answer schema round-trips with the contract vocabulary")
    func groundedAnswerSchemaRoundTrips() throws {
        let answer = GroundedAnswer(
            abstain: false,
            answer: "Green tea is steamed Camellia sinensis.",
            claims: [
                GroundedClaim(
                    text: "Green tea is steamed Camellia sinensis.",
                    cites: [0, 2]
                )
            ]
        )
        let content = answer.generatedContent
        let decoded = try GroundedAnswer(content)
        #expect(
            decoded.generatedContent == content,
            "the typed answer survives the GeneratedContent round-trip unchanged"
        )
        // The wire vocabulary, pinned key by key: the kernel's mirror and
        // the support verdict read exactly these names.
        guard case .structure(let properties, _) = content.kind else {
            Issue.record("the generated content is not the declared structure")
            return
        }
        #expect(
            Set(properties.keys) == ["abstain", "answer", "claims"],
            "schema keys drifted: \(properties.keys.sorted())"
        )
        let json = content.jsonString
        #expect(json.contains("\"abstain\""), "the abstain key is on the wire")
        #expect(json.contains("\"claims\""), "the claims key is on the wire")
        #expect(json.contains("\"text\""), "a claim's text key is on the wire")
        #expect(json.contains("\"cites\""), "a claim's cites key is on the wire")
    }
    #endif

    /// THE TYPED FINAL, ON THE REAL WIRE: a driven turn's final event
    /// carries the contract keys EXACTLY — `abstain` and `claims` present
    /// even when the model abstains or cites nothing, because "every key is
    /// always present" is the reader's contract. Model-backed like the rest
    /// of this suite: where the model is unavailable the turn answers the
    /// NAMED refusal, and that refusal is what this pin asserts instead.
    @Test("the real bridge's typed final carries the contract keys")
    func typedFinalCarriesTheContractKeys() async throws {
        let replies = try Self.drive([
            ["id": "c1", "op": "capabilities"],
            [
                "id": "t1", "op": "turn", "session": "ll04-typed",
                "prompt": "Green tea is a tea made from steamed Camellia sinensis leaves. Answer with one grounded claim.",
                "instructions":
                    "You are a memory-grounded assistant. Answer from the given memory and cite the hit index each claim draws from; say plainly when the memory does not answer.",
            ],
        ])
        let final = replies.last { $0["event"] as? String == "final" }
        if final == nil {
            // The model lane is unavailable on this machine: the turn must
            // have refused BY NAME — never a halfway answer.
            let error = replies.last { $0["event"] as? String == "error" }
            #expect(error?["code"] as? String == "model_unavailable")
            return
        }
        #expect(final?["abstain"] as? Bool != nil, "abstain is present, always")
        let claims = final?["claims"] as? [[String: Any]]
        #expect(claims != nil, "claims is present, always — empty is a value")
        for claim in claims ?? [] {
            #expect(claim["text"] is String, "a claim carries text")
            #expect(claim["cites"] is [Int], "a claim carries hit indices")
        }
    }

    /// THE COLD CONTROL (LL-04 §4 pins), MEASURED AND REPORTED, NOT GATED —
    /// AFM is a model, so a flaky sample must never red the lane; the
    /// measurement is what the evidence file records. Ten turns whose
    /// prompt carries ZERO evidence (the degraded-recall posture's exact
    /// shape): an honest model abstains. The count is printed for the run
    /// log and reported in `LL-04_EVIDENCE_2026-10-28.md`.
    @Test("the cold control: a zero-evidence prompt yields abstention (measured, reported)")
    func coldControlZeroEvidenceAbstains() async throws {
        let replies = try Self.drive([["id": "c1", "op": "capabilities"]])
        if replies.first?["window"] as? Int == 0 {
            // No model on this machine: the measurement cannot be made, and
            // the honest answer is the named posture, not a fake sample.
            print("cold-control: NOT MEASURED — the on-device model is unavailable on this machine")
            return
        }
        var requests: [[String: Any]] = [["id": "c0", "op": "capabilities"]]
        for n in 0..<10 {
            requests.append([
                "id": "cc\(n)", "op": "turn", "session": "ll04-cold",
                // Zero evidence: the prompt asserts nothing about the world
                // and grounds nothing — there is no hit to cite.
                "prompt": "What is the capital of Ruritania?",
                "instructions":
                    "You are a memory-grounded assistant. The recalled memory for this turn is EMPTY: no evidence was found. Answer from the given memory only, citing hit indices; if the memory does not answer, set abstain to true and leave claims empty. Say plainly when it is not.",
            ])
        }
        let turns = try Self.drive(requests)
        var abstained = 0
        var answered = 0
        for reply in turns where reply["event"] as? String == "final" {
            answered += 1
            if reply["abstain"] as? Bool == true { abstained += 1 }
        }
        print(
            "cold-control: \(abstained)/\(answered) zero-evidence turns abstained " +
                "(the plan's band: ≥ 9/10; reported, not gated — AFM is a model)"
        )
        // The HARNESS assertion only: every turn produced a typed final. The
        // abstention RATE is the reported measurement, deliberately not an
        // assertion — a model is not a deterministic component.
        #expect(answered == 10, "all ten cold-control turns answered a typed final")
    }
}
