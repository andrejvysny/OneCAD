import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Hosted in the app: drives the process's single `onecad_ios_app` through the
/// acceptance-only C hooks (onecad_ios_test.h). Built only in Acceptance,
/// where the Rust archive carries the `test-hooks` feature.
@MainActor
final class TestHookPingTests: XCTestCase {
    func testPingThroughTestCommand() async throws {
        let bridge = try await runningBridge()
        let app = try XCTUnwrap(bridge.app)
        var response: UnsafeMutablePointer<CChar>?
        let status = onecad_ios_test_command(app, #"{"cmd":"ping"}"#, &response)
        defer { onecad_ios_test_string_free(response) }
        XCTAssertEqual(status, ONECAD_IOS_OK, bridge.lastError)
        let json = try XCTUnwrap(response.map { String(cString: $0) })
        let object = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
        XCTAssertEqual(object?["ok"] as? Bool, true, json)
    }

    /// The bridge is created when the scene connects, which can be after the
    /// hosted test starts; wait for it on the main run loop instead of racing.
    private func runningBridge(timeout: TimeInterval = 20) async throws -> RustBridge {
        if let bridge = AppDelegate.shared?.bridge { return bridge }
        let ready = expectation(description: "Rust bridge running")
        // Fulfilled on every tick once ready, until invalidated below.
        ready.assertForOverFulfill = false
        let poll = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { _ in
            MainActor.assumeIsolated {
                if AppDelegate.shared?.bridge != nil { ready.fulfill() }
            }
        }
        await fulfillment(of: [ready], timeout: timeout)
        poll.invalidate()
        return try XCTUnwrap(AppDelegate.shared?.bridge, "hosted app has no running Rust bridge")
    }
}
#else
/// Makes a non-Acceptance run visibly skip instead of passing with 0 tests.
final class TestHookPingTests: XCTestCase {
    func testPingThroughTestCommand() throws {
        throw XCTSkip("native test hooks exist only in the Acceptance configuration")
    }
}
#endif
