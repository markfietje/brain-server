import Foundation
import MLXFMProbe

// The FM surface is macOS 27+, so the availability check is the USE SITE rather
// than the package floor — a package cannot raise its floor to 27 without
// becoming unusable everywhere below it. `shell-macos-mlx` already carries this
// pattern for the same reason.
if #available(macOS 27.0, *) {
    // One shot, then the paired verdict. No flags and no knobs: a measurement
    // whose parameters are adjusted after seeing the output is a
    // rationalization, not a criterion (plan 08's discipline, which is the
    // point of that plan).
    let results = await BannerExperiment.run(verbose: true)
    BannerExperiment.report(results)
} else {
    print("banner-experiment: requires macOS 27.0+ (the MLX/FoundationModels surface).")
    print("  host is \(ProcessInfo.processInfo.operatingSystemVersionString)")
    exit(2)
}
