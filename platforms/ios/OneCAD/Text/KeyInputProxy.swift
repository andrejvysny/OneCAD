import UIKit

@MainActor
protocol KeyInputProxyDelegate: AnyObject {
    func proxyApply(_ operation: NativeTextOperation) -> NativeTextSnapshot?
    func proxyPresses(_ presses: Set<UIPress>, pressed: Bool)
    func proxyDidResignWithoutRequest()
}

/// Native editable accessibility target for the focused egui field. Rust owns its full draft.
/// Only this field's bounds accept text interactions; the canvas never becomes a Scribble target.
final class KeyInputProxy: UIView, UITextInput {
    static let accessibilityID = "onecad.keyInputProxy"
    weak var proxyDelegate: KeyInputProxyDelegate?
    private(set) var session: NativeTextSnapshot?
    private var resignRequested = false
    var caretRect = CGRect(x: 0, y: 0, width: 2, height: 17)
    weak var inputDelegate: UITextInputDelegate?
    var markedTextStyle: [NSAttributedString.Key: Any]?
    var keyboardType: UIKeyboardType = .default
    var autocorrectionType: UITextAutocorrectionType = .no
    var autocapitalizationType: UITextAutocapitalizationType = .none
    var spellCheckingType: UITextSpellCheckingType = .no
    var smartQuotesType: UITextSmartQuotesType = .no
    var smartDashesType: UITextSmartDashesType = .no
    var smartInsertDeleteType: UITextSmartInsertDeleteType = .no
    var returnKeyType: UIReturnKeyType = .done
    var isEditable: Bool { session != nil }
    var textInputView: UIView { self }
    override var isAccessibilityElement: Bool {
        get { session != nil }
        set { super.isAccessibilityElement = newValue }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        tintColor = .clear
        accessibilityIdentifier = Self.accessibilityID
        accessibilityTextualContext = .wordProcessing
        isAccessibilityElement = false
        isUserInteractionEnabled = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    override var canBecomeFirstResponder: Bool { session != nil }

    // The public tokenizer understands custom positions. Keep it outside
    // proxy ownership to avoid a reference cycle.
    var tokenizer: UITextInputTokenizer {
        DraftTokenizer(input: self)
    }

    func reconcile(_ next: NativeTextSnapshot?, notifyInputDelegate: Bool = true) {
        let focusChanged = session?.generation != next?.generation || session?.fieldID != next?.fieldID
        let textChanged = notifyInputDelegate && (focusChanged || session?.text != next?.text)
        let selectionChanged = notifyInputDelegate && (textChanged || session?.selection != next?.selection
            || session?.marked != next?.marked)
        if textChanged { inputDelegate?.textWillChange(self) }
        if selectionChanged { inputDelegate?.selectionWillChange(self) }
        session = next
        if let next {
            frame = next.fieldRect
            caretRect = next.caretRect.offsetBy(dx: -frame.minX, dy: -frame.minY)
        }
        if selectionChanged { inputDelegate?.selectionDidChange(self) }
        if textChanged { inputDelegate?.textDidChange(self) }
        accessibilityLabel = next?.label
        accessibilityValue = next?.text
        isUserInteractionEnabled = next != nil
    }

    func submit(_ kind: UInt32, range: NSRange = NSRange(location: 0, length: 0),
                text: String = "", selection: NSRange = NSRange(location: 0, length: 0),
                notifyInputDelegate: Bool = false) {
        guard let session, session.sequence < UInt64.max else { return }
        let operation = NativeTextOperation(generation: session.generation, revision: session.revision,
                                            sequence: session.sequence + 1, kind: kind, range: range,
                                            selection: selection, text: text)
        // UIKit already knows edits it initiated. Only external changes need
        // delegate notifications; repeating them can reset keyboard/IME state.
        reconcile(proxyDelegate?.proxyApply(operation), notifyInputDelegate: notifyInputDelegate)
    }

    func resign() {
        resignRequested = true
        _ = resignFirstResponder()
        resignRequested = false
    }

    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        if resigned {
            if session?.marked != nil {
                submit(ONECAD_IOS_TEXT_CANCEL.rawValue, notifyInputDelegate: resignRequested)
            }
            if !resignRequested { proxyDelegate?.proxyDidResignWithoutRequest() }
        }
        return resigned
    }

    var hasText: Bool { session?.text.isEmpty == false }

    func insertText(_ text: String) {
        if text == "\n" || text == "\r" { submit(ONECAD_IOS_TEXT_CONFIRM.rawValue); return }
        if text == "\t" { submit(ONECAD_IOS_TEXT_TAB.rawValue); return }
        guard let session else { return }
        submit(ONECAD_IOS_TEXT_REPLACE.rawValue, range: session.marked ?? session.selection, text: text)
    }

    func deleteBackward() {
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

    var markedTextRange: UITextRange? {
        guard let session, let range = session.marked else { return nil }
        return IndexRange(range.location, range.upperBound, generation: session.generation)
    }

    func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        let text = markedText ?? ""
        let local = NativeTextSnapshot(generation: 0, revision: 0, sequence: 0, fieldID: 0,
                                      text: text, selection: selectedRange, label: "", fieldRect: .zero, caretRect: .zero)
        guard local.valid(selectedRange) else { return }
        submit(ONECAD_IOS_TEXT_MARK.rawValue, text: text, selection: selectedRange)
    }

    func unmarkText() { submit(ONECAD_IOS_TEXT_UNMARK.rawValue) }
}
