import CryptoKit
import Foundation

public enum VifuGatewayPairingCodeError: LocalizedError {
    case invalidCode

    public var errorDescription: String? {
        "This Vifu pairing code is invalid or has expired."
    }
}

/// A one-time Agent Gateway enrollment bound to one Vifu Server URL.
public struct VifuGatewayPairingCode: Sendable {
    public let serverURL: String
    public let enrollmentToken: String
    public let serverCertificateDER: Data?
    public let serverCertificateSHA256: String?

    public init(code: String) throws {
        guard let components = URLComponents(string: code),
              components.scheme == "vifu",
              components.host == "gateway",
              components.path == "/enroll",
              let serverURL = components.queryItems?.first(
                where: { $0.name == "server" }
              )?.value,
              Self.isHTTPSOrigin(serverURL),
              let token = components.queryItems?.first(
                where: { $0.name == "token" }
              )?.value,
              token.hasPrefix("vifu_ge_"),
              token.utf8.count == 72,
              token.utf8.dropFirst(8).allSatisfy(Self.isASCIIHexDigit)
        else {
            throw VifuGatewayPairingCodeError.invalidCode
        }

        let encodedCertificate = components.queryItems?.first(
            where: { $0.name == "certificate" }
        )?.value
        let fingerprint = components.queryItems?.first(
            where: { $0.name == "fingerprint" }
        )?.value
        let certificate: Data?
        switch (encodedCertificate, fingerprint) {
        case (nil, nil):
            certificate = nil
        case let (encoded?, fingerprint?):
            guard let decoded = Data(base64Encoded: encoded),
                  !decoded.isEmpty,
                  fingerprint == Self.fingerprint(decoded)
            else {
                throw VifuGatewayPairingCodeError.invalidCode
            }
            certificate = decoded
        default:
            throw VifuGatewayPairingCodeError.invalidCode
        }

        self.serverURL = serverURL
        enrollmentToken = token
        serverCertificateDER = certificate
        serverCertificateSHA256 = fingerprint
    }

    private static func isHTTPSOrigin(_ value: String) -> Bool {
        guard let components = URLComponents(string: value),
              components.scheme?.lowercased() == "https",
              let host = components.host,
              !host.isEmpty,
              components.user == nil,
              components.password == nil,
              components.query == nil,
              components.fragment == nil
        else {
            return false
        }
        return components.path.isEmpty || components.path == "/"
    }

    private static func fingerprint(_ certificateDER: Data) -> String {
        let digest = SHA256.hash(data: certificateDER)
        return "sha256:" + digest.map { String(format: "%02x", $0) }.joined()
    }

    private static func isASCIIHexDigit(_ byte: UInt8) -> Bool {
        (48 ... 57).contains(byte) || (65 ... 70).contains(byte) || (97 ... 102).contains(byte)
    }
}
