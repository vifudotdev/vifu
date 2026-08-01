import Foundation

/// Coordinates one embedded Runtime Bridge across multiple in-process clients.
///
/// Game-engine adapters and native UI code can share this connection without
/// competing to drain the Runtime's streaming event queue.
public actor VifuRuntimeBridgeConnection {
    private let runtime: VifuEmbeddedRuntime
    private var subscribers: [UUID: AsyncStream<String>.Continuation] = [:]
    private var eventPump: Task<Void, Never>?

    public init(runtime: VifuEmbeddedRuntime) {
        self.runtime = runtime
    }

    /// Sends one encoded Runtime Bridge request and returns its response frames.
    public func handle(_ encodedFrame: String) throws -> [String] {
        try runtime.handleBridgeFrame(encodedFrame: encodedFrame)
    }

    /// Subscribes to Runtime Bridge event frames.
    ///
    /// Each subscriber receives every event, so native UI and an embedded game
    /// engine can independently filter invocation handles without losing data.
    public func frames() -> AsyncStream<String> {
        let id = UUID()
        let (stream, continuation) = AsyncStream<String>.makeStream(
            bufferingPolicy: .bufferingNewest(256)
        )
        continuation.onTermination = { [weak self] _ in
            guard let connection = self else { return }
            Task {
                await connection.removeSubscriber(id)
            }
        }
        subscribers[id] = continuation
        startEventPump()
        return stream
    }

    private func startEventPump() {
        guard eventPump == nil else { return }
        eventPump = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                await self.drainAndBroadcast()
                try? await Task.sleep(for: .milliseconds(30))
            }
        }
    }

    private func drainAndBroadcast() {
        guard !subscribers.isEmpty else { return }
        guard let frames = try? runtime.drainBridgeFrames() else { return }
        for frame in frames {
            for subscriber in subscribers.values {
                subscriber.yield(frame)
            }
        }
    }

    private func removeSubscriber(_ id: UUID) {
        subscribers.removeValue(forKey: id)
        guard subscribers.isEmpty else { return }
        eventPump?.cancel()
        eventPump = nil
    }
}
