import UIKit

/// Shows and hides the keyboard from the frame's TEXT_INPUT_ACTIVE flag,
/// forwards text and hardware keys, and reports keyboard overlap.
@MainActor
final class TextInputController: KeyInputProxyDelegate {
    private let bridge: RustBridge
    private weak var canvas: UIView?
    private let proxy = KeyInputProxy(frame: CGRect(x: 0, y: 0, width: 1, height: 1))
    private var kind = ONECAD_IOS_TEXT_GENERAL.rawValue
    /// Rust still has a focused field but the user dismissed the keyboard;
    /// it is not re-shown until Rust ends and restarts text input.
    private var dismissedByUser = false
    /// Presses whose began was handled, mapped to whether it reached Rust.
    /// One UIPress arrives twice (proxy, then bubbled to the canvas) and its
    /// end must follow its begin's decision, so identity is the key.
    private var livePresses: [ObjectIdentifier: Bool] = [:]
    var onInput: (() -> Void)?
    var onCapabilities: ((UInt32) -> Void)?
    /// Keyboard overlap in canvas points, for the safe-area/resize update.
    var onKeyboardOverlap: ((CGFloat) -> Void)?

    init(bridge: RustBridge, canvas: UIView) {
        self.bridge = bridge
        self.canvas = canvas
        proxy.proxyDelegate = self
        canvas.addSubview(proxy)
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(keyboardWillChange(_:)),
                           name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        center.addObserver(self, selector: #selector(keyboardWillHide(_:)),
                           name: UIResponder.keyboardWillHideNotification, object: nil)
    }

    var isEditing: Bool { proxy.isFirstResponder }

    func apply(_ result: onecad_ios_frame_result) {
        let active = result.flags & ONECAD_IOS_FRAME_TEXT_INPUT_ACTIVE != 0
        guard active else {
            dismissedByUser = false
            if proxy.isFirstResponder {
                proxy.resign()
                canvas?.becomeFirstResponder()
            }
            return
        }
        placeCaret(x: result.text_cursor_x, y: result.text_cursor_y, height: result.text_cursor_h)
        let kindChanged = result.text_input_kind != kind
        kind = result.text_input_kind
        proxy.keyboardType = kind == ONECAD_IOS_TEXT_NUMERIC_EXPRESSION.rawValue
            ? .numbersAndPunctuation : .default
        if proxy.isFirstResponder {
            if kindChanged { proxy.reloadInputViews() }
        } else if !dismissedByUser {
            _ = proxy.becomeFirstResponder()
        }
    }

    private func placeCaret(x: Float, y: Float, height: Float) {
        let caretHeight = max(CGFloat(height), 1)
        proxy.frame = CGRect(x: CGFloat(x), y: CGFloat(y), width: 1, height: caretHeight)
        proxy.caretRect = CGRect(x: 0, y: 0, width: 2, height: caretHeight)
    }

    // MARK: Keyboard frame

    @objc private func keyboardWillChange(_ note: Notification) {
        guard let canvas, let window = canvas.window,
              let screenFrame = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect
        else { return }
        let local = canvas.convert(screenFrame, from: window.screen.coordinateSpace)
        let overlap = canvas.bounds.intersection(local)
        report(overlap: overlap.isNull ? 0 : overlap.height)
    }

    @objc private func keyboardWillHide(_ note: Notification) {
        report(overlap: 0)
    }

    private func report(overlap: CGFloat) {
        bridge.setKeyboardOverlap(Float(overlap))
        onKeyboardOverlap?(overlap)
        onInput?()
    }

    // MARK: Hardware keys

    func handlePresses(_ presses: Set<UIPress>, pressed: Bool) {
        var sawKeyboard = false
        for press in presses {
            guard let key = press.key else { continue }
            sawKeyboard = true
            guard let submit = pressDecision(press, key: key, pressed: pressed), submit else { continue }
            var event = onecad_ios_key_event()
            event.hid_usage = UInt32(truncatingIfNeeded: key.keyCode.rawValue)
            event.modifiers = Self.modifiers(key.modifierFlags)
            event.pressed = pressed ? 1 : 0
            // UIKit does not surface auto-repeat through presses.
            event.`repeat` = 0
            bridge.submitKey(event)
        }
        if sawKeyboard { onCapabilities?(ONECAD_IOS_CAP_HARDWARE_KEYBOARD) }
        onInput?()
    }

    /// Nil for a duplicate delivery (bubbled press, or an end whose begin was
    /// never seen); otherwise whether this edge goes to `key_submit`.
    private func pressDecision(_ press: UIPress, key: UIKey, pressed: Bool) -> Bool? {
        let id = ObjectIdentifier(press)
        if !pressed { return livePresses.removeValue(forKey: id) }
        guard livePresses[id] == nil else { return nil }
        let submit = !(proxy.isFirstResponder && Self.isTextChannelKey(key))
        livePresses[id] = submit
        return submit
    }

    /// One-channel rule (docs/ios/BUILD.md, "Keyboard channels"): while the
    /// proxy is first responder, UIKit's text system already turns these keys
    /// into insertText("\n" / "\t") and deleteBackward, with system key repeat,
    /// exactly as the software keyboard does. Sending them as key events too
    /// would apply each twice. ⌘/⌃ chords produce no text, so they stay keys.
    static func isTextChannelKey(_ key: UIKey) -> Bool {
        guard key.modifierFlags.isDisjoint(with: [.command, .control]) else { return false }
        switch key.keyCode {
        case .keyboardReturnOrEnter, .keypadEnter, .keyboardDeleteOrBackspace, .keyboardTab: return true
        default: return false
        }
    }

    /// Scene inactive: ends whose begins were seen may never arrive.
    func resetPresses() {
        livePresses.removeAll()
    }

    static func modifiers(_ flags: UIKeyModifierFlags) -> UInt32 {
        var mask: UInt32 = 0
        if flags.contains(.shift) { mask |= ONECAD_IOS_MOD_SHIFT }
        if flags.contains(.control) { mask |= ONECAD_IOS_MOD_CONTROL }
        if flags.contains(.alternate) { mask |= ONECAD_IOS_MOD_OPTION }
        if flags.contains(.command) { mask |= ONECAD_IOS_MOD_COMMAND }
        return mask
    }

    // MARK: KeyInputProxyDelegate

    func proxyInsert(_ text: String) {
        bridge.insertText(text)
        onInput?()
    }

    func proxyDeleteBackward() {
        bridge.deleteBackward()
        onInput?()
    }

    func proxySetMarked(_ text: String?) {
        bridge.setMarkedText(text)
        onInput?()
    }

    func proxyPresses(_ presses: Set<UIPress>, pressed: Bool) {
        handlePresses(presses, pressed: pressed)
    }

    func proxyDidResignWithoutRequest() {
        // Host UI (save-name alert, picker) took focus: not a user dismissal,
        // and the canvas must not steal it back. The coordinator's completion
        // restores canvas focus; the next frame re-shows the keyboard if
        // Rust's field is still focused.
        guard !isHostUIPresented else { return }
        dismissedByUser = true
        // Deferred: UIKit is still inside the proxy's resign transition.
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                guard let self, !self.isHostUIPresented else { return }
                _ = self.canvas?.becomeFirstResponder()
            }
        }
    }

    private var isHostUIPresented: Bool {
        canvas?.window?.rootViewController?.presentedViewController != nil
    }
}
