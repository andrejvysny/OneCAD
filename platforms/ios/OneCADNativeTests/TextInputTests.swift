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

    final class InputDelegateRecorder: NSObject, UITextInputDelegate {
        var calls: [String] = []
        func textWillChange(_ textInput: (any UITextInput)?) { calls.append("textWill") }
        func textDidChange(_ textInput: (any UITextInput)?) { calls.append("textDid") }
        func selectionWillChange(_ textInput: (any UITextInput)?) { calls.append("selectionWill") }
        func selectionDidChange(_ textInput: (any UITextInput)?) { calls.append("selectionDid") }
        @available(iOS 18.4, *)
        func conversationContext(_ context: UIConversationContext?, didChange textInput: (any UITextInput)?) {}
    }

    private func assertExternalNotifications(_ proxy: KeyInputProxy) {
        let delegate = InputDelegateRecorder()
        proxy.inputDelegate = delegate
        let initial = snapshot("2mm")
        proxy.reconcile(initial)
        delegate.calls.removeAll()
        proxy.reconcile(initial)
        XCTAssertEqual(delegate.calls, [], "unchanged frames must not reset UIKit's text state")
        var next = initial
        next.revision = 2
        next.sequence = 1
        next.fieldRect.origin.x += 10
        next.caretRect.origin.x += 10
        proxy.reconcile(next)
        XCTAssertEqual(delegate.calls, [], "metadata and layout do not change text or selection")
        XCTAssertEqual(proxy.frame, next.fieldRect)
        XCTAssertEqual(proxy.session?.revision, 2)
        next.selection = NSRange(location: 0, length: 1)
        proxy.reconcile(next)
        XCTAssertEqual(delegate.calls, ["selectionWill", "selectionDid"])
        delegate.calls.removeAll()
        next.marked = NSRange(location: 0, length: 1)
        proxy.reconcile(next)
        XCTAssertEqual(delegate.calls, ["selectionWill", "selectionDid"])
        for change in 0..<3 {
            delegate.calls.removeAll()
            if change == 0 { next.text = "3mm" }
            if change == 1 { next.generation += 1 }
            if change == 2 { next.fieldID += 1 }
            proxy.reconcile(next)
            XCTAssertEqual(delegate.calls, ["textWill", "selectionWill", "selectionDid", "textDid"])
        }
    }

    private func assertRequestedResignNotifies() {
        let proxy = KeyInputProxy(frame: .zero)
        let transport = Recorder()
        proxy.proxyDelegate = transport
        proxy.reconcile(snapshot("幅", marked: NSRange(location: 0, length: 1)))
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        window.addSubview(proxy)
        XCTAssertTrue(proxy.becomeFirstResponder())
        let delegate = InputDelegateRecorder()
        proxy.inputDelegate = delegate
        transport.reply = snapshot("幅", revision: 2, sequence: 1)
        proxy.resign()
        XCTAssertEqual(transport.operations.last?.kind, ONECAD_IOS_TEXT_CANCEL.rawValue)
        XCTAssertNil(proxy.markedTextRange)
        XCTAssertEqual(delegate.calls, ["selectionWill", "selectionDid"],
                       "app-requested composition cancellation is external to UIKit")
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

    private func assertGrapheme(_ proxy: KeyInputProxy, text: String, start: Int, end: Int) throws {
        let forward = UITextDirection(rawValue: UITextStorageDirection.forward.rawValue)
        let backward = UITextDirection(rawValue: UITextStorageDirection.backward.rawValue)
        for (direction, from, to) in [
            (forward, start, end), (backward, end, start),
            (UITextDirection(rawValue: UITextLayoutDirection.right.rawValue), start, end),
            (UITextDirection(rawValue: UITextLayoutDirection.left.rawValue), end, start),
        ] {
            let position = try XCTUnwrap(proxy.position(from: proxy.beginningOfDocument, offset: from))
            let range = try XCTUnwrap(proxy.tokenizer.rangeEnclosingPosition(position, with: .character, inDirection: direction))
            XCTAssertEqual(proxy.text(in: range), text)
            XCTAssertTrue(proxy.tokenizer.isPosition(position, atBoundary: .character, inDirection: direction))
            let boundary = try XCTUnwrap(proxy.tokenizer.position(from: position, toBoundary: .character, inDirection: direction))
            XCTAssertEqual(proxy.offset(from: proxy.beginningOfDocument, to: boundary), to)
        }
    }

    func testFullDraftMiddleExpressionReplacementAndSelection() throws {
        let proxy = KeyInputProxy(frame: .zero)
        let transport = Recorder()
        proxy.proxyDelegate = transport
        proxy.reconcile(snapshot("=幅😀 + 2mm + 90°"))
        let forward = UITextDirection(rawValue: UITextStorageDirection.forward.rawValue)
        let wordPosition = try XCTUnwrap(proxy.position(from: proxy.beginningOfDocument, offset: 8))
        let word = try XCTUnwrap(proxy.tokenizer.rangeEnclosingPosition(wordPosition, with: .word, inDirection: forward))
        XCTAssertEqual(proxy.text(in: word), "2mm")
        try assertGrapheme(proxy, text: "😀", start: 2, end: 4)
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
        let markedWord = try XCTUnwrap(proxy.tokenizer.rangeEnclosingPosition(
            proxy.beginningOfDocument, with: .word,
            inDirection: UITextDirection(rawValue: UITextStorageDirection.forward.rawValue)))
        XCTAssertEqual(proxy.text(in: markedWord), "幅")
        try assertGrapheme(proxy, text: "😀", start: 1, end: 3)
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

    func testStalePositionsSurrogatesAndOverflowCannotEditNewFocus() throws {
        let proxy = KeyInputProxy(frame: .zero)
        let transport = Recorder()
        proxy.proxyDelegate = transport
        proxy.reconcile(snapshot("😀mm"))
        let stale = proxy.selectedTextRange!
        XCTAssertNil(proxy.position(from: proxy.beginningOfDocument, offset: 1))
        XCTAssertNil(proxy.position(from: proxy.endOfDocument, offset: Int.max))
        XCTAssertNil(proxy.position(from: proxy.endOfDocument, in: .left, offset: Int.min))
        proxy.reconcile(snapshot("90°", generation: 2))
        let forward = UITextDirection(rawValue: UITextStorageDirection.forward.rawValue)
        let currentWord = try XCTUnwrap(proxy.tokenizer.rangeEnclosingPosition(
            proxy.beginningOfDocument, with: .word, inDirection: forward))
        XCTAssertEqual(proxy.text(in: currentWord), "90")
        XCTAssertNil(proxy.tokenizer.rangeEnclosingPosition(stale.start, with: .word, inDirection: forward))
        proxy.replace(stale, withText: "99")
        proxy.selectedTextRange = stale
        XCTAssertNil(proxy.text(in: stale))
        XCTAssertEqual(transport.operations.count, 0)
        XCTAssertEqual(proxy.session?.text, "90°")
    }

    func testDeleteUsesSelectionAndPrecedingGrapheme() throws {
        let proxy = KeyInputProxy(frame: .zero)
        let transport = Recorder()
        proxy.proxyDelegate = transport
        let value = "e\u{301}👩‍👧mm"
        let beforeUnits = value.utf16.count - 2
        proxy.reconcile(snapshot(value, selection: NSRange(location: beforeUnits, length: 0)))
        try assertGrapheme(proxy, text: "e\u{301}", start: 0, end: 2)
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
        assertExternalNotifications(proxy)
        let transport = Recorder()
        proxy.proxyDelegate = transport
        proxy.reconcile(snapshot("2mm"))
        let inputDelegate = InputDelegateRecorder()
        proxy.inputDelegate = inputDelegate
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
        transport.reply = snapshot("2mm4", revision: 4, sequence: 3)
        proxy.insertText("4")
        XCTAssertEqual(proxy.session?.text, "2mm4")
        XCTAssertEqual(inputDelegate.calls, [], "UIKit-initiated ACKs must not notify UIKit again")
        assertRequestedResignNotifies()
    }

    func testAccessibleTargetMatchesFocusedFieldAndExactDraft() {
        let proxy = KeyInputProxy(frame: .zero)
        proxy.reconcile(snapshot("=width * 2mm"))
        XCTAssertTrue(proxy.isAccessibilityElement)
        let editableField: any UITextInput = proxy
        let draft = editableField.textRange(from: editableField.beginningOfDocument,
                                            to: editableField.endOfDocument)
        XCTAssertEqual(draft.flatMap(editableField.text(in:)), "=width * 2mm")
        XCTAssertTrue(proxy.isEditable)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        window.addSubview(proxy)
        XCTAssertTrue(proxy.becomeFirstResponder())
        XCTAssertTrue(proxy.isFirstResponder)
        XCTAssertTrue(proxy.isAccessibilityElement)
        proxy.resign()
        XCTAssertFalse(proxy.isFirstResponder)
        XCTAssertTrue(proxy.isAccessibilityElement, "the acknowledged field remains active until Rust ends its session")
        XCTAssertEqual(proxy.frame, CGRect(x: 20, y: 40, width: 150, height: 32))
        XCTAssertEqual(proxy.accessibilityLabel, "Length mm")
        XCTAssertEqual(proxy.accessibilityValue, "=width * 2mm")
        XCTAssertEqual(proxy.caretRect, CGRect(x: 50, y: 5, width: 2, height: 20))
        proxy.reconcile(nil)
        XCTAssertFalse(proxy.isEditable)
        XCTAssertFalse(proxy.isAccessibilityElement)
        XCTAssertFalse(proxy.canBecomeFirstResponder)
        weak var released: KeyInputProxy?
        autoreleasepool {
            let transient = KeyInputProxy(frame: .zero)
            transient.reconcile(snapshot("2mm"))
            released = transient
            _ = transient.tokenizer
        }
        XCTAssertNil(released, "the proxy must not own a tokenizer that retains it")
    }
}
