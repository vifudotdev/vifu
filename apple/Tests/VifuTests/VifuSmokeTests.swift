import XCTest
@testable import Vifu
@testable import VifuRuntimeBridge

final class VifuSmokeTests: XCTestCase {
    func testEmbeddedRuntimeLinksAndExportsState() throws {
        let runtime = try VifuEmbeddedRuntime(projectId: "swift-package-smoke")
        let snapshot = try runtime.exportSnapshot()

        XCTAssertFalse(snapshot.isEmpty)
    }

    func testRuntimeBridgeConnectionBroadcastsEventsToEveryClient() async throws {
        let runtime = try VifuEmbeddedRuntime(projectId: "swift-bridge-test")
        try runtime.registerProvider(providerId: "echo", provider: EchoProvider())
        try runtime.registerAgent(
            agentId: "guide",
            name: "Guide",
            providerId: "echo",
            capabilities: ["chat"],
            metadataJson: "{}"
        )
        try runtime.registerEndpoint(
            name: "guide",
            agentId: "guide",
            capability: "chat",
            timeoutMs: 1_000
        )

        let connection = VifuRuntimeBridgeConnection(runtime: runtime)
        let firstFrames = await connection.frames()
        let secondFrames = await connection.frames()
        let completed = expectation(description: "both bridge clients receive completion")
        completed.expectedFulfillmentCount = 2

        let firstTask = observeCompletion(in: firstFrames, expectation: completed)
        let secondTask = observeCompletion(in: secondFrames, expectation: completed)
        let responses = try await connection.handle(
            """
            {"type":"req","id":"invoke-1","method":"runtime.invoke","params":{"endpoint":"guide","sessionId":"player-one","data":{"format":"json","value":{"message":"hello"}},"metadata":{}}}
            """
        )

        XCTAssertEqual(responses.count, 1)
        XCTAssertTrue(responses[0].contains(#""ok":true"#))
        await fulfillment(of: [completed], timeout: 2)
        firstTask.cancel()
        secondTask.cancel()
    }

    func testRuntimeBridgeSessionRoutesRuntimeAndApplicationFrames() async throws {
        let runtime = try VifuEmbeddedRuntime(projectId: "swift-session-test")
        try runtime.registerProvider(providerId: "echo", provider: EchoProvider())
        try runtime.registerAgent(
            agentId: "guide",
            name: "Guide",
            providerId: "echo",
            capabilities: ["chat"],
            metadataJson: "{}"
        )
        try runtime.registerEndpoint(
            name: "guide",
            agentId: "guide",
            capability: "chat",
            timeoutMs: 1_000
        )

        let transport = TestBridgeTransport()
        let session = VifuRuntimeBridgeSession(transport: transport)
        let applicationFrames = try await session.startFrames()
        await session.attachRuntime(VifuRuntimeBridgeConnection(runtime: runtime))

        let applicationFrameReceived = expectation(
            description: "application frame reaches the host"
        )
        let applicationTask = Task {
            for await frame in applicationFrames {
                guard frame.contains("stage.ready") else { continue }
                applicationFrameReceived.fulfill()
                return
            }
        }

        let runtimeResponseReceived = expectation(
            description: "runtime response returns to the engine"
        )
        let runtimeCompletionReceived = expectation(
            description: "runtime event returns to the engine"
        )
        let sentFrames = transport.sent
        let sentTask = Task {
            for await frame in sentFrames {
                if frame.contains(#""id":"invoke-from-engine""#),
                   frame.contains(#""ok":true"#) {
                    runtimeResponseReceived.fulfill()
                }
                if frame.contains("runtime.invocation.completed"),
                   frame.contains(#""message":"hello from Godot""#) {
                    runtimeCompletionReceived.fulfill()
                    return
                }
            }
        }

        await transport.inject(
            #"{"type":"event","event":"stage.ready","payload":{}}"#
        )
        await transport.inject(
            """
            {"type":"req","id":"invoke-from-engine","method":"runtime.invoke","params":{"endpoint":"guide","sessionId":"player-one","data":{"format":"json","value":{"message":"hello from Godot"}},"metadata":{}}}
            """
        )

        await fulfillment(
            of: [
                applicationFrameReceived,
                runtimeResponseReceived,
                runtimeCompletionReceived,
            ],
            timeout: 2
        )
        applicationTask.cancel()
        sentTask.cancel()
        await session.stop()
    }

    func testRuntimeBridgeSessionPreservesRuntimeRequestsWithoutLocalRuntime() async throws {
        let transport = TestBridgeTransport()
        let session = VifuRuntimeBridgeSession(transport: transport)
        let applicationFrames = try await session.startFrames()
        let forwarded = expectation(
            description: "host can route runtime request to a remote runtime"
        )
        let task = Task {
            for await frame in applicationFrames {
                guard frame.contains("runtime.hello") else { continue }
                forwarded.fulfill()
                return
            }
        }

        await transport.inject(
            #"{"type":"req","id":"hello","method":"runtime.hello","params":{}}"#
        )

        await fulfillment(of: [forwarded], timeout: 1)
        task.cancel()
        await session.stop()
    }

    func testRuntimeBridgeSessionSubscribesBeforeReceivingStartupFrames() async throws {
        let startupFrame = #"{"type":"event","event":"stage.ready","payload":{}}"#
        let transport = TestBridgeTransport(frameOnConnect: startupFrame)
        let session = VifuRuntimeBridgeSession(transport: transport)

        let applicationFrames = try await session.startFrames()
        let received = expectation(description: "startup frame reaches the host")
        let task = Task {
            for await frame in applicationFrames {
                guard frame == startupFrame else { continue }
                received.fulfill()
                return
            }
        }

        await fulfillment(of: [received], timeout: 1)
        task.cancel()
        await session.stop()
    }

    private func observeCompletion(
        in frames: AsyncStream<String>,
        expectation: XCTestExpectation
    ) -> Task<Void, Never> {
        Task {
            for await frame in frames {
                guard frame.contains("runtime.invocation.completed") else { continue }
                XCTAssertTrue(frame.contains(#""message":"hello""#))
                expectation.fulfill()
                return
            }
        }
    }
}

private actor TestBridgeTransport: VifuRuntimeBridgeTransport {
    nonisolated let incoming: AsyncStream<String>
    nonisolated let sent: AsyncStream<String>

    private let incomingContinuation: AsyncStream<String>.Continuation
    private let sentContinuation: AsyncStream<String>.Continuation
    private let frameOnConnect: String?

    init(frameOnConnect: String? = nil) {
        self.frameOnConnect = frameOnConnect
        let incomingPair = AsyncStream<String>.makeStream()
        incoming = incomingPair.stream
        incomingContinuation = incomingPair.continuation

        let sentPair = AsyncStream<String>.makeStream()
        sent = sentPair.stream
        sentContinuation = sentPair.continuation
    }

    func connect() async throws {
        if let frameOnConnect {
            incomingContinuation.yield(frameOnConnect)
        }
    }

    func send(_ encodedFrame: String) async throws {
        sentContinuation.yield(encodedFrame)
    }

    func disconnect() async {}

    func inject(_ encodedFrame: String) {
        incomingContinuation.yield(encodedFrame)
    }
}

private final class EchoProvider: VifuAgentProvider, @unchecked Sendable {
    func invoke(request: VifuProviderRequest) throws -> VifuProviderResponse {
        VifuProviderResponse(
            data: request.data,
            metadataJson: #"{"contentType":"application/json"}"#,
            stateJson: nil
        )
    }
}
