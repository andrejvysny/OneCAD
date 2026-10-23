import UIKit
import XCTest
@testable import OneCAD

/// Exercises the production UITextInput object. The recorder supplies explicit transport ACKs;
/// Rust's independently tested adapter owns operation semantics and stale-revision rejection.
@MainActor
final class TextInputTests: XCTestCase {
    final class Recorder: KeyInputProxyDelegate {
        var operations: [NativeTextOperation] = []
        var reply: NativeTextSnapshot?
        func proxyApply(_ operation: NativeTextOperation) -> NativeTextSnapshot? {
            operations.append(operation)
            return reply
        }
        func proxyPresses(_ presses: Set<UIPress>, pressed: Bool) {}
        func proxyDidResignWithoutRequest() {}
    }

    private func snapshot(_ text: String, generation: UInt64 = 1, revision: UInt64 = 1,
                          sequence: UInt64 = 0, selection: NSRange? = nil,
                          marked: NSRange? = nil) -> NativeTextSnapshot {
        NativeTextSnapshot(generation: generation, revision: revision, sequence: sequence, fieldID: 15,
                           text: text, selection: selection ?? NSRange(location: text.utf16.count, length: 0),
                           marked: marked, label: "Length mm",
                           fieldRect: CGRect(x: 20, y: 40, width: 150, height: 32),
                           caretRect: CGRect(x: 70, y: 45, width: 2, height: 20))
    }

    func testFullDraftMiddleExpressionReplacementAndSelection() throws {
        let proxy = KeyInputProxy(frame: .zero)
        let transport = Recorder()
        proxy.proxyDelegate = transport
        proxy.reconcile(snapshot("=幅😀 + 2mm + 90°"))
        let range = IndexRange(7, 10, generation: 1)
        XCTAssertEqual(proxy.text(in: range), "2mm")
        transport.reply = snapshot("=幅😀 + 3in + 90°", revision: 2, sequence: 1,
                                   selection: NSRange(location: 10, length: 0))
        proxy.replace(range, withText: "3in")
        let edit = try XCTUnwrap(transport.operations.last)
        XCTAssertEqual(edit.range, NSRange(location: 7, length: 3))
        XCTAssertEqual(edit.text, "3in")
        XCTAssertEqual(proxy.session?.text, "=幅😀 + 3in + 90°")
        transport.reply = snapshot("=幅😀 + 3in + 90°", revision: 3, sequence: 2,
                                   selection: NSRange(location: 0, length: 1))
        proxy.selectedTextRange = IndexRange(0, 1, generation: 1)
        XCTAssertEqual(transport.operations.last?.kind, ONECAD_IOS_TEXT_SELECT.rawValue)
        XCTAssertEqual(proxy.text(in: try XCTUnwrap(proxy.selectedTextRange)), "=")
    }

    func testMarkedReplacementUnmarkAndFollowingGenuineInsertion() throws {
        let proxy = KeyInputProxy(frame: .zero)
        let transport = Recorder()
        proxy.proxyDelegate = transport
        proxy.reconcile(snapshot("a+2mm", selection: NSRange(location: 0, length: 1)))
        transport.reply = snapshot("幅😀+2mm", revision: 2, sequence: 1,
                                   selection: NSRange(location: 3, length: 0), marked: NSRange(location: 0, length: 3))
        proxy.setMarkedText("幅😀", selectedRange: NSRange(location: 3, length: 0))
        XCTAssertEqual(transport.operations.last?.selection, NSRange(location: 3, length: 0))
        XCTAssertEqual(proxy.text(in: try XCTUnwrap(proxy.markedTextRange)), "幅😀")
        transport.reply = snapshot("幅😀+2mm", revision: 3, sequence: 2,
                                   selection: NSRange(location: 3, length: 0))
        proxy.unmarkText()
        XCTAssertEqual(transport.operations.count, 2)
        XCTAssertNil(proxy.markedTextRange)
        transport.reply = snapshot("幅😀+2mm", revision: 4, sequence: 3,
                                   selection: NSRange(location: 3, length: 0))
        proxy.unmarkText()
        transport.reply = snapshot("幅😀幅+2mm", revision: 5, sequence: 4,
                                   selection: NSRange(location: 4, length: 0))
        proxy.insertText("幅")
        XCTAssertEqual(transport.operations.count, 4)
        XCTAssertEqual(transport.operations.last?.kind, ONECAD_IOS_TEXT_REPLACE.rawValue)
        XCTAssertEqual(transport.operations.last?.range, NSRange(location: 3, length: 0))
    }

