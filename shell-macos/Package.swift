// swift-tools-version: 6.0
import PackageDescription

// The Swift half of the shell's Apple-Intelligence bridge.
//
// This package is a PROBE, not the product. It exists so CI can answer one
// question that no Linux lane can: does the FoundationModels surface we
// intend to build against still compile, and still bind, against the macOS
// 27 SDK? Its entire value is that it is small enough to review in one
// sitting and touches no product code.
//
// The bridge proper moves here when the shell ships on-device generation.
// Until then this is the whole Swift surface, deliberately.
//
// Nothing here is compiled by `shell.yml`, which stays on ubuntu-latest and
// is byte-identical to before this package existed. The macOS half is gated
// by `.github/workflows/shell-macos.yml` on the `xcode-27` runner.
let package = Package(
    name: "BrainShellAppleFM",
    platforms: [.macOS(.v15)],
    products: [
        .library(name: "AppleFMProbe", targets: ["AppleFMProbe"]),
        .executable(name: "brain-afm", targets: ["brain-afm"])
    ],
    targets: [
        .target(
            name: "AppleFMProbe",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        // The bridge executable (the chat spine, Round 1): JSONL over stdio,
        // one request per line in, one event per line out. It holds the
        // stateful `LanguageModelSession`s (the framework has no seed-history
        // API, so conversation continuity lives HERE, behind the kernel's
        // audit copy), reuses ChatModelLayer's router/trimmer/planner as-is,
        // and reaches no network except Apple's own framework (PCC).
        .executableTarget(
            name: "brain-afm",
            dependencies: ["AppleFMProbe"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(
            name: "AppleFMProbeTests",
            dependencies: ["AppleFMProbe"],
            // `apple-verified-quotes.txt` is read from the SOURCE tree at
            // `#filePath` rather than bundled: it is corpus data the pin
            // compares against, and a reviewer reading the test needs to read
            // it next to the test. Declaring it a resource would copy it into
            // the bundle and make the reviewed copy and the compared copy
            // different artifacts.
            exclude: ["apple-verified-quotes.txt"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
    ]
)