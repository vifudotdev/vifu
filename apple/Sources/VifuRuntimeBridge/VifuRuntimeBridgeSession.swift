import Foundation

/// A transport for encoded Runtime Bridge frames.
///
/// Transports move complete UTF-8 JSON frames and do not interpret runtime or
/// application messages. A transport can therefore be reused by Godot, Unity,
/// Unreal, a WebSocket connection, or an in-process host.
public protocol VifuRuntimeBridgeTransport: Actor {
    var incoming: AsyncStream<String> { get }

    func connect() async throws
    func send(_ encodedFrame: String) async throws
    func disconnect() async
}

/// Handles Runtime Bridge requests for an attached runtime.
///
/// The transport package owns only this protocol. Embedded and remote runtime
/// products can implement it without making hosts link a specific FFI binary.
public protocol VifuRuntimeBridgeRuntimeConnection: Actor {
    func handle(_ encodedFrame: String) throws -> [String]
    func frames() -> AsyncStream<String>
}

/// A transport for two runtimes hosted in the same process.
///
/// The engine integration installs the outbound sender when its runtime is
/// ready and calls `receive(_:)` for frames emitted by the engine.
public actor VifuInProcessBridgeTransport: VifuRuntimeBridgeTransport {
    public typealias Sender = @Sendable (String) async throws -> Void

    public nonisolated let incoming: AsyncStream<String>

    private let continuation: AsyncStream<String>.Continuation
    private var sender: (id: UUID, send: Sender)?

    public init() {
        let pair = AsyncStream<String>.makeStream(
            bufferingPolicy: .bufferingNewest(256)
        )
        incoming = pair.stream
        continuation = pair.continuation
    }

    public func connect() async throws {}

    public func send(_ encodedFrame: String) async throws {
        guard let sender else {
            throw VifuRuntimeBridgeTransportError.notConnected
        }
        try await sender.send(encodedFrame)
    }

    public func disconnect() async {
        sender = nil
    }

    @discardableResult
    public func installSender(_ sender: @escaping Sender) -> UUID {
        let id = UUID()
        self.sender = (id, sender)
        return id
    }

    public func removeSender() {
        sender = nil
    }

    /// Removes the sender only when it is still the installed connection.
    ///
    /// Engine adapters use this during synchronous host teardown so a delayed
    /// cleanup task cannot remove a newer connection established by a restart.
    public func removeSender(_ id: UUID) {
        guard sender?.id == id else { return }
        sender = nil
    }

    public func receive(_ encodedFrame: String) {
        continuation.yield(encodedFrame)
    }
}

/// Routes Runtime Bridge requests while preserving application-defined frames.
///
/// An attached embedded runtime handles `runtime.*` requests. Runtime responses
/// and streaming events return through the same transport. Every other frame is
/// broadcast to application subscribers without being interpreted by Vifu.
public actor VifuRuntimeBridgeSession {
    private let transport: any VifuRuntimeBridgeTransport
    private var runtimeConnection: (any VifuRuntimeBridgeRuntimeConnection)?
    private var transportTask: Task<Void, Never>?
    private var runtimeFramesTask: Task<Void, Never>?
    private var subscribers: [UUID: AsyncStream<String>.Continuation] = [:]

    public init(transport: any VifuRuntimeBridgeTransport) {
        self.transport = transport
    }

    /// Subscribes to application frames before the transport begins receiving.
    ///
    /// Hosts should prefer this entry point during startup so a frame emitted
    /// immediately after `connect()` cannot be broadcast before a subscriber
    /// exists.
    public func startFrames() async throws -> AsyncStream<String> {
        let applicationFrames = frames()
        try await start()
        return applicationFrames
    }

    public func start() async throws {
        guard transportTask == nil else { return }
        try await transport.connect()
        let incoming = await transport.incoming
        transportTask = Task { [weak self] in
            for await frame in incoming {
                guard !Task.isCancelled, let self else { return }
                await self.receive(frame)
            }
        }
    }

    public func stop() async {
        transportTask?.cancel()
        transportTask = nil
        runtimeFramesTask?.cancel()
        runtimeFramesTask = nil
        runtimeConnection = nil
        await transport.disconnect()
    }

    public func attachRuntime(_ connection: any VifuRuntimeBridgeRuntimeConnection) {
        runtimeFramesTask?.cancel()
        runtimeConnection = connection
        runtimeFramesTask = Task { [weak self] in
            let frames = await connection.frames()
            for await frame in frames {
                guard !Task.isCancelled, let self else { return }
                await self.sendRuntimeFrame(frame)
            }
        }
    }

    public func detachRuntime() {
        runtimeFramesTask?.cancel()
        runtimeFramesTask = nil
        runtimeConnection = nil
    }

    /// Sends an application or runtime frame to the connected engine.
    public func send(_ encodedFrame: String) async throws {
        try await transport.send(encodedFrame)
    }

    /// Subscribes to frames not consumed by an attached embedded runtime.
    ///
    /// Each subscriber receives every application frame. This lets independent
    /// host services observe engine events without competing for one stream.
    public func frames() -> AsyncStream<String> {
        let id = UUID()
        let pair = AsyncStream<String>.makeStream(
            bufferingPolicy: .bufferingNewest(256)
        )
        pair.continuation.onTermination = { [weak self] _ in
            guard let session = self else { return }
            Task {
                await session.removeSubscriber(id)
            }
        }
        subscribers[id] = pair.continuation
        return pair.stream
    }

    private func receive(_ encodedFrame: String) async {
        guard isRuntimeRequest(encodedFrame), let runtimeConnection else {
            broadcast(encodedFrame)
            return
        }

        do {
            let responses = try await runtimeConnection.handle(encodedFrame)
            for response in responses {
                try await transport.send(response)
            }
        } catch {
            // A host may provide a remote runtime fallback. Preserve the frame
            // instead of silently dropping it when the embedded runtime rejects
            // transport-level input.
            broadcast(encodedFrame)
        }
    }

    private func sendRuntimeFrame(_ encodedFrame: String) async {
        try? await transport.send(encodedFrame)
    }

    private func broadcast(_ encodedFrame: String) {
        for subscriber in subscribers.values {
            subscriber.yield(encodedFrame)
        }
    }

    private func removeSubscriber(_ id: UUID) {
        subscribers.removeValue(forKey: id)
    }

    private func isRuntimeRequest(_ encodedFrame: String) -> Bool {
        guard
            let object = try? JSONSerialization.jsonObject(
                with: Data(encodedFrame.utf8)
            ),
            let frame = object as? [String: Any],
            frame["type"] as? String == "req",
            let method = frame["method"] as? String
        else {
            return false
        }
        return method.hasPrefix("runtime.")
    }
}

public enum VifuRuntimeBridgeTransportError: LocalizedError {
    case notConnected

    public var errorDescription: String? {
        switch self {
        case .notConnected:
            "The runtime bridge transport is not connected."
        }
    }
}
