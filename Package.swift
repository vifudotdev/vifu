// swift-tools-version: 5.9

import Foundation
import PackageDescription

let generatedLocalArtifact = "Frameworks/VifuMobileFFI.xcframework"
let configuredLocalArtifact = ProcessInfo.processInfo.environment["VIFU_SWIFT_LOCAL_ARTIFACT"]
let packageRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let generatedLocalArtifactURL = packageRoot.appendingPathComponent(generatedLocalArtifact)

func supportsAllApplePlatforms(_ artifactURL: URL) -> Bool {
    let infoURL = artifactURL.appendingPathComponent("Info.plist")
    guard
        let data = try? Data(contentsOf: infoURL),
        let plist = try? PropertyListSerialization.propertyList(from: data, format: nil),
        let info = plist as? [String: Any],
        let libraries = info["AvailableLibraries"] as? [[String: Any]]
    else {
        return false
    }
    let platforms = Set(libraries.compactMap { $0["SupportedPlatform"] as? String })
    return platforms.isSuperset(of: ["ios", "macos"])
}

let localArtifact = configuredLocalArtifact
    ?? (FileManager.default.fileExists(atPath: generatedLocalArtifactURL.path)
        && supportsAllApplePlatforms(generatedLocalArtifactURL)
        ? generatedLocalArtifact
        : nil)
let ffiTarget: Target

if let localArtifact {
    ffiTarget = .binaryTarget(
        name: "VifuMobileFFI",
        path: localArtifact
    )
} else {
    ffiTarget = .binaryTarget(
        name: "VifuMobileFFI",
        url: "https://github.com/vifudotdev/vifu/releases/download/v0.1.5/VifuMobileFFI.xcframework.zip",
        checksum: "980cd902b66a304890ff95a6ed4e58aa51c824fca2aa56d32e2c3dd90623edbc"
    )
}

let package = Package(
    name: "Vifu",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        .library(
            name: "Vifu",
            targets: ["Vifu"]
        ),
        .library(
            name: "VifuRuntimeBridge",
            targets: ["VifuRuntimeBridge"]
        ),
    ],
    targets: [
        .target(
            name: "Vifu",
            dependencies: ["VifuMobileFFI", "VifuRuntimeBridge"],
            path: "apple/Sources/Vifu",
            linkerSettings: [
                .linkedLibrary("c++"),
                .linkedFramework("Accelerate"),
                .linkedFramework("Metal"),
                .linkedFramework("MetalKit"),
                .linkedFramework("Security"),
            ]
        ),
        .target(
            name: "VifuRuntimeBridge",
            path: "apple/Sources/VifuRuntimeBridge"
        ),
        .testTarget(
            name: "VifuTests",
            dependencies: ["Vifu", "VifuRuntimeBridge"],
            path: "apple/Tests/VifuTests"
        ),
        ffiTarget,
    ]
)
