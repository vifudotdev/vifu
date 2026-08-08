import CryptoKit
import Foundation
import Security

/// Stable installation identity used to prove ownership to Vifu servers.
///
/// The private key remains in Keychain. Gateway IDs and Device Tokens are
/// server-specific authorizations and are stored separately.
public struct VifuGatewayMachineIdentity: Codable, Sendable {
    public let machineId: String
    public let publicKey: String
    public let privateKey: String

    public init(machineId: String, publicKey: String, privateKey: String) {
        self.machineId = machineId
        self.publicKey = publicKey
        self.privateKey = privateKey
    }

    public init(generated: VifuGeneratedGatewayIdentity) {
        self.init(
            machineId: generated.machineId,
            publicKey: generated.publicKey,
            privateKey: generated.privateKey
        )
    }
}

/// The paired server endpoint and its optional local TLS trust anchor.
///
/// This is intentionally separate from the installation identity and from the
/// server-issued authorization so an app can replace either independently.
public struct VifuGatewayServerBinding: Codable, Sendable {
    public let serverURL: String
    public let certificateDER: Data?
    public let certificateSHA256: String?

    public init(
        serverURL: String,
        certificateDER: Data? = nil,
        certificateSHA256: String? = nil
    ) {
        self.serverURL = serverURL
        self.certificateDER = certificateDER
        self.certificateSHA256 = certificateSHA256
    }
}

func vifuGatewayServerBindingHasValidTrust(_ binding: VifuGatewayServerBinding) -> Bool {
    switch (binding.certificateDER, binding.certificateSHA256) {
    case (nil, nil):
        return true
    case let (certificate?, fingerprint?):
        guard !certificate.isEmpty else { return false }
        let digest = SHA256.hash(data: certificate)
        let expected = "sha256:" + digest.map { String(format: "%02x", $0) }.joined()
        return fingerprint == expected
    default:
        return false
    }
}

private struct StoredGatewayAuthorization: Codable {
    let gatewayId: String
    let deviceToken: String
    let generation: UInt64
    let expiresAt: String

    init(_ authorization: VifuGatewayAuthorization) {
        gatewayId = authorization.gatewayId
        deviceToken = authorization.deviceToken
        generation = authorization.generation
        expiresAt = authorization.expiresAt
    }

    var value: VifuGatewayAuthorization {
        VifuGatewayAuthorization(
            gatewayId: gatewayId,
            deviceToken: deviceToken,
            generation: generation,
            expiresAt: expiresAt
        )
    }
}

public enum VifuGatewayIdentityStoreError: LocalizedError {
    case invalidIdentity
    case invalidServerURL
    case keychain(OSStatus)
    case invalidStoredValue

    public var errorDescription: String? {
        switch self {
        case .invalidIdentity:
            "The Agent Gateway machine identity is invalid."
        case .invalidServerURL:
            "The Vifu server URL is invalid."
        case let .keychain(status):
            "Keychain operation failed with status \(status)."
        case .invalidStoredValue:
            "The stored Agent Gateway value is invalid."
        }
    }
}

/// Stores one Machine identity per app installation and one authorization per server.
public struct VifuGatewayIdentityStore: Sendable {
    private static let machineAccount = "machine-identity"
    private static let serverBindingAccount = "server-binding"
    private let service: String

    public init(service: String? = nil) {
        self.service = service
            ?? Bundle.main.bundleIdentifier.map { "\($0).vifu.gateway" }
            ?? "dev.vifu.gateway"
    }

    public func loadOrCreateMachineIdentity() throws -> VifuGatewayMachineIdentity {
        if let stored: VifuGatewayMachineIdentity = try load(
            account: Self.machineAccount,
            as: VifuGatewayMachineIdentity.self
        ) {
            return stored
        }
        let generated = try generateVifuGatewayIdentity()
        let identity = VifuGatewayMachineIdentity(generated: generated)
        try save(identity, account: Self.machineAccount)
        return identity
    }

    public func saveServerBinding(_ binding: VifuGatewayServerBinding) throws {
        _ = try authorizationAccount(binding.serverURL)
        guard vifuGatewayServerBindingHasValidTrust(binding) else {
            throw VifuGatewayIdentityStoreError.invalidStoredValue
        }
        try save(binding, account: Self.serverBindingAccount)
    }

