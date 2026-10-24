import UIKit
import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
@MainActor
final class AccessibilityBridgeTests: XCTestCase {
    func testUnchangedTreeReusesElementsWhileNativeFocusAndTextChangesRefresh() async throws {
        _ = try await SemanticHooks.attach(self)
        let bridge = try XCTUnwrap(AppDelegate.shared?.bridge)
        let container = UIView(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
        let accessibility = AccessibilityBridge(container: container, bridge: bridge)
        var nodes = [node(1, role: ONECAD_IOS_ROLE_TEXT_FIELD.rawValue, value: "12"),
                     node(2, role: ONECAD_IOS_ROLE_BUTTON.rawValue, value: "")]
        accessibility.applySnapshot(nodes: nodes, generation: 1, container: container)
        let first = try XCTUnwrap(container.accessibilityElements?.first as? CanvasAccessibilityElement)
        accessibility.applySnapshot(nodes: nodes, generation: 1, container: container)
        XCTAssertEqual(accessibility.rebuildCount, 1)
        XCTAssertTrue(container.accessibilityElements?.first as? CanvasAccessibilityElement === first)
        let proxy = UIView()
        proxy.isAccessibilityElement = true
        proxy.accessibilityValue = "12"
        accessibility.focusedTextProxy = proxy
        accessibility.focusedTextFieldID = 1
        accessibility.applySnapshot(nodes: nodes, generation: 1, container: container)
        XCTAssertEqual(accessibility.rebuildCount, 2)
        XCTAssertEqual(container.accessibilityElements?.count, 2)
        XCTAssertTrue(container.accessibilityElements?.last as? UIView === proxy)
        nodes[0] = node(1, role: ONECAD_IOS_ROLE_TEXT_FIELD.rawValue, value: "34")
        accessibility.applySnapshot(nodes: nodes, generation: 2, container: container)
        accessibility.focusedTextProxy = nil
        accessibility.focusedTextFieldID = nil
        accessibility.applySnapshot(nodes: nodes, generation: 2, container: container)
        let restored = try XCTUnwrap(container.accessibilityElements?.first as? CanvasAccessibilityElement)
        XCTAssertEqual(restored.nodeID, 1)
        XCTAssertEqual(restored.accessibilityValue, "34")
    }

    func testUnchangedFrameSkipsRustSnapshotButNativeProxyTransitionRefreshes() async throws {
        _ = try await SemanticHooks.attach(self)
        let bridge = try XCTUnwrap(AppDelegate.shared?.bridge)
        let container = UIView()
        let accessibility = AccessibilityBridge(container: container, bridge: bridge)
        accessibility.rebuild(changed: true)
        XCTAssertEqual(accessibility.snapshotReadCount, 1)
        accessibility.rebuild(changed: false)
        XCTAssertEqual(accessibility.snapshotReadCount, 1)
        let proxy = UIView()
        proxy.isAccessibilityElement = true
        accessibility.focusedTextProxy = proxy
        accessibility.focusedTextFieldID = UInt64.max
        accessibility.rebuild(changed: false)
        XCTAssertEqual(accessibility.snapshotReadCount, 2)
        XCTAssertTrue(container.accessibilityElements?.last as? UIView === proxy)
        accessibility.focusedTextProxy = nil
        accessibility.focusedTextFieldID = nil
        accessibility.rebuild(changed: false)
        XCTAssertEqual(accessibility.snapshotReadCount, 3)
    }

    private func node(_ id: UInt64, role: UInt32, value: String) -> AccessibilityNode {
        AccessibilityNode(id: id, role: role, flags: 0,
            frame: CGRect(x: 0, y: 0, width: 80, height: 25),
            identifier: "test.\(id)", label: "Field \(id)", value: value)
    }
}
#endif
