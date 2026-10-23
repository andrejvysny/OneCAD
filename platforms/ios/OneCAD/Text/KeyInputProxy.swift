import UIKit

@MainActor
protocol KeyInputProxyDelegate: AnyObject {
    func proxyApply(_ operation: NativeTextOperation) -> NativeTextSnapshot?
    func proxyPresses(_ presses: Set<UIPress>, pressed: Bool)
    func proxyDidResignWithoutRequest()
}

/// Native editable accessibility target for the focused egui field. Rust owns its full draft.
/// Only this field's bounds accept text interactions; the canvas never becomes a Scribble target.
final class KeyInputProxy: UITextField {
    static let accessibilityID = "onecad.keyInputProxy"
    weak var proxyDelegate: KeyInputProxyDelegate?
    private(set) var session: NativeTextSnapshot?
    private var resignRequested = false
    var caretRect = CGRect(x: 0, y: 0, width: 2, height: 17)

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        textColor = .clear
        tintColor = .clear
        borderStyle = .none
        autocorrectionType = .no
        autocapitalizationType = .none
        spellCheckingType = .no
        smartQuotesType = .no
        smartDashesType = .no
        smartInsertDeleteType = .no
        returnKeyType = .done
        accessibilityIdentifier = Self.accessibilityID
        accessibilityTextualContext = .wordProcessing
        isAccessibilityElement = false
        isUserInteractionEnabled = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    override var canBecomeFirstResponder: Bool { session != nil }

    func reconcile(_ next: NativeTextSnapshot?) {
        inputDelegate?.textWillChange(self)
        inputDelegate?.selectionWillChange(self)
        session = next
        super.text = next?.text
        isUserInteractionEnabled = next != nil
        isAccessibilityElement = next != nil
        accessibilityLabel = next?.label
        accessibilityValue = next?.text
        if let next {
            frame = next.fieldRect
            caretRect = next.caretRect.offsetBy(dx: -frame.minX, dy: -frame.minY)
        }
        inputDelegate?.selectionDidChange(self)
        inputDelegate?.textDidChange(self)
    }

    func submit(_ kind: UInt32, range: NSRange = NSRange(location: 0, length: 0),
                text: String = "", selection: NSRange = NSRange(location: 0, length: 0)) {
        guard let session, session.sequence < UInt64.max else { return }
        let operation = NativeTextOperation(generation: session.generation, revision: session.revision,
                                            sequence: session.sequence + 1, kind: kind, range: range,
                                            selection: selection, text: text)
        reconcile(proxyDelegate?.proxyApply(operation))
    }

    func resign() {
        resignRequested = true
        _ = resignFirstResponder()
        resignRequested = false
    }

    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        if resigned {
            if session?.marked != nil { submit(ONECAD_IOS_TEXT_CANCEL.rawValue) }
            if !resignRequested { proxyDelegate?.proxyDidResignWithoutRequest() }
        }
        return resigned
    }

    override var hasText: Bool { session?.text.isEmpty == false }

    override func insertText(_ text: String) {
        if text == "\n" || text == "\r" { submit(ONECAD_IOS_TEXT_CONFIRM.rawValue); return }
        if text == "\t" { submit(ONECAD_IOS_TEXT_TAB.rawValue); return }
        guard let session else { return }
        submit(ONECAD_IOS_TEXT_REPLACE.rawValue, range: session.marked ?? session.selection, text: text)
    }

    override func deleteBackward() {
        guard let session else { return }
        var range = session.selection
        if range.length == 0 {
            guard range.location > 0 else { return }
            // NSString expands to the preceding composed character, including emoji/combining marks.
            range = (session.text as NSString).rangeOfComposedCharacterSequence(at: range.location - 1)
        }
        submit(ONECAD_IOS_TEXT_REPLACE.rawValue, range: range)
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        proxyDelegate?.proxyPresses(presses, pressed: true)
        super.pressesBegan(presses, with: event)
    }
    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        proxyDelegate?.proxyPresses(presses, pressed: false)
        super.pressesEnded(presses, with: event)
    }
    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        proxyDelegate?.proxyPresses(presses, pressed: false)
        super.pressesCancelled(presses, with: event)
    }

    override var markedTextRange: UITextRange? {
        guard let session, let range = session.marked else { return nil }
        return IndexRange(range.location, range.upperBound, generation: session.generation)
    }
    override var markedTextStyle: [NSAttributedString.Key: Any]? { get { super.markedTextStyle } set { super.markedTextStyle = newValue } }

    override func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        let text = markedText ?? ""
        let local = NativeTextSnapshot(generation: 0, revision: 0, sequence: 0, fieldID: 0,
                                      text: text, selection: selectedRange, label: "", fieldRect: .zero, caretRect: .zero)
        guard local.valid(selectedRange) else { return }
        submit(ONECAD_IOS_TEXT_MARK.rawValue, text: text, selection: selectedRange)
    }

    override func unmarkText() { submit(ONECAD_IOS_TEXT_UNMARK.rawValue) }
}