    public func loadServerBinding() throws -> VifuGatewayServerBinding? {
        let binding: VifuGatewayServerBinding? = try load(
            account: Self.serverBindingAccount,
            as: VifuGatewayServerBinding.self
        )
        guard let binding else { return nil }
        _ = try authorizationAccount(binding.serverURL)
        guard vifuGatewayServerBindingHasValidTrust(binding) else {
            throw VifuGatewayIdentityStoreError.invalidStoredValue
        }
        return binding
    }

    public func deleteServerBinding() throws {
        let status = SecItemDelete(
            keychainQuery(account: Self.serverBindingAccount) as CFDictionary
        )
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw VifuGatewayIdentityStoreError.keychain(status)
        }
    }

    public func saveAuthorization(
        _ authorization: VifuGatewayAuthorization,
        for serverURL: String
    ) throws {
        try save(StoredGatewayAuthorization(authorization), account: try authorizationAccount(serverURL))
    }

    public func loadAuthorization(for serverURL: String) throws -> VifuGatewayAuthorization? {
        let stored: StoredGatewayAuthorization? = try load(
            account: try authorizationAccount(serverURL),
            as: StoredGatewayAuthorization.self
        )
        return stored?.value
    }

    public func deleteAuthorization(for serverURL: String) throws {
        let status = SecItemDelete(
            keychainQuery(account: try authorizationAccount(serverURL)) as CFDictionary
        )
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw VifuGatewayIdentityStoreError.keychain(status)
        }
    }

    private func authorizationAccount(_ serverURL: String) throws -> String {
        guard let components = URLComponents(string: serverURL),
              let scheme = components.scheme?.lowercased(),
              ["http", "https"].contains(scheme),
              let host = components.host?.lowercased()
        else {
            throw VifuGatewayIdentityStoreError.invalidServerURL
        }
        let port = components.port.map { ":\($0)" } ?? ""
        return "authorization:\(scheme)://\(host)\(port)"
    }

    private func save<Value: Encodable>(_ value: Value, account: String) throws {
        let data = try JSONEncoder().encode(value)
        let query = keychainQuery(account: account)
        let attributes: [CFString: Any] = [
            kSecValueData: data,
            kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let updateStatus = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if updateStatus == errSecSuccess {
            return
        }
        guard updateStatus == errSecItemNotFound else {
            throw VifuGatewayIdentityStoreError.keychain(updateStatus)
        }
        var insert = query
        attributes.forEach { insert[$0] = $1 }
        let insertStatus = SecItemAdd(insert as CFDictionary, nil)
        guard insertStatus == errSecSuccess else {
            throw VifuGatewayIdentityStoreError.keychain(insertStatus)
        }
    }

    private func load<Value: Decodable>(account: String, as: Value.Type) throws -> Value? {
        var query = keychainQuery(account: account)
        query[kSecReturnData] = true
        query[kSecMatchLimit] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw VifuGatewayIdentityStoreError.keychain(status)
        }
        guard let data = result as? Data,
              let value = try? JSONDecoder().decode(Value.self, from: data)
        else {
            throw VifuGatewayIdentityStoreError.invalidStoredValue
        }
        return value
    }

    private func keychainQuery(account: String) -> [CFString: Any] {
        [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
            kSecAttrSynchronizable: false,
        ]
    }
}

public extension VifuEmbeddedGateway {
    func start(
        identity: VifuGatewayMachineIdentity,
        authorization: VifuGatewayAuthorization? = nil,
        enrollmentToken: String? = nil
    ) throws {
        try start(
            machinePrivateKey: identity.privateKey,
            deviceToken: authorization?.deviceToken,
            enrollmentToken: enrollmentToken
        )
    }

    /// Starts monitoring with an explicit application consent decision for invocation content.
    func startWithMonitorIo(
        identity: VifuGatewayMachineIdentity,
        authorization: VifuGatewayAuthorization? = nil,
        enrollmentToken: String? = nil,
        captureMonitorIo: Bool
    ) throws {
        try startWithMonitorIo(
            machinePrivateKey: identity.privateKey,
            deviceToken: authorization?.deviceToken,
            enrollmentToken: enrollmentToken,
            captureMonitorIo: captureMonitorIo
        )
    }
}

public extension VifuEmbeddedGatewayConfig {
    /// Source-compatible initializer for servers using the system trust store.
    init(serverUrl: String, runtimeDatabasePath: String) {
        self.init(
            serverUrl: serverUrl,
            runtimeDatabasePath: runtimeDatabasePath,
            serverCertificateDer: nil
        )
    }
}
