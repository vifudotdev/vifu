import Foundation
import Vifu

struct RuntimeTurn: Sendable {
    let text: String
    let firstTokenLatency: TimeInterval?
    let tokensPerSecond: Double?
}

final class EmbeddedRuntime: @unchecked Sendable {
    static let projectID = "ios-embedding"

    let bridgeConnection: VifuRuntimeBridgeConnection
    private let embeddedRuntime: VifuEmbeddedRuntime
    private let gatewayIdentityStore = VifuGatewayIdentityStore()
    private let runtimeDatabasePath: String
    private let gatewayLock = NSLock()
    private var gateway: VifuEmbeddedGateway?
    private var gatewayServerURL: String?
    private var persistedAuthorization: VifuGatewayAuthorization?

    init(modelURL: URL) throws {
        Self.liveTestLog("opening runtime storage")
        runtimeDatabasePath = try Self.gatewaySupportDirectory()
            .appendingPathComponent("runtime.sqlite", isDirectory: false)
            .path
        Self.liveTestLog("creating embedded runtime")
        let runtime = try VifuEmbeddedRuntime.open(
            projectId: Self.projectID,
            databasePath: runtimeDatabasePath
        )
        Self.liveTestLog("loading local model")
        try runtime.registerLlamaProvider(
            providerId: "local-qwen",
            config: VifuLlamaProviderConfig(
                modelPath: modelURL.path,
                contextSize: 4_096,
                gpuLayers: UInt32.max,
                defaultMaxTokens: 220
            )
        )
        Self.liveTestLog("restoring runtime release")
        if try runtime.restoreActiveRuntimeRelease() == nil {
            guard let manifestURL = Bundle.main.url(
                forResource: "StarterRuntime",
                withExtension: "json"
            ) else {
                throw RuntimeBridgeError.starterRuntimeUnavailable
            }
            let manifest = try String(contentsOf: manifestURL, encoding: .utf8)
            _ = try runtime.bootstrapRuntimeRelease(manifestJson: manifest)
        }
        embeddedRuntime = runtime
        bridgeConnection = VifuRuntimeBridgeConnection(runtime: runtime)
    }

    @discardableResult
    func startGateway() throws -> Bool {
        guard let binding = try gatewayIdentityStore.loadServerBinding() else {
            return false
        }
        Self.liveTestLog("loading gateway identity")
        let identity = try gatewayIdentityStore.loadOrCreateMachineIdentity()
        let authorization = try gatewayIdentityStore.loadAuthorization(
            for: binding.serverURL
        )
        Self.liveTestLog("starting gateway")
        try replaceGateway(
            binding: binding,
            identity: identity,
            authorization: authorization,
            enrollmentToken: nil
        )
        Self.liveTestLog("gateway start requested")
        return true
    }

    func enrollGateway(pairingCode: String) throws {
        let enrollment = try VifuGatewayPairingCode(code: pairingCode)
        let binding = VifuGatewayServerBinding(
            serverURL: enrollment.serverURL,
            certificateDER: enrollment.serverCertificateDER,
            certificateSHA256: enrollment.serverCertificateSHA256
        )
        try gatewayIdentityStore.saveServerBinding(binding)
        let identity = try gatewayIdentityStore.loadOrCreateMachineIdentity()
        try replaceGateway(
            binding: binding,
            identity: identity,
            authorization: nil,
            enrollmentToken: enrollment.enrollmentToken
        )
    }

    func gatewayStatus() throws -> VifuEmbeddedGatewayStatus? {
        let (currentGateway, serverURL, storedAuthorization) = gatewayLock.withLock {
            (gateway, gatewayServerURL, persistedAuthorization)
        }
        guard let status = try currentGateway?.status() else { return nil }
        if let authorization = status.authorization,
           let serverURL,
           authorization != storedAuthorization {
            try gatewayIdentityStore.saveAuthorization(
                authorization,
                for: serverURL
            )
            gatewayLock.withLock {
                guard gatewayServerURL == serverURL else { return }
                persistedAuthorization = authorization
            }
        }
        return status
    }

    private func replaceGateway(
        binding: VifuGatewayServerBinding,
        identity: VifuGatewayMachineIdentity,
        authorization: VifuGatewayAuthorization?,
        enrollmentToken: String?
    ) throws {
        let previousGateway = gatewayLock.withLock { () -> VifuEmbeddedGateway? in
            defer {
                gateway = nil
                gatewayServerURL = nil
                persistedAuthorization = nil
            }
            return gateway
        }
        try previousGateway?.stop()
        let nextGateway = try VifuEmbeddedGateway(
            runtime: embeddedRuntime,
            config: VifuEmbeddedGatewayConfig(
                serverUrl: binding.serverURL,
                runtimeDatabasePath: runtimeDatabasePath,
                serverCertificateDer: binding.certificateDER
            )
        )
        try nextGateway.start(
            identity: identity,
            authorization: authorization,
            enrollmentToken: enrollmentToken
        )
        gatewayLock.withLock {
            gateway = nextGateway
            gatewayServerURL = binding.serverURL
            persistedAuthorization = authorization
        }
    }

