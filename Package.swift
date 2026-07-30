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
        checksum: "412332bf52ce88c28e1755f18c3e23cd1aeeab3bc23187c1addb1badd97c81d2"
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
