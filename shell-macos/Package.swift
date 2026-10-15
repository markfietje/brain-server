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
        .library(name: "AppleFMProbe", targets: ["AppleFMProbe"])
    ],
    targets: [
        .target(
            name: "AppleFMProbe",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(
            name: "AppleFMProbeTests",
            dependencies: ["AppleFMProbe"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
    ]
)