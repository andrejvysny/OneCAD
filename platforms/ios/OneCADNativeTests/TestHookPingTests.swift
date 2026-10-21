import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Hosted in the app: drives the process's single `onecad_ios_app` through the
/// acceptance-only C hooks (onecad_ios_test.h). Built only in Acceptance,
/// where the Rust archive carries the `test-hooks` feature.
@MainActor
final class TestHookPingTests: XCTestCase {
    func testPingThroughTestCommand() throws {
        let bridge = try XCTUnwrap(AppDelegate.shared?.bridge, "hosted app has no running Rust bridge")
        let app = try XCTUnwrap(bridge.app)
        var response: UnsafeMutablePointer<CChar>?
        let status = onecad_ios_test_command(app, #"{"cmd":"ping"}"#, &response)
        defer { onecad_ios_test_string_free(response) }
        XCTAssertEqual(status, ONECAD_IOS_OK, bridge.lastError)
        let json = try XCTUnwrap(response.map { String(cString: $0) })
        let object = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
        XCTAssertEqual(object?["ok"] as? Bool, true, json)
    }
}
#endif
