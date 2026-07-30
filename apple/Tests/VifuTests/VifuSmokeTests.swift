import XCTest
@testable import Vifu

final class VifuSmokeTests: XCTestCase {
    func testEmbeddedRuntimeLinksAndExportsState() throws {
        let runtime = try VifuEmbeddedRuntime(projectId: "swift-package-smoke")
        let snapshot = try runtime.exportSnapshot()

        XCTAssertFalse(snapshot.isEmpty)
    }
}