    private static func gatewaySupportDirectory() throws -> URL {
        guard let root = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first else {
            throw RuntimeBridgeError.storageUnavailable
        }
        let directory = root.appendingPathComponent("VifuIOSEmbedding", isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        let database = directory.appendingPathComponent("runtime.sqlite", isDirectory: false)
        if !FileManager.default.fileExists(atPath: database.path),
           !FileManager.default.createFile(atPath: database.path, contents: nil) {
            throw RuntimeBridgeError.storageUnavailable
        }
        return directory
    }

    private static func liveTestLog(_ message: String) {
#if DEBUG
        if ProcessInfo.processInfo.environment["VIFU_LIVE_TEST"] == "1" {
            print("VIFU_LIVE_TEST_STAGE=\(message)")
        }
#endif
    }

    func reply(
        messages: [ChatMessage],
        onDelta: @escaping @Sendable (String) async -> Void
    ) async throws -> RuntimeTurn {
        let request = ChatRequest(
            messages: messages.map { .init(role: $0.role.rawValue, content: $0.text) }
        )
        let requestPayload = try jsonObject(request)
        let requestID = UUID().uuidString
        let events = await bridgeConnection.frames()
        let responses = try await bridgeConnection.handle(
            try encodeFrame([
                "type": "req",
                "id": requestID,
                "method": "runtime.invoke",
                "params": [
                    "endpoint": "chat",
                    "sessionId": "local-user",
                    "data": [
                        "format": "json",
                        "value": requestPayload,
                    ],
                    "metadata": [:],
                ],
            ])
        )
        guard let response = responses.first.flatMap(decodeFrame),
              response["ok"] as? Bool == true,
              let payload = response["payload"] as? [String: Any],
              let handle = payload["handle"] as? String
        else {
            throw RuntimeBridgeError.invocationFailed("The local runtime rejected the request.")
        }
        let startedAt = ContinuousClock.now
        var firstTokenAt: ContinuousClock.Instant?
        var accumulated = ""

        for await encodedEvent in events {
            try Task.checkCancellation()
            guard let frame = decodeFrame(encodedEvent),
                  frame["type"] as? String == "event",
                  let payload = frame["payload"] as? [String: Any],
                  payload["handle"] as? String == handle,
                  let event = payload["event"] as? [String: Any],
                  let kind = event["kind"] as? String
            else {
                continue
            }
            switch kind {
            case "outputDelta":
                guard let delta = invocationText(event["data"]), !delta.isEmpty else {
                    continue
                }
                if firstTokenAt == nil { firstTokenAt = .now }
                accumulated += delta
                await onDelta(delta)
            case "completed":
                let output = payload["output"] as? [String: Any]
                let finalText = invocationText(output?["data"]) ?? accumulated
                let metadata = output?["metadata"] as? [String: Any]
                let outputTokens = metadata?["outputTokens"] as? Int
                let elapsed = startedAt.duration(to: .now).seconds
                let firstTokenLatency = firstTokenAt.map {
                    startedAt.duration(to: $0).seconds
                }
                return RuntimeTurn(
                    text: finalText.trimmingCharacters(in: .whitespacesAndNewlines),
                    firstTokenLatency: firstTokenLatency,
                    tokensPerSecond: outputTokens.map {
                        Double($0) / max(elapsed, 0.001)
                    }
                )
            case "failed":
                throw RuntimeBridgeError.invocationFailed(
                    event["error"] as? String ?? "Local inference failed."
                )
            case "cancelled":
                throw CancellationError()
            case "started":
                break
            default:
                continue
            }
        }
        throw RuntimeBridgeError.invocationFailed("The local runtime event stream ended.")
    }

    private func jsonObject(_ value: some Encodable) throws -> Any {
        let data = try JSONEncoder().encode(value)
        return try JSONSerialization.jsonObject(with: data)
    }

    private func encodeFrame(_ frame: [String: Any]) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: frame)
        guard let encoded = String(data: data, encoding: .utf8) else {
            throw RuntimeBridgeError.encodingFailed
        }
        return encoded
    }

    private func decodeFrame(_ encoded: String) -> [String: Any]? {
        guard let value = try? JSONSerialization.jsonObject(with: Data(encoded.utf8)) else {
            return nil
        }
        return value as? [String: Any]
    }

    private func invocationText(_ data: Any?) -> String? {
        guard let data = data as? [String: Any],
              data["format"] as? String == "json"
        else {
            return nil
        }
        if let text = data["value"] as? String {
            return text
        }
        return (data["value"] as? [String: Any])?["text"] as? String
    }
}

private struct ChatRequest: Encodable {
    struct Message: Encodable {
        let role: String
        let content: String
    }

    let messages: [Message]
}

private enum RuntimeBridgeError: LocalizedError {
    case encodingFailed
    case invocationFailed(String)
    case storageUnavailable
    case starterRuntimeUnavailable

    var errorDescription: String? {
        switch self {
        case .encodingFailed:
            "The message could not be prepared."
        case let .invocationFailed(message):
            message
        case .storageUnavailable:
            "Vifu could not open its local storage."
        case .starterRuntimeUnavailable:
            "The bundled starter runtime is unavailable."
        }
    }
}

private extension Duration {
    var seconds: Double {
        let components = self.components
        return Double(components.seconds)
            + Double(components.attoseconds) / 1_000_000_000_000_000_000
    }
}
