import Foundation
import Testing

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
}