    func testStalePositionsSurrogatesAndOverflowCannotEditNewFocus() {
        let proxy = KeyInputProxy(frame: .zero)
        let transport = Recorder()
        proxy.proxyDelegate = transport
        proxy.reconcile(snapshot("😀mm"))
        let stale = proxy.selectedTextRange!
        XCTAssertNil(proxy.position(from: proxy.beginningOfDocument, offset: 1))
        XCTAssertNil(proxy.position(from: proxy.endOfDocument, offset: Int.max))
        XCTAssertNil(proxy.position(from: proxy.endOfDocument, in: .left, offset: Int.min))
        proxy.reconcile(snapshot("90°", generation: 2))
        proxy.replace(stale, withText: "99")
        proxy.selectedTextRange = stale
        XCTAssertNil(proxy.text(in: stale))
        XCTAssertEqual(transport.operations.count, 0)
        XCTAssertEqual(proxy.session?.text, "90°")
    }

    func testDeleteUsesSelectionAndPrecedingGrapheme() {
        let proxy = KeyInputProxy(frame: .zero)
        let transport = Recorder()
        proxy.proxyDelegate = transport
        let value = "e\u{301}👩‍👧mm"
        let beforeUnits = value.utf16.count - 2
        proxy.reconcile(snapshot(value, selection: NSRange(location: beforeUnits, length: 0)))
        transport.reply = snapshot("e\u{301}mm", revision: 2, sequence: 1, selection: NSRange(location: 2, length: 0))
        proxy.deleteBackward()
        XCTAssertEqual(transport.operations.last?.range, NSRange(location: 2, length: beforeUnits - 2))
        proxy.reconcile(snapshot("20mm", revision: 3, sequence: 2, selection: NSRange(location: 0, length: 2)))
        transport.reply = snapshot("mm", revision: 4, sequence: 3, selection: NSRange(location: 0, length: 0))
        proxy.deleteBackward()
        XCTAssertEqual(transport.operations.last?.range, NSRange(location: 0, length: 2))
    }

    func testOnlyAcknowledgedRevisionAdvancesAndConfirmHasSingleChannel() {
        let proxy = KeyInputProxy(frame: .zero)
        let transport = Recorder()
        proxy.proxyDelegate = transport
        proxy.reconcile(snapshot("2mm"))
        transport.reply = snapshot("2mm") // rejected replace: fresh unchanged Rust snapshot
        proxy.insertText("x")
        XCTAssertEqual(proxy.session?.text, "2mm")
        XCTAssertEqual(proxy.session?.revision, 1)
        transport.reply = snapshot("2mm", revision: 2, sequence: 1)
        proxy.insertText("\n")
        XCTAssertEqual(transport.operations.count, 2)
        XCTAssertEqual(transport.operations.last?.kind, ONECAD_IOS_TEXT_CONFIRM.rawValue)
        XCTAssertEqual(transport.operations.last?.revision, 1)
        XCTAssertEqual(transport.operations.last?.sequence, 1)
        transport.reply = snapshot("2mm", revision: 3, sequence: 2)
        proxy.insertText("\t")
        XCTAssertEqual(transport.operations.last?.kind, ONECAD_IOS_TEXT_TAB.rawValue)
        XCTAssertEqual(transport.operations.last?.revision, 2)
    }

    func testAccessibleTargetMatchesFocusedFieldAndExactDraft() {
        let proxy = KeyInputProxy(frame: .zero)
        proxy.reconcile(snapshot("=width * 2mm"))
        XCTAssertTrue(proxy.isAccessibilityElement)
        let editableField: UITextField = proxy
        XCTAssertEqual(editableField.text, "=width * 2mm")
        XCTAssertEqual(proxy.frame, CGRect(x: 20, y: 40, width: 150, height: 32))
        XCTAssertEqual(proxy.accessibilityLabel, "Length mm")
        XCTAssertEqual(proxy.accessibilityValue, "=width * 2mm")
        XCTAssertEqual(proxy.caretRect, CGRect(x: 50, y: 5, width: 2, height: 20))
        proxy.reconcile(nil)
        XCTAssertFalse(proxy.isAccessibilityElement)
        XCTAssertFalse(proxy.canBecomeFirstResponder)
    }
}
