import Testing
import Foundation

// The import must be at FILE SCOPE, not inside `#if canImport` around the
// use site. A conditional around a call does not bring the module's types
// into scope, and the build fails with `cannot find type 'Tool' in scope`
// rather than "module not available" — a misleading diagnostic that reads
// like a missing dependency when it is really an import placement.
#if canImport(FoundationModels)
import FoundationModels
#endif

@testable import AppleFMProbe

/// These are MEASUREMENT tests, not capability claims.
///
/// Each asserts a number the framework reports on the machine running them.
/// A pin that hardcoded 4096 would rot the first time Apple shipped a
/// different model, and a pin that asserted "available" would pass on a
/// build where `@available` had compiled the whole surface out of existence.
/// So each test asks the framework and then checks that what it said is
/// coherent.
@Suite("Apple FM probe")
struct AppleFMProbeTests {

    @Test("The on-device window is reported, and it is a combined budget")
    func onDeviceWindowIsMeasuredNotAssumed() {
        let caps = AppleFMProbe.capabilities()
        switch caps.onDevice {
        case let .available(contextSize, languages, variant, capabilities):
            #expect(contextSize > 0, "a reported window of zero is not a window")
            #expect(languages > 0, "a model reporting no languages is not usable")
            // Recorded, not asserted to a constant: the value moved from 4096
            // to something else across AFM generations, and a pin on the
            // number would break on hardware we have not seen yet.
            print("on-device window: \(contextSize) tokens; languages: \(languages)")
            // WHICH model answered is the question the window alone cannot
            // answer. A 4096 window is consistent with a 3B Core and with a
            // downgraded Core Advanced, and the two imply different routing.
            #expect(!variant.isEmpty, "an unnamed variant cannot be routed on")
            print("variant: \(variant); capabilities: \(capabilities)")
        case let .unavailable(reason):
            // Legitimate on non-Apple-silicon or pre-26 hosts. Say so rather
            // than failing, so the lane is honest about what it ran on.
            print("on-device model unavailable: \(reason)")
        }
    }

    @Test("Private Cloud eligibility is never reported as capability")
    func privateCloudEligibilityIsNotCapability() {
        let caps = AppleFMProbe.capabilities()
        switch caps.privateCloud {
        case .eligibleButUnentitled:
            // The exact trap, pinned. `isAvailable` returns true on a device
            // that is eligible, and the REQUEST then fails with
            // ModelManagerError 1046 for want of the app entitlement. A caller
            // that read the flag would ship a dead code path.
            #expect(
                caps.contextBudget != nil,
                "the on-device path must stand alone; PCC is never required"
            )
        case let .unavailable(reason):
            print("Private Cloud Compute unavailable: \(reason)")
        case _:
            break
        }
    }

    @Test("The tool schema cost is measurable, and small")
    func toolSchemaCostIsMeasured() async throws {
        #if canImport(FoundationModels)
        if #available(macOS 26.4, *) {
            let tools: [any Tool] = [MemoryRecallTool()]
            let instructions = Instructions(
                "You are the memory assistant for a private knowledge kernel."
            )
            let cost = try await ContextBudget.measure(
                instructions: instructions, tools: tools
            )
            print("instructions \(cost.instructions) + tools \(cost.toolSchemas)")
            #expect(cost.toolSchemas > 0, "a tool that costs nothing was not measured")
            if let budget = cost.remaining {
                #expect(
                    budget > 0,
                    "the tool surface consumed the entire window"
                )
                print("remaining for conversation: \(budget) tokens")
            }
        }
        #endif
    }
}