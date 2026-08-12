import AVFoundation
import Foundation
import Observation
import Vifu

struct ChatMessage: Identifiable, Sendable {
    enum Role: String, Sendable {
        case user
        case assistant
    }

    let id: UUID
    let role: Role
    var text: String

    init(id: UUID = UUID(), role: Role, text: String) {
        self.id = id
        self.role = role
        self.text = text
    }
}

@MainActor
@Observable
final class GodotEmbeddingViewModel {
    enum Activity: Equatable {
        case loading
        case idle
        case thinking
        case speaking
        case failed(String)
    }

    var draft = ""
    private(set) var messages = [
        ChatMessage(
            role: .assistant,
            text: "Hi. What is on your mind?"
        ),
    ]
    private(set) var activity: Activity = .loading
    private(set) var firstTokenLatency: TimeInterval?
    private(set) var tokensPerSecond: Double?
    private(set) var gatewayLabel = "Not paired"
    private(set) var gatewayError: String?

    private let modelURL: URL
    private let synthesizer = AVSpeechSynthesizer()
    private var runtime: GodotEmbeddedRuntime?
    private var gatewayPollingTask: Task<Void, Never>?
    private var reportedLiveTestGatewayStatus: String?

    init(modelURL: URL) {
        self.modelURL = modelURL
    }

    var canSend: Bool {
        !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && activity != .loading
            && activity != .thinking
    }

    var activityLabel: String {
        switch activity {
        case .loading:
            "Loading local model"
        case .idle:
            "Ready on device"
        case .thinking:
            "Thinking"
        case .speaking:
            "Speaking"
        case .failed:
            "Needs attention"
        }
    }

    var runtimeBridge: VifuRuntimeBridgeConnection? {
        runtime?.bridgeConnection
    }

    func start() async {
        guard runtime == nil else { return }
        activity = .loading
        let modelURL = modelURL
        do {
            let runtime = try await Task.detached(priority: .userInitiated) {
                try GodotEmbeddedRuntime(modelURL: modelURL)
            }.value
            self.runtime = runtime
            activity = .idle
            do {
                let paired = try await Task.detached(priority: .userInitiated) {
                    try runtime.startGateway()
                }.value
                if paired {
                    gatewayLabel = "Gateway connecting"
                    startGatewayPolling()
                }
            } catch {
                Self.logLiveTestError(error, stage: "gateway-start")
                gatewayLabel = "Gateway unavailable"
                gatewayError = error.localizedDescription
            }
        } catch {
            Self.logLiveTestError(error, stage: "runtime-start")
            activity = .failed(error.localizedDescription)
        }
    }

    private static func logLiveTestError(_ error: Error, stage: String) {
#if DEBUG
        guard ProcessInfo.processInfo.environment["VIFU_LIVE_TEST"] == "1" else { return }
        let error = error as NSError
        print("VIFU_LIVE_TEST_ERROR=\(stage):\(error.domain):\(error.code)")
#endif
    }

    func enrollGateway(pairingCode: String) async {
        guard let runtime else { return }
        gatewayLabel = "Connecting Gateway"
        gatewayError = nil
        do {
            try await Task.detached(priority: .userInitiated) {
                try runtime.enrollGateway(pairingCode: pairingCode)
            }.value
            startGatewayPolling()
        } catch {
            gatewayLabel = "Gateway unavailable"
            gatewayError = error.localizedDescription
        }
    }

    func send() async {
        guard canSend, let runtime else { return }
        let prompt = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        draft = ""
        synthesizer.stopSpeaking(at: .immediate)
        messages.append(.init(role: .user, text: prompt))
        let assistantID = UUID()
        messages.append(.init(id: assistantID, role: .assistant, text: ""))
        activity = .thinking
        firstTokenLatency = nil
        tokensPerSecond = nil
        let requestMessages = Array(messages.dropLast())

        do {
            let turn = try await runtime.reply(messages: requestMessages) { [self] delta in
                await append(delta, to: assistantID)
            }
            if let index = messages.firstIndex(where: { $0.id == assistantID }) {
                messages[index].text = turn.text
            }
            firstTokenLatency = turn.firstTokenLatency
            tokensPerSecond = turn.tokensPerSecond
            speak(turn.text)
        } catch {
            if let index = messages.firstIndex(where: { $0.id == assistantID }) {
                messages[index].text = "I could not finish that reply."
            }
            activity = .failed(error.localizedDescription)
        }
    }

    func stopSpeaking() {
        synthesizer.stopSpeaking(at: .immediate)
        activity = .idle
    }

    private func append(_ delta: String, to messageID: UUID) {
        guard let index = messages.firstIndex(where: { $0.id == messageID }) else { return }
        messages[index].text += delta
    }

    private func speak(_ text: String) {
        guard !text.isEmpty else {
            activity = .idle
            return
        }
        let utterance = AVSpeechUtterance(string: text)
        utterance.rate = 0.48
        utterance.pitchMultiplier = 1.03
        utterance.voice = AVSpeechSynthesisVoice(language: "en-US")
        synthesizer.speak(utterance)
        activity = .speaking
        Task {
            while synthesizer.isSpeaking {
                try? await Task.sleep(for: .milliseconds(100))
            }
            if activity == .speaking { activity = .idle }
        }
    }

    private func startGatewayPolling() {
        gatewayPollingTask?.cancel()
        gatewayPollingTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                self.refreshGatewayStatus()
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    private func refreshGatewayStatus() {
        guard let runtime, let status = try? runtime.gatewayStatus() else { return }
        gatewayError = status.lastError
        reportLiveTestGatewayStatus(status)
        switch status.state {
        case .connected:
            gatewayLabel = "Gateway connected"
        case .connecting:
            gatewayLabel = "Gateway connecting"
        case .reconnecting:
            gatewayLabel = "Gateway reconnecting"
        case .authorizationRequired:
            gatewayLabel = "Gateway authorization required"
        case .degraded:
            gatewayLabel = "Gateway connected with warnings"
        case .stopped:
            gatewayLabel = "Gateway stopped"
        case .failed:
            gatewayLabel = "Gateway unavailable"
        }
    }

    private func reportLiveTestGatewayStatus(_ status: VifuEmbeddedGatewayStatus) {
#if DEBUG
        guard ProcessInfo.processInfo.environment["VIFU_LIVE_TEST"] == "1" else { return }
        let category: String
        if let error = status.lastError?.lowercased() {
            if error.contains("credential") || error.contains("auth") || error.contains("401") {
                category = "authentication"
            } else if error.contains("timeout") || error.contains("timed out") {
                category = "timeout"
            } else if error.contains("connect") || error.contains("dns") || error.contains("tls") {
                category = "connection"
            } else if error.contains("guest") || error.contains("bootstrap") {
                category = "guest-bootstrap"
            } else {
                category = "unknown"
            }
        } else {
            category = "none"
        }
        let value = "\(status.state):\(category)"
        guard value != reportedLiveTestGatewayStatus else { return }
        reportedLiveTestGatewayStatus = value
        print("VIFU_LIVE_TEST_GATEWAY_STATUS=\(value)")
        if let error = status.lastError {
            print("VIFU_LIVE_TEST_GATEWAY_ERROR=\(Self.redactLiveTestError(error))")
        }
#endif
    }

    private static func redactLiveTestError(_ value: String) -> String {
        let patterns = [
            #"https?://\S+"#,
            #"(?i)vifu_[a-z]+_[A-Za-z0-9_-]+"#,
            #"(?:/[^\s:]+)+"#,
        ]
        return patterns.reduce(value) { result, pattern in
            result.replacingOccurrences(
                of: pattern,
                with: "<redacted>",
                options: .regularExpression
            )
        }
    }
}
