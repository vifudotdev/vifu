// swift-tools-version: 5.9

import Foundation
import PackageDescription

let localArtifact = ProcessInfo.processInfo.environment["VIFU_SWIFT_LOCAL_ARTIFACT"]
let ffiTarget: Target

if let localArtifact {
    ffiTarget = .binaryTarget(
        name: "VifuMobileFFI",
        path: localArtifact
    )
} else {
    ffiTarget = .binaryTarget(
        name: "VifuMobileFFI",
        url: "https://github.com/vifudotdev/vifu/releases/download/v0.1.3/VifuMobileFFI.xcframework.zip",
        checksum: "b5eadb0cba9fbb874980be477396cd6e059d841ae781de18f8c9eef95a837f59"
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
            path: "apple/Sources/Vifu"
        ),
        .testTarget(
            name: "VifuTests",
            dependencies: ["Vifu"],
            path: "apple/Tests/VifuTests"
        ),
        ffiTarget,
    ]
)
