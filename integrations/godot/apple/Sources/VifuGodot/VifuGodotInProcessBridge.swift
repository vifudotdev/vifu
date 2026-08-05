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
    private var host: (any VifuGodotBridgeHost)?
    private var senderID: UUID?

    public init(transport: VifuInProcessBridgeTransport) {
        self.transport = transport
    }

    /// Connects to a Godot instance whose lifecycle is owned by the host app.
    ///
    /// The host remains responsible for creating, starting, iterating,
    /// restarting, and destroying the instance.
    public func connect(to instance: GodotInstance) async throws {
        try await connect(to: SwiftGodotBridgeHost(instance: instance))
    }

    /// Disconnects the Godot signal immediately and removes the transport
    /// sender asynchronously.
    public func disconnect() {
        disconnectHost()
        guard let senderID else { return }
        self.senderID = nil
        let transport = self.transport
        Task {
            await transport.removeSender(senderID)
        }
    }

    func connect(to host: any VifuGodotBridgeHost) async throws {
        disconnectHost()
        if let senderID {
            await transport.removeSender(senderID)
            self.senderID = nil
        }

        do {
            let transport = self.transport
            try host.installReceiver { encodedFrame in
                Task {
                    await transport.receive(encodedFrame)
                }
            }
        } catch {
            host.disconnect()
            throw error
        }

        self.host = host
        senderID = await transport.installSender { [weak self] encodedFrame in
            guard let self else {
                throw VifuGodotBridgeError.runtimeNotReady
            }
            try await self.sendToGodot(encodedFrame)
        }
    }

    func disconnectAndWait() async {
        disconnectHost()
        guard let senderID else { return }
        self.senderID = nil
        await transport.removeSender(senderID)
    }

    private func sendToGodot(_ encodedFrame: String) throws {
        guard let host else {
            throw VifuGodotBridgeError.runtimeNotReady
        }
        try host.send(encodedFrame)
    }

    private func disconnectHost() {
        host?.disconnect()
        host = nil
    }
}

@MainActor
protocol VifuGodotBridgeHost: AnyObject {
    func installReceiver(_ receiver: @escaping @Sendable (String) -> Void) throws
    func send(_ encodedFrame: String) throws
    func disconnect()
}

@MainActor
private final class SwiftGodotBridgeHost: VifuGodotBridgeHost {
    private var signalProxy: SignalProxy?
    private var signalCallable: Callable?
    private weak var globalState: Node?

    init(instance: GodotInstance) throws {
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
        self.globalState = globalState
    }

    func installReceiver(
        _ receiver: @escaping @Sendable (String) -> Void
    ) throws {
        guard let globalState else {
            throw VifuGodotBridgeError.runtimeNotReady
        }

        let proxy = SignalProxy()
        proxy.proxy = { arguments in
            guard let first = arguments.first,
                  let encodedFrame = String(first)
            else {
                return
            }
            receiver(encodedFrame)
        }
        let callable = Callable(object: proxy, method: StringName("proxy"))
        guard globalState.connect(
            signal: "godot_message_to_swift",
            callable: callable
        ) == .ok else {
            proxy.proxy = nil
            _ = proxy.callDeferred(method: "free")
            throw VifuGodotBridgeError.signalConnectionFailed
        }

        signalProxy = proxy
        signalCallable = callable
    }

    func send(_ encodedFrame: String) throws {
        guard let globalState else {
            throw VifuGodotBridgeError.runtimeNotReady
        }
        _ = globalState.call(method: "handle_swift_message", Variant(encodedFrame))
    }

    func disconnect() {
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
