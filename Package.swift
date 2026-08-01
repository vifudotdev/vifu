// swift-tools-version: 5.9

import Foundation
import PackageDescription

let generatedLocalArtifact = "Frameworks/VifuMobileFFI.xcframework"
let configuredLocalArtifact = ProcessInfo.processInfo.environment["VIFU_SWIFT_LOCAL_ARTIFACT"]
let packageRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let generatedLocalArtifactURL = packageRoot.appendingPathComponent(generatedLocalArtifact)
let localArtifact = configuredLocalArtifact
    ?? (FileManager.default.fileExists(atPath: generatedLocalArtifactURL.path)
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
    ],
    targets: [
        .target(
            name: "Vifu",
            dependencies: ["VifuMobileFFI"],
            path: "apple/Sources/Vifu",
            linkerSettings: [
                .linkedLibrary("c++"),
                .linkedFramework("Accelerate"),
                .linkedFramework("Metal"),
                .linkedFramework("MetalKit"),
                .linkedFramework("Security"),
            ]
        ),
        .testTarget(
            name: "VifuTests",
            dependencies: ["Vifu"],
            path: "apple/Tests/VifuTests"
        ),
        ffiTarget,
    ]
)
