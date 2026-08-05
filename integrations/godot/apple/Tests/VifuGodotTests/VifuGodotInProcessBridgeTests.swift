import XCTest
import VifuRuntimeBridge
@testable import VifuGodot

@MainActor
final class VifuGodotInProcessBridgeTests: XCTestCase {
    func testForwardsGodotFramesToTransport() async throws {
        let transport = VifuInProcessBridgeTransport()
        let bridge = VifuGodotInProcessBridge(transport: transport)
        let host = TestGodotBridgeHost()
        var incoming = transport.incoming.makeAsyncIterator()

        try await bridge.connect(to: host)
        host.receive("from-godot")

        let frame = await incoming.next()
        XCTAssertEqual(frame, "from-godot")
    }

    func testForwardsTransportFramesToGodot() async throws {
        let transport = VifuInProcessBridgeTransport()
        let bridge = VifuGodotInProcessBridge(transport: transport)
        let host = TestGodotBridgeHost()

        try await bridge.connect(to: host)
        try await transport.send("to-godot")

        XCTAssertEqual(host.sentFrames, ["to-godot"])
    }

    func testReconnectDisconnectsPreviousHost() async throws {
        let transport = VifuInProcessBridgeTransport()
        let bridge = VifuGodotInProcessBridge(transport: transport)
        let firstHost = TestGodotBridgeHost()
        let secondHost = TestGodotBridgeHost()

        try await bridge.connect(to: firstHost)
        try await bridge.connect(to: secondHost)
        try await transport.send("latest")

        XCTAssertEqual(firstHost.disconnectCount, 1)
        XCTAssertEqual(firstHost.sentFrames, [])
        XCTAssertEqual(secondHost.sentFrames, ["latest"])
    }

    func testDisconnectRemovesTransportSender() async throws {
        let transport = VifuInProcessBridgeTransport()
        let bridge = VifuGodotInProcessBridge(transport: transport)
        let host = TestGodotBridgeHost()

        try await bridge.connect(to: host)
        await bridge.disconnectAndWait()

        XCTAssertEqual(host.disconnectCount, 1)
        do {
            try await transport.send("after-disconnect")
            XCTFail("Expected the disconnected transport to reject sends")
        } catch VifuRuntimeBridgeTransportError.notConnected {
            // Expected.
        }
    }

    func testImmediateReconnectDoesNotLoseNewSender() async throws {
        let transport = VifuInProcessBridgeTransport()
        let bridge = VifuGodotInProcessBridge(transport: transport)
        let firstHost = TestGodotBridgeHost()
        let secondHost = TestGodotBridgeHost()

        try await bridge.connect(to: firstHost)
        bridge.disconnect()
        try await bridge.connect(to: secondHost)
        await Task.yield()
        try await transport.send("after-reconnect")

        XCTAssertEqual(firstHost.disconnectCount, 1)
        XCTAssertEqual(secondHost.sentFrames, ["after-reconnect"])
    }
}

@MainActor
private final class TestGodotBridgeHost: VifuGodotBridgeHost {
    private var receiver: (@Sendable (String) -> Void)?
    private(set) var sentFrames: [String] = []
    private(set) var disconnectCount = 0

    func installReceiver(
        _ receiver: @escaping @Sendable (String) -> Void
    ) throws {
        self.receiver = receiver
    }

    func send(_ encodedFrame: String) throws {
        sentFrames.append(encodedFrame)
    }

    func disconnect() {
        receiver = nil
        disconnectCount += 1
    }

    func receive(_ encodedFrame: String) {
        receiver?(encodedFrame)
    }
}
