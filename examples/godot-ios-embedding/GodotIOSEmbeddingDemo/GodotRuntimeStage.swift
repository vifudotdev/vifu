import Foundation
import SwiftUI
import Vifu

struct GodotRuntimeStage: View {
    let activity: GodotEmbeddingViewModel.Activity
    let runtimeBridge: VifuRuntimeBridgeConnection?

    @StateObject private var host = GodotRuntimeHost()

    var body: some View {
        ZStack {
            Color.clear
            VifuGodotView(godotApp: host.godotApp)
        }
            .background(Color.appBackground)
            .task {
                await host.start()
                host.send(activity)
            }
            .task(id: runtimeBridge.map(ObjectIdentifier.init)) {
                guard let runtimeBridge else { return }
                await host.attach(runtimeBridge)
            }
            .onChange(of: activity) {
                host.send(activity)
            }
    }
}

@MainActor
private final class GodotRuntimeHost: ObservableObject {
    let godotApp = VifuGodotApp()

    private let transport = VifuInProcessBridgeTransport()
    private lazy var bridge = VifuGodotInProcessBridge(transport: transport)
    private lazy var bridgeSession = VifuRuntimeBridgeSession(transport: transport)
    private var started = false
    private var connected = false
    private var runtimeBridge: VifuRuntimeBridgeConnection?
    private var acknowledgedActivity: String?
    private var runtimeProjectID: String?

    func start() async {
        guard !started else { return }
        started = true
        godotApp.boot(packFile: "godot-ios-embedding")

        for _ in 0 ..< 80 {
            do {
                guard let instance = godotApp.instance else {
                    throw VifuGodotBridgeError.runtimeNotReady
                }
                try await bridge.connect(to: instance)
                try await bridgeSession.start()
                connected = true
                observeEngineFrames()
                if let runtimeBridge {
                    await attach(runtimeBridge)
                }
                return
            } catch {
                try? await Task.sleep(for: .milliseconds(100))
            }
        }
    }

    func attach(_ runtimeBridge: VifuRuntimeBridgeConnection) async {
        self.runtimeBridge = runtimeBridge
        guard connected else { return }
        await bridgeSession.attachRuntime(runtimeBridge)
        await sendEvent("host.runtime.available")
    }

    func send(_ activity: GodotEmbeddingViewModel.Activity) {
        guard connected else { return }
        let activityName: String
        switch activity {
        case .loading:
            activityName = "loading"
        case .idle:
            activityName = "idle"
        case .thinking:
            activityName = "thinking"
        case .speaking:
            activityName = "speaking"
        case .failed:
            activityName = "failed"
        }
        Task {
            await sendEvent("host.activity", payload: ["activity": activityName])
        }
    }

    private func sendEvent(_ event: String, payload: [String: Any] = [:]) async {
        let frame: [String: Any] = [
            "type": "event",
            "event": event,
            "payload": payload,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: frame),
              let encoded = String(data: data, encoding: .utf8)
        else {
            return
        }
        try? await bridgeSession.send(encoded)
    }

    private func observeEngineFrames() {
        Task { [weak self, bridgeSession] in
            let incoming = await bridgeSession.frames()
            for await encodedFrame in incoming {
                guard !Task.isCancelled,
                      let data = encodedFrame.data(using: .utf8),
                      let frame = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      frame["type"] as? String == "event",
                      let event = frame["event"] as? String,
                      let payload = frame["payload"] as? [String: Any]
                else {
                    continue
                }
                switch event {
                case "stage.activity.changed":
                    self?.acknowledgedActivity = payload["activity"] as? String
                case "stage.runtime.connected":
                    self?.runtimeProjectID = payload["projectId"] as? String
                default:
                    continue
                }
            }
        }
    }
}
