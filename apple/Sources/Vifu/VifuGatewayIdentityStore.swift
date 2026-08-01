import Foundation
import Security

/// Device-local Agent Gateway identity returned by an enrollment flow.
///
/// The credential is stored in Keychain and is only passed to the Rust Gateway
/// while its network connection is running.
public struct VifuGatewayIdentity: Codable, Sendable {
    public let gatewayId: String
    public let credential: String

    public init(gatewayId: String, credential: String) {
        self.gatewayId = gatewayId
        self.credential = credential
    }

    public init(generated: VifuGeneratedGatewayIdentity) {
        self.init(
            gatewayId: generated.gatewayId,
            credential: generated.credential
        )
    }
}

public enum VifuGatewayIdentityStoreError: LocalizedError {
    case invalidIdentity
    case keychain(OSStatus)
    case invalidStoredIdentity

    public var errorDescription: String? {
        switch self {
        case .invalidIdentity:
            "The Agent Gateway identity is invalid."
        case let .keychain(status):
            "Keychain operation failed with status \(status)."
        case .invalidStoredIdentity:
            "The stored Agent Gateway identity is invalid."
        }
    }
}

/// Stores one Gateway identity per runtime project in the Apple Keychain.
public struct VifuGatewayIdentityStore: Sendable {
    private let service: String

    public init(service: String? = nil) {
        self.service = service
            ?? Bundle.main.bundleIdentifier.map { "\($0).vifu.gateway" }
            ?? "dev.vifu.gateway"
    }

    public func save(_ identity: VifuGatewayIdentity, for projectId: String) throws {
        guard !projectId.isEmpty,
              !identity.gatewayId.isEmpty,
              !identity.credential.isEmpty
        else {
            throw VifuGatewayIdentityStoreError.invalidIdentity
        }
        let data = try JSONEncoder().encode(identity)
        let query = keychainQuery(projectId: projectId)
        let attributes: [CFString: Any] = [
            kSecValueData: data,
            kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let updateStatus = SecItemUpdate(
            query as CFDictionary,
            attributes as CFDictionary
        )
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

    public func load(for projectId: String) throws -> VifuGatewayIdentity? {
        var query = keychainQuery(projectId: projectId)
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
              let identity = try? JSONDecoder().decode(VifuGatewayIdentity.self, from: data),
              !identity.gatewayId.isEmpty,
              !identity.credential.isEmpty
        else {
            throw VifuGatewayIdentityStoreError.invalidStoredIdentity
        }
        return identity
    }

    public func delete(for projectId: String) throws {
        let status = SecItemDelete(keychainQuery(projectId: projectId) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw VifuGatewayIdentityStoreError.keychain(status)
        }
    }

    private func keychainQuery(projectId: String) -> [CFString: Any] {
        [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: projectId,
            kSecAttrSynchronizable: false,
        ]
    }
}

public extension VifuEmbeddedGateway {
    func start(
        identity: VifuGatewayIdentity,
        enrollmentToken: String? = nil
    ) throws {
        try start(
            gatewayCredential: identity.credential,
            enrollmentToken: enrollmentToken
        )
    }
}
