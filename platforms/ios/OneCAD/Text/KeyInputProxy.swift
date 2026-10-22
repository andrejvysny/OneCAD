import UIKit

@MainActor
protocol KeyInputProxyDelegate: AnyObject {
    func proxyInsert(_ text: String)
    func proxyDeleteBackward()
    func proxySetMarked(_ text: String?)
    func proxyPresses(_ presses: Set<UIPress>, pressed: Bool)
    func proxyDidResignWithoutRequest()
}

/// Invisible first responder that brings up the software keyboard for the
/// focused egui field. Rust owns the field contents; the proxy only mirrors
/// IME marked text so UIKit's text system can compose (UITextInput subset).
/// It stays 1x1 point at the caret so Scribble cannot claim the canvas.
final class KeyInputProxy: UIView, UIKeyInput, UITextInput {
    static let accessibilityID = "onecad.keyInputProxy"

    weak var proxyDelegate: KeyInputProxyDelegate?
    private(set) var markedText = ""
    /// Selection inside the marked text, as given by the IME.
    private(set) var markedSelection = NSRange(location: 0, length: 0)
    weak var inputDelegate: UITextInputDelegate?
    lazy var tokenizer: UITextInputTokenizer = UITextInputStringTokenizer(textInput: self)
    private var resignRequested = false
    /// Caret in this view's coordinates, used for IME candidate placement.
    var caretRect = CGRect(x: 0, y: 0, width: 2, height: 17)

    // UITextInputTraits: expressions and names must not be autocorrected.
    var keyboardType: UIKeyboardType = .default
    var autocorrectionType: UITextAutocorrectionType = .no
    var autocapitalizationType: UITextAutocapitalizationType = .none
    var spellCheckingType: UITextSpellCheckingType = .no
    var smartQuotesType: UITextSmartQuotesType = .no
    var smartDashesType: UITextSmartDashesType = .no
    var smartInsertDeleteType: UITextSmartInsertDeleteType = .no
    var returnKeyType: UIReturnKeyType = .done

    override init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        backgroundColor = .clear
        accessibilityIdentifier = Self.accessibilityID
        isAccessibilityElement = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override var canBecomeFirstResponder: Bool { true }

    /// Resigns on Rust's request; any other resignation (keyboard dismiss key,
    /// focus moving elsewhere) is reported so the host does not re-show it.
    func resign() {
        resignRequested = true
        _ = resignFirstResponder()
        resignRequested = false
    }

    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        if resigned {
            endComposition()
            if !resignRequested { proxyDelegate?.proxyDidResignWithoutRequest() }
        }
        return resigned
    }

    private func endComposition() {
        guard !markedText.isEmpty else { return }
        markedText = ""
        proxyDelegate?.proxySetMarked(nil)
    }

    // MARK: UIKeyInput

    var hasText: Bool { true }

    func insertText(_ text: String) {
        if !markedText.isEmpty {
            markedText = ""
            proxyDelegate?.proxySetMarked(nil)
        }
        proxyDelegate?.proxyInsert(text)
    }

    func deleteBackward() {
        proxyDelegate?.proxyDeleteBackward()
    }

    // MARK: Hardware keys (also forwarded while text input is active)

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

    // MARK: UITextInput — marked text

    var markedTextRange: UITextRange? {
        markedText.isEmpty ? nil : IndexRange(0, markedLength)
    }

    var markedTextStyle: [NSAttributedString.Key: Any]?

    func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        self.markedText = markedText ?? ""
        markedSelection = selectedRange
        proxyDelegate?.proxySetMarked(self.markedText.isEmpty ? nil : self.markedText)
    }

    /// Commits the composition as ordinary text.
    func unmarkText() {
        let committed = markedText
        guard !committed.isEmpty else { return }
        markedText = ""
        proxyDelegate?.proxySetMarked(nil)
        proxyDelegate?.proxyInsert(committed)
    }

    var markedLength: Int { (markedText as NSString).length }
}
