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
        url: "https://github.com/vifudotdev/vifu/releases/download/v0.1.4/VifuMobileFFI.xcframework.zip",
        checksum: "5d5211eaf0dd1ee24d6041e3af78850acde88ea9de4892cbc661247be3094708"
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
