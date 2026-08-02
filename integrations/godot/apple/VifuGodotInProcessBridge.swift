import Foundation
import VifuRuntimeBridge
@preconcurrency import SwiftGodot
@preconcurrency import SwiftGodotKit

/// Moves encoded Vifu protocol frames across an embedded Godot boundary.
///
/// Protocol parsing, runtime routing, and application message handling remain
/// outside this adapter. It only connects Godot's `GlobalState` node to
/// `VifuInProcessBridgeTransport`.
@MainActor
public final class VifuGodotInProcessBridge {
    private let transport: VifuInProcessBridgeTransport
    private var signalProxy: SignalProxy?
    private var signalCallable: Callable?
    private weak var globalState: Node?

    public init(transport: VifuInProcessBridgeTransport) {
        self.transport = transport
    }

    /// Connects to a Godot instance whose lifecycle is owned by the host app.
    ///
    /// The host remains responsible for creating, starting, iterating,
    /// restarting, and destroying the instance.
    public func connect(to instance: GodotInstance) async throws {
        disconnectSignal()
        guard instance.isStarted() else {
            throw VifuGodotBridgeError.runtimeNotReady
        }
        guard let sceneTree = Engine.getMainLoop() as? SceneTree,
              let root = sceneTree.root
        else {
            throw VifuGodotBridgeError.sceneTreeUnavailable
        }
        guard let globalState = root.findChild(
            pattern: "GlobalState",
            recursive: true,
            owned: false
        ) else {
            throw VifuGodotBridgeError.globalStateUnavailable
        }

        let proxy = SignalProxy()
        let transport = self.transport
        proxy.proxy = { arguments in
            guard let first = arguments.first, let encodedFrame = String(first) else { return }
            Task {
                await transport.receive(encodedFrame)
            }
        }
        let callable = Callable(object: proxy, method: StringName("proxy"))
        guard globalState.connect(
            signal: "godot_message_to_swift",
            callable: callable
        ) == .ok else {
            throw VifuGodotBridgeError.signalConnectionFailed
        }

        self.globalState = globalState
        signalProxy = proxy
        signalCallable = callable
        await transport.installSender { [weak self] encodedFrame in
            guard let self else {
                throw VifuGodotBridgeError.runtimeNotReady
            }
            try await self.sendToGodot(encodedFrame)
        }
    }

    public func disconnect() {
        disconnectSignal()
        let transport = self.transport
        Task {
            await transport.removeSender()
        }
    }

    private func sendToGodot(_ encodedFrame: String) throws {
        guard let globalState else {
            throw VifuGodotBridgeError.runtimeNotReady
        }
        _ = globalState.call(method: "handle_swift_message", Variant(encodedFrame))
    }

    private func disconnectSignal() {
        if let globalState, let signalCallable {
            globalState.disconnect(
                signal: "godot_message_to_swift",
                callable: signalCallable
            )
        }
        signalCallable = nil
        if let signalProxy {
            signalProxy.proxy = nil
            _ = signalProxy.callDeferred(method: "free")
        }
        signalProxy = nil
        globalState = nil
    }
}

public enum VifuGodotBridgeError: LocalizedError {
    case runtimeNotReady
    case sceneTreeUnavailable
    case globalStateUnavailable
    case signalConnectionFailed

    public var errorDescription: String? {
        switch self {
        case .runtimeNotReady:
            "The embedded game runtime is not ready."
        case .sceneTreeUnavailable:
            "The embedded game scene is not available."
        case .globalStateUnavailable:
            "The Vifu bridge node is not available."
        case .signalConnectionFailed:
            "The Vifu bridge signal could not be connected."
        }
    }
}
