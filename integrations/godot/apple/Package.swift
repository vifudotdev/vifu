// swift-tools-version: 5.9

import Foundation
import PackageDescription

let environment = ProcessInfo.processInfo.environment

let vifuDependency: Package.Dependency = if let path = environment["VIFU_GODOT_VIFU_PATH"] {
    .package(name: "Vifu", path: path)
} else {
    .package(url: "https://github.com/vifudotdev/vifu.git", branch: "main")
}

let swiftGodotKitDependency: Package.Dependency = if let path = environment["VIFU_GODOT_SWIFTGODOTKIT_PATH"] {
    .package(name: "SwiftGodotKit", path: path)
} else {
    .package(
        url: "https://github.com/vifudotdev/SwiftGodotKit.git",
        revision: "f72ec6f03e22f0209819716a84c54d5b56064cf0"
    )
}

let swiftGodotDependency: Package.Dependency = if let path = environment["VIFU_GODOT_SWIFTGODOT_PATH"] {
    .package(name: "SwiftGodot", path: path)
} else {
    .package(
        url: "https://github.com/vifudotdev/SwiftGodot.git",
        revision: "6644df67f538a5de8f750762f632b4dda56c982e"
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
                .product(name: "VifuRuntimeBridge", package: "Vifu"),
                .product(name: "SwiftGodotKit", package: "SwiftGodotKit"),
                .product(name: "SwiftGodot", package: "SwiftGodot"),
            ]
        ),
        .testTarget(
            name: "VifuGodotTests",
            dependencies: [
                "VifuGodot",
                .product(name: "VifuRuntimeBridge", package: "Vifu"),
            ]
        ),
    ]
)
