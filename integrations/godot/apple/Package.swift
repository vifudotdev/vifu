// swift-tools-version: 5.9

import Foundation
import PackageDescription

let environment = ProcessInfo.processInfo.environment

let vifuDependency: Package.Dependency = if let path = environment["VIFU_GODOT_VIFU_PATH"] {
    .package(name: "Vifu", path: path)
} else {
    .package(url: "https://github.com/vifudotdev/vifu.git", exact: "0.1.10")
}

let swiftGodotKitDependency: Package.Dependency = if let path = environment["VIFU_GODOT_SWIFTGODOTKIT_PATH"] {
    .package(name: "SwiftGodotKit", path: path)
} else {
    .package(
        url: "https://github.com/vifudotdev/SwiftGodotKit.git",
        exact: "4.5.1-vifu.1"
    )
}

let swiftGodotDependency: Package.Dependency = if let path = environment["VIFU_GODOT_SWIFTGODOT_PATH"] {
    .package(name: "SwiftGodot", path: path)
} else {
    .package(
        url: "https://github.com/vifudotdev/SwiftGodot.git",
        exact: "4.5.1-vifu.1"
    )
}

let libgodotReleaseTag = "libgodot-4.5.1-vifu.1"
let libgodotReleaseBaseURL = "https://github.com/vifudotdev/vifu/releases/download/\(libgodotReleaseTag)"

let iosLibgodotTarget: Target = if let path = environment["VIFU_GODOT_IOS_LIBGODOT_PATH"] {
    .binaryTarget(name: "VifuLibgodotIOS", path: path)
} else {
    .binaryTarget(
        name: "VifuLibgodotIOS",
        url: "\(libgodotReleaseBaseURL)/ios_libgodot.xcframework.zip",
        checksum: "52eb883a52d93f5b0605e4a0b816f1e13397678562963afc1ac21db6be5186e6"
    )
}

let macLibgodotTarget: Target = if let path = environment["VIFU_GODOT_MACOS_LIBGODOT_PATH"] {
    .binaryTarget(name: "VifuLibgodotMacOS", path: path)
} else {
    .binaryTarget(
        name: "VifuLibgodotMacOS",
        url: "\(libgodotReleaseBaseURL)/mac_libgodot.xcframework.zip",
        checksum: "7717f474a2ce5bde1e922d211e6d071d8b05c0ce57899da25dd8dfc56606c8cd"
    )
}

let package = Package(
    name: "VifuGodot",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        .library(name: "VifuGodot", targets: ["VifuGodot"]),
    ],
    dependencies: [
        vifuDependency,
        swiftGodotKitDependency,
        swiftGodotDependency,
    ],
    targets: [
        .target(
            name: "VifuGodot",
            dependencies: [
                .product(name: "Vifu", package: "Vifu"),
                .product(name: "VifuRuntimeBridge", package: "Vifu"),
                .product(name: "SwiftGodotKit", package: "SwiftGodotKit"),
                .product(name: "SwiftGodot", package: "SwiftGodot"),
                .target(name: "VifuLibgodotIOS", condition: .when(platforms: [.iOS])),
                .target(name: "VifuLibgodotMacOS", condition: .when(platforms: [.macOS])),
            ]
        ),
        .testTarget(
            name: "VifuGodotTests",
            dependencies: [
                "VifuGodot",
                .product(name: "VifuRuntimeBridge", package: "Vifu"),
            ]
        ),
        iosLibgodotTarget,
        macLibgodotTarget,
    ]
)
