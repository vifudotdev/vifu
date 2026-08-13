import Foundation

#if canImport(UIKit)
import UIKit
#endif

public struct VifuGatewayMetadata: Codable, Equatable, Sendable {
    public var name: String
    public var kind: String?
    public var platform: String?
    public var device: [String: String]
    public var application: [String: String]
    public var attributes: [String: String]

    public init(
        name: String,
        kind: String? = nil,
        platform: String? = nil,
        device: [String: String] = [:],
        application: [String: String] = [:],
        attributes: [String: String] = [:]
    ) {
        self.name = name
        self.kind = kind
        self.platform = platform
        self.device = device
        self.application = application
        self.attributes = attributes
    }
}

public extension VifuEmbeddedGatewayConfig {
    init(
        serverUrl: String,
        runtimeDatabasePath: String,
        serverCertificateDer: Data?
    ) {
        self.init(
            serverUrl: serverUrl,
            runtimeDatabasePath: runtimeDatabasePath,
            serverCertificateDer: serverCertificateDer,
            gatewayMetadataJson: "{}"
        )
    }

    init(
        serverUrl: String,
        runtimeDatabasePath: String,
        serverCertificateDer: Data?,
        gatewayMetadata: VifuGatewayMetadata
    ) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(gatewayMetadata)
        guard let json = String(data: data, encoding: .utf8) else {
            throw EncodingError.invalidValue(
                gatewayMetadata,
                EncodingError.Context(
                    codingPath: [],
                    debugDescription: "Gateway metadata is not valid UTF-8."
                )
            )
        }
        self.init(
            serverUrl: serverUrl,
            runtimeDatabasePath: runtimeDatabasePath,
            serverCertificateDer: serverCertificateDer,
            gatewayMetadataJson: json
        )
    }
}

#if canImport(UIKit)
public extension VifuGatewayMetadata {
    static func currentAppleMobile(
        attributes: [String: String] = [:]
    ) -> VifuGatewayMetadata {
        let bundle = Bundle.main
        let device = UIDevice.current
        let applicationName = bundle.object(
            forInfoDictionaryKey: "CFBundleDisplayName"
        ) as? String ?? bundle.object(
            forInfoDictionaryKey: "CFBundleName"
        ) as? String ?? "iOS application"

        return VifuGatewayMetadata(
            name: "\(applicationName) · \(device.model)",
            kind: "mobile",
            platform: "ios",
            device: [
                "architecture": currentAppleArchitecture,
                "localizedModel": device.localizedModel,
                "manufacturer": "Apple",
                "model": device.model,
                "osVersion": device.systemVersion,
                "systemName": device.systemName,
            ],
            application: [
                "build": bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "",
                "id": bundle.bundleIdentifier ?? "",
                "name": applicationName,
                "version": bundle.object(
                    forInfoDictionaryKey: "CFBundleShortVersionString"
                ) as? String ?? "",
            ],
            attributes: attributes
        )
    }
}

private var currentAppleArchitecture: String {
#if arch(arm64)
    "arm64"
#elseif arch(x86_64)
    "x86_64"
#else
    "unknown"
#endif
}
#endif
