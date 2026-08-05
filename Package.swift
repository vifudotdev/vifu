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
    func hasSlice(platform: String, variant: String? = nil, architectures: Set<String>) -> Bool {
        libraries.contains { library in
            guard library["SupportedPlatform"] as? String == platform else { return false }
            let actualVariant = library["SupportedPlatformVariant"] as? String
            guard actualVariant == variant else { return false }
            let actualArchitectures = Set(library["SupportedArchitectures"] as? [String] ?? [])
            return actualArchitectures.isSuperset(of: architectures)
        }
    }
    return hasSlice(platform: "ios", architectures: ["arm64"])
        && hasSlice(platform: "ios", variant: "simulator", architectures: ["arm64", "x86_64"])
        && hasSlice(platform: "macos", architectures: ["arm64", "x86_64"])
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
        url: "https://github.com/vifudotdev/vifu/releases/download/v0.1.8/VifuMobileFFI.xcframework.zip",
        checksum: "622b423fc96a3459e781f329cdce70759ccd0a7e204ab6023d4a751f92895426"
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
