// tauri-agent macOS native-input helper.
//
// Long-lived CLI. Reads one JSON request object per line from stdin and writes one JSON reply
// per line to stdout. Every diagnostic goes to stderr; stdout carries replies only.
//
//   request : {"id": 7, "verb": "click", "button": "left", "x": 100, "y": 200}
//   reply   : {"id": 7, "ok": true,  "result": {...}}
//           | {"id": 7, "ok": false, "code": "INVALID_ARGS", "message": "..."}
//
// Coordinates are global display points, top-left origin, y down.
// All events come from one CGEventSource(.hidSystemState) and are posted to .cghidEventTap.
// Modifier state is carried on EVERY posted event (mouse, scroll, key): WebKit reads shiftKey
// and friends off the event flags, not off global keyboard state.
//
// Every numeric argument is range-checked before it reaches a fixed-width type: a trapping
// conversion would kill the helper mid-drag with a button still held at the OS level.

import Cocoa
import Carbon.HIToolbox
import ApplicationServices
import Foundation

let helperVersion = "1.3.0"
let helperProtocol = 4

/// Longest sleep any single duration/interval/hold/dwell argument may ask for.
let maxSleepMs = 60_000.0
/// Widest interpolation any one motion verb may run.
let maxSteps = 2_000
/// Widest scroll a single verb may post, in lines.
let maxScrollLines = 100.0

// Accessibility reads are bounded on four axes at once, because an AX read is synchronous IPC into
// another application and this helper answers one verb at a time (see the Accessibility section).
/// Nodes a walk may EMIT by default, and the ceiling `maxNodes` is clamped to.
let axDefaultMaxNodes = 500
let axMaxNodes = 5_000
/// Elements a walk may VISIT. `ax_find` emits only matches, so this is what bounds ITS cost.
let axMaxVisit = 4_000
/// Depth cap. An AX tree is not guaranteed acyclic, so this is a termination guarantee, not a budget.
let axMaxDepth = 64
/// A menu bar is big: Chrome's is 1,436 nodes and 307 KB of reply at this cap, in 362 ms.
let axMenuMaxNodes = 2_000
let axMenuMaxVisit = 3_000
/// A menu bar item wraps its items in an `AXMenu`, so each visible level of menu costs two.
let axMenuMaxDepth = 8
/// Wall-clock budget for one walk, measured at ~0.26 ms per node against a real application.
let axWalkBudgetMs = 2_500.0
/// Gap between `ax_point`'s two rect samples, and the tolerance between them — both mirrors of the
/// webview resolver's `rectStable` (geometry/mapping.ts), so the two ladders agree on "still".
let axSampleGapMs = 50.0
let axRectTolerance = 0.5
/// Per-message timeout on every application element this helper creates, in seconds.
let axMessagingTimeoutS: Float = 2.0
/// Longest string echoed back for a title or a value; an `AXTextArea`'s value can be megabytes.
let axMaxTextChars = 256
let axMaxActions = 32
let axMaxWindows = 64
/// A CGWindowID is a `UInt32`, so this is the widest `window` argument that could ever match one.
let axMaxWindowId = Int(UInt32.max)
/// An AX parent chain is not guaranteed to terminate; the climb to a containing window is capped.
let axMaxParentHops = 32

// MARK: - Errors

struct HelperError: Error {
    let code: String
    let message: String
    init(_ code: String, _ message: String) { self.code = code; self.message = message }
}

private func badArgs(_ message: String) -> HelperError { HelperError("INVALID_ARGS", message) }

// MARK: - JSON I/O

func writeJSON(_ obj: [String: Any]) {
    let data = (try? JSONSerialization.data(withJSONObject: obj, options: [])) ?? unserializable(obj["id"])
    var line = data
    line.append(0x0A)
    FileHandle.standardOutput.write(line)
}

/// A reply with no id is a reply the client can never settle, so the fallback carries it too.
private func unserializable(_ id: Any?) -> Data {
    let fallback: [String: Any] = ["id": (id as? NSNumber) ?? NSNull(), "ok": false,
                                   "code": "INTERNAL", "message": "unserializable reply"]
    return (try? JSONSerialization.data(withJSONObject: fallback, options: []))
        ?? Data(#"{"id":null,"ok":false,"code":"INTERNAL","message":"unserializable reply"}"#.utf8)
}

func writeErr(_ message: String) {
    FileHandle.standardError.write(Data(("tauri-agent-helper: " + message + "\n").utf8))
}

// MARK: - Argument helpers

private func num(_ r: [String: Any], _ k: String) throws -> Double {
    guard let v = r[k] as? NSNumber else { throw badArgs("missing or non-numeric '\(k)'") }
    guard v.doubleValue.isFinite else { throw badArgs("'\(k)' must be a finite number") }
    return v.doubleValue
}
private func optNum(_ r: [String: Any], _ k: String) throws -> Double? {
    r[k] == nil ? nil : try num(r, k)
}
private func point(_ r: [String: Any]) throws -> CGPoint { CGPoint(x: try num(r, "x"), y: try num(r, "y")) }
private func str(_ r: [String: Any], _ k: String) throws -> String {
    guard let v = r[k] as? String else { throw badArgs("missing or non-string '\(k)'") }
    return v
}

private func clamp(_ v: Double, _ lo: Double, _ hi: Double) -> Double { min(max(v, lo), hi) }

/// Durations are clamped, not rejected: a caller asking for a year of easing means milliseconds.
private func msArg(_ r: [String: Any], _ k: String, _ fallback: Double) throws -> Double {
    clamp(try optNum(r, k) ?? fallback, 0, maxSleepMs)
}

private func intArg(_ r: [String: Any], _ k: String, _ lo: Int, _ hi: Int) throws -> Int? {
    guard let v = try optNum(r, k) else { return nil }
    return Int(clamp(v.rounded(), Double(lo), Double(hi)))
}

private func int32Arg(_ v: Double, _ k: String) throws -> Int32 {
    guard let out = Int32(exactly: clamp(v, -maxScrollLines, maxScrollLines).rounded()) else {
        throw badArgs("'\(k)' is out of range")
    }
    return out
}

private func pidArg(_ r: [String: Any]) throws -> pid_t {
    guard let raw = try optNum(r, "pid"), let pid = pid_t(exactly: raw.rounded()), pid > 0 else {
        throw badArgs("'pid' must be a process id")
    }
    return pid
}

/// The only sleep in the helper. Non-finite and out-of-range values become no sleep at all.
private func sleepMs(_ ms: Double) {
    guard ms.isFinite, ms > 0 else { return }
    if let micros = UInt32(exactly: (clamp(ms, 0, maxSleepMs) * 1000.0).rounded()) { usleep(micros) }
}

// MARK: - Modifiers

enum Mods {
    static let all = ["Command", "Control", "Option", "Shift", "Fn"]

    static func canonical(_ raw: String) throws -> String {
        switch raw.lowercased() {
        case "command", "cmd": return "Command"
        case "control", "ctrl": return "Control"
        case "option", "alt": return "Option"
        case "shift": return "Shift"
        case "fn", "function": return "Fn"
        default: throw badArgs("unknown modifier '\(raw)' (Primary must be translated by the caller)")
        }
    }

    /// Non-throwing sibling of `canonical`, for callers deciding WHICH branch a name belongs to.
    static func canonicalOrNil(_ raw: String) -> String? {
        try? canonical(raw)
    }

    static func parse(_ r: [String: Any]) throws -> Set<String> {
        guard let raw = r["mods"] else { return [] }
        guard let list = raw as? [String] else { throw badArgs("'mods' must be an array of strings") }
        return Set(try list.map(canonical))
    }

    static func mask(_ name: String) -> CGEventFlags {
        switch name {
        case "Command": return .maskCommand
        case "Control": return .maskControl
        case "Option": return .maskAlternate
        case "Shift": return .maskShift
        case "Fn": return .maskSecondaryFn
        default: return []
        }
    }

    static func flags(_ names: Set<String>) -> CGEventFlags {
        names.reduce(into: CGEventFlags()) { $0.insert(mask($1)) }
    }

    static func keyCode(_ name: String) -> CGKeyCode {
        switch name {
        case "Command": return CGKeyCode(kVK_Command)
        case "Control": return CGKeyCode(kVK_Control)
        case "Option": return CGKeyCode(kVK_Option)
        case "Shift": return CGKeyCode(kVK_Shift)
        default: return CGKeyCode(kVK_Function)
        }
    }
}

// MARK: - Key names (US layout)

enum Keys {
    static let map: [String: CGKeyCode] = {
        let letters = [kVK_ANSI_A, kVK_ANSI_B, kVK_ANSI_C, kVK_ANSI_D, kVK_ANSI_E, kVK_ANSI_F,
                       kVK_ANSI_G, kVK_ANSI_H, kVK_ANSI_I, kVK_ANSI_J, kVK_ANSI_K, kVK_ANSI_L,
                       kVK_ANSI_M, kVK_ANSI_N, kVK_ANSI_O, kVK_ANSI_P, kVK_ANSI_Q, kVK_ANSI_R,
                       kVK_ANSI_S, kVK_ANSI_T, kVK_ANSI_U, kVK_ANSI_V, kVK_ANSI_W, kVK_ANSI_X,
                       kVK_ANSI_Y, kVK_ANSI_Z]
        let digits = [kVK_ANSI_0, kVK_ANSI_1, kVK_ANSI_2, kVK_ANSI_3, kVK_ANSI_4,
                      kVK_ANSI_5, kVK_ANSI_6, kVK_ANSI_7, kVK_ANSI_8, kVK_ANSI_9]
        let fkeys = [kVK_F1, kVK_F2, kVK_F3, kVK_F4, kVK_F5, kVK_F6,
                     kVK_F7, kVK_F8, kVK_F9, kVK_F10, kVK_F11, kVK_F12]
        var m: [String: CGKeyCode] = [
            "enter": CGKeyCode(kVK_Return), "return": CGKeyCode(kVK_Return),
            "escape": CGKeyCode(kVK_Escape), "esc": CGKeyCode(kVK_Escape),
            "tab": CGKeyCode(kVK_Tab), "space": CGKeyCode(kVK_Space),
            "backspace": CGKeyCode(kVK_Delete), "delete": CGKeyCode(kVK_ForwardDelete),
            "arrowup": CGKeyCode(kVK_UpArrow), "arrowdown": CGKeyCode(kVK_DownArrow),
            "arrowleft": CGKeyCode(kVK_LeftArrow), "arrowright": CGKeyCode(kVK_RightArrow),
            "home": CGKeyCode(kVK_Home), "end": CGKeyCode(kVK_End),
            "pageup": CGKeyCode(kVK_PageUp), "pagedown": CGKeyCode(kVK_PageDown),
            "-": CGKeyCode(kVK_ANSI_Minus), "=": CGKeyCode(kVK_ANSI_Equal),
            "[": CGKeyCode(kVK_ANSI_LeftBracket), "]": CGKeyCode(kVK_ANSI_RightBracket),
            "\\": CGKeyCode(kVK_ANSI_Backslash), ";": CGKeyCode(kVK_ANSI_Semicolon),
            "'": CGKeyCode(kVK_ANSI_Quote), ",": CGKeyCode(kVK_ANSI_Comma),
            ".": CGKeyCode(kVK_ANSI_Period), "/": CGKeyCode(kVK_ANSI_Slash),
            "`": CGKeyCode(kVK_ANSI_Grave), "alt": Mods.keyCode("Option"),
        ]
        for (i, c) in letters.enumerated() { m[String(UnicodeScalar(UInt8(97 + i)))] = CGKeyCode(c) }
        for (i, c) in digits.enumerated() { m[String(i)] = CGKeyCode(c) }
        for (i, c) in fkeys.enumerated() { m["f\(i + 1)"] = CGKeyCode(c) }
        for name in Mods.all { m[name.lowercased()] = Mods.keyCode(name) }
        return m
    }()

    static func isModifier(_ key: String) -> Bool { (try? Mods.canonical(key)) != nil }

    static func code(_ key: String) throws -> CGKeyCode {
        guard let c = map[key.lowercased()] else { throw badArgs("unknown key '\(key)'") }
        return c
    }
}

// MARK: - Engine

final class Engine {
    static let shared = Engine()

    private let source = CGEventSource(stateID: .hidSystemState)
    private(set) var heldMods: Set<String> = []
    private(set) var heldButtons: [String] = []
    private var clickStates: [String: Int64] = [:]

    func cursor() -> CGPoint { CGEvent(source: nil)?.location ?? .zero }

    func requirePermission() throws {
        guard AXIsProcessTrusted() else {
            throw HelperError("NATIVE_INPUT_PERMISSION_DENIED",
                              "Accessibility is not granted to this process tree; CGEvent posts are dropped")
        }
    }

    /// Held buttons are tracked under this name, so every alias must collapse to one key —
    /// a "center" that never matched the "middle" it was pressed as could not be released.
    func canonicalButton(_ name: String) throws -> String {
        switch name.lowercased() {
        case "left": return "left"
        case "right": return "right"
        case "middle", "center", "other": return "middle"
        default: throw badArgs("unknown button '\(name)'")
        }
    }

    private func buttonSpec(_ name: String) throws -> (CGMouseButton, CGEventType, CGEventType, CGEventType) {
        switch try canonicalButton(name) {
        case "left": return (.left, .leftMouseDown, .leftMouseUp, .leftMouseDragged)
        case "right": return (.right, .rightMouseDown, .rightMouseUp, .rightMouseDragged)
        default: return (.center, .otherMouseDown, .otherMouseUp, .otherMouseDragged)
        }
    }

    private func postMouse(_ type: CGEventType, _ p: CGPoint, _ button: CGMouseButton,
                           flags: CGEventFlags, clickState: Int64) throws {
        guard let ev = CGEvent(mouseEventSource: source, mouseType: type,
                               mouseCursorPosition: p, mouseButton: button) else {
            throw HelperError("INTERNAL", "CGEvent mouse creation failed")
        }
        ev.flags = flags
        ev.setIntegerValueField(.mouseEventClickState, value: max(1, clickState))
        if button == .center { ev.setIntegerValueField(.mouseEventButtonNumber, value: 2) }
        ev.post(tap: .cghidEventTap)
    }

    /// Drag type + click state of whichever button is currently held, else a plain move.
    private func motionSpec() -> (CGEventType, CGMouseButton, Int64) {
        guard let held = heldButtons.last, let spec = try? buttonSpec(held) else {
            return (.mouseMoved, .left, 1)
        }
        return (spec.3, spec.0, clickStates[held] ?? 1)
    }

    private func ease(_ t: Double) -> Double {
        t < 0.5 ? 2 * t * t : 1 - pow(-2 * t + 2, 2) / 2
    }

    func flagsNow(_ mods: Set<String>) -> CGEventFlags { Mods.flags(heldMods.union(mods)) }
}

// MARK: - Engine: pointer

extension Engine {
    func move(to target: CGPoint, durationMs: Double, steps: Int?, mods: Set<String>) throws {
        try requirePermission()
        let from = cursor()
        let auto = durationMs <= 0 ? 1 : min(600, Int((clamp(durationMs, 0, maxSleepMs) / 8.0).rounded(.up)))
        let n = max(1, min(maxSteps, steps ?? auto))
        let flags = flagsNow(mods)
        let (type, button, clickState) = motionSpec()
        let delay = n > 1 ? durationMs / Double(n) : 0
        for i in 1...n {
            let e = ease(Double(i) / Double(n))
            let p = i == n ? target
                : CGPoint(x: from.x + (target.x - from.x) * e, y: from.y + (target.y - from.y) * e)
            try postMouse(type, p, button, flags: flags, clickState: clickState)
            if i < n { sleepMs(delay) }
        }
    }

    func down(_ name: String, at p: CGPoint, clickState: Int64, mods: Set<String>) throws {
        try requirePermission()
        let spec = try buttonSpec(name)
        let key = try canonicalButton(name)
        clickStates[key] = clickState
        try postMouse(spec.1, p, spec.0, flags: flagsNow(mods), clickState: clickState)
        heldButtons.removeAll { $0 == key }
        heldButtons.append(key)
    }

    /// Tracking is dropped only after the up actually posted; a button whose release failed is
    /// still down at the OS level and must stay releasable.
    func up(_ name: String, at p: CGPoint, clickState: Int64, mods: Set<String>) throws {
        try requirePermission()
        let spec = try buttonSpec(name)
        let key = try canonicalButton(name)
        try postMouse(spec.2, p, spec.0, flags: flagsNow(mods), clickState: clickState)
        heldButtons.removeAll { $0 == key }
        clickStates[key] = nil
    }

    /// Best effort, for a `defer`: releases the button only if this helper still tracks it down.
    func releaseIfHeld(_ name: String) {
        guard let key = try? canonicalButton(name), heldButtons.contains(key) else { return }
        try? up(key, at: cursor(), clickState: 1, mods: [])
    }

    func click(_ name: String, at p: CGPoint, count: Int, intervalMs: Double, mods: Set<String>) throws {
        try move(to: p, durationMs: 0, steps: 1, mods: mods)
        defer { releaseIfHeld(name) }
        for n in 1...max(1, count) {
            let cs = Int64(min(n, 3))
            try down(name, at: p, clickState: cs, mods: mods)
            try up(name, at: p, clickState: cs, mods: mods)
            if n < count { sleepMs(intervalMs) }
        }
    }

    func path(_ name: String, points: [CGPoint], durationMs: Double,
              holdMs: Double, dwellMs: Double, mods: Set<String>) throws {
        guard let first = points.first, let last = points.last else { throw badArgs("'points' is empty") }
        try move(to: first, durationMs: 0, steps: 1, mods: mods)
        defer { releaseIfHeld(name) }
        try down(name, at: first, clickState: 1, mods: mods)
        sleepMs(holdMs)
        let (lengths, total) = Engine.arcLengths(points)
        let n = max(points.count,
                    durationMs <= 0 ? points.count : Int((clamp(durationMs, 0, maxSleepMs) / 8.0).rounded(.up)))
        let steps = max(2, min(600, n))
        let delay = durationMs / Double(steps)
        for i in 1...steps {
            let e = ease(Double(i) / Double(steps))
            let p = i == steps ? last : Engine.along(points, lengths, total * e)
            try move(to: p, durationMs: 0, steps: 1, mods: mods)
            if i < steps { sleepMs(delay) }
        }
        sleepMs(dwellMs)
        try up(name, at: last, clickState: 1, mods: mods)
    }

    func scroll(at p: CGPoint, dy: Int32, dx: Int32, mods: Set<String>) throws {
        try move(to: p, durationMs: 0, steps: 1, mods: mods)
        guard let ev = CGEvent(scrollWheelEvent2Source: source, units: .line,
                               wheelCount: 2, wheel1: dy, wheel2: dx, wheel3: 0) else {
            throw HelperError("INTERNAL", "CGEvent scroll creation failed")
        }
        ev.location = p
        ev.flags = flagsNow(mods)
        ev.post(tap: .cghidEventTap)
    }

    static func arcLengths(_ pts: [CGPoint]) -> ([Double], Double) {
        var lengths: [Double] = []
        var total = 0.0
        for i in 1..<max(1, pts.count) {
            let d = hypot(pts[i].x - pts[i - 1].x, pts[i].y - pts[i - 1].y)
            lengths.append(d)
            total += d
        }
        return (lengths, total)
    }

    static func along(_ pts: [CGPoint], _ lengths: [Double], _ dist: Double) -> CGPoint {
        var remaining = dist
        for (i, seg) in lengths.enumerated() {
            if seg <= 0 { continue }
            if remaining <= seg {
                let t = remaining / seg
                return CGPoint(x: pts[i].x + (pts[i + 1].x - pts[i].x) * t,
                               y: pts[i].y + (pts[i + 1].y - pts[i].y) * t)
            }
            remaining -= seg
        }
        return pts.last ?? .zero
    }
}

// MARK: - Engine: keyboard

extension Engine {
    private func postKey(_ code: CGKeyCode, down: Bool, flags: CGEventFlags) throws {
        guard let ev = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down) else {
            throw HelperError("INTERNAL", "CGEvent key creation failed")
        }
        ev.flags = flags
        ev.post(tap: .cghidEventTap)
    }

    func keyDown(_ key: String, mods: Set<String>) throws {
        try requirePermission()
        if Keys.isModifier(key) {
            let name = try Mods.canonical(key)
            // The modifier's own down event must already carry its flag, so track first and
            // untrack again if the post failed — otherwise release_all would chase a ghost.
            heldMods.insert(name)
            do { try postKey(Mods.keyCode(name), down: true, flags: flagsNow(mods)) }
            catch { heldMods.remove(name); throw error }
        } else {
            try postKey(try Keys.code(key), down: true, flags: flagsNow(mods))
        }
    }

    func keyUp(_ key: String, mods: Set<String>) throws {
        try requirePermission()
        if Keys.isModifier(key) {
            let name = try Mods.canonical(key)
            heldMods.remove(name)
            try postKey(Mods.keyCode(name), down: false, flags: flagsNow(mods))
        } else {
            try postKey(try Keys.code(key), down: false, flags: flagsNow(mods))
        }
    }

    /// A real user holds the modifiers down around the key, so synthesize the modifier key
    /// events too — native menu key equivalents and global hot keys need them, while WebKit
    /// only needs the flags that every event already carries.
    func press(_ key: String, mods: Set<String>) throws {
        try requirePermission()
        // The defer is armed BEFORE the first modifier goes down: a throw on the second
        // modifier must not leave the first one latched for the rest of the session.
        var pressed: [String] = []
        defer { for m in pressed.reversed() { try? keyUp(m, mods: []) } }
        for m in mods.subtracting(heldMods).sorted() {
            try keyDown(m, mods: [])
            pressed.append(m)
        }
        let code = try Keys.code(key)
        let flags = flagsNow([])
        try postKey(code, down: true, flags: flags)
        try postKey(code, down: false, flags: flags)
    }

    func type(_ text: String, perCharMs: Double) throws {
        try requirePermission()
        let flags = flagsNow([])
        for ch in text {
            if ch == "\n" || ch == "\r" {
                try postKey(CGKeyCode(kVK_Return), down: true, flags: flags)
                try postKey(CGKeyCode(kVK_Return), down: false, flags: flags)
            } else {
                let units = Array(String(ch).utf16)
                // virtualKey 0 is kVK_ANSI_A: the unicode string set below is what the text
                // system consumes, but a consumer reading `event.code` sees "KeyA". Accepted —
                // WebKit's `key`/`data` are correct, and no keycode is free of that ambiguity.
                for isDown in [true, false] {
                    guard let ev = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: isDown) else {
                        throw HelperError("INTERNAL", "CGEvent unicode key creation failed")
                    }
                    ev.flags = flags
                    units.withUnsafeBufferPointer {
                        ev.keyboardSetUnicodeString(stringLength: units.count, unicodeString: $0.baseAddress)
                    }
                    ev.post(tap: .cghidEventTap)
                }
            }
            sleepMs(perCharMs)
        }
    }

    /// Never throws. Releases what this helper is holding, plus anything reached by `force` or
    /// `osState` that the OS still reports held. The reply names what was actually released.
    ///
    /// `force` is the crash path: the CLIENT knows which buttons and modifiers IT may have left
    /// held, so it names exactly those and the user's own hand is never touched. The OS check is
    /// what keeps a name the client guessed at from posting a spurious up.
    ///
    /// `osState` releases everything the OS reports held, the user's own hand and modifiers
    /// included, so it is a MANUAL escape hatch only — a human unwedging a machine by hand. No
    /// automatic caller uses it any more; the client names `force` instead.
    func releaseAll(osState: Bool, force: Set<String> = []) -> [String: Any] {
        let p = cursor()
        var buttons: [String] = []
        var mods: [String] = []
        for name in ["left", "right", "middle"] {
            let cg: CGMouseButton = name == "left" ? .left : (name == "right" ? .right : .center)
            // The OS is consulted only for a name this caller reached for, and only to CONFIRM:
            // an unheld button must not be handed a synthetic up.
            let held = heldButtons.contains(name)
                || ((osState || force.contains(name))
                    && CGEventSource.buttonState(.combinedSessionState, button: cg))
            guard held else { continue }
            if (try? up(name, at: p, clickState: 1, mods: [])) != nil { buttons.append(name) }
        }
        heldButtons.removeAll()
        let wantsOSFlags = osState || !force.intersection(Mods.all).isEmpty
        let osFlags: CGEventFlags = wantsOSFlags ? CGEventSource.flagsState(.combinedSessionState) : []
        for name in Mods.all where heldMods.contains(name)
            || ((osState || force.contains(name)) && osFlags.contains(Mods.mask(name))) {
            heldMods.remove(name)
            if (try? postKey(Mods.keyCode(name), down: false, flags: flagsNow([]))) != nil { mods.append(name) }
        }
        heldMods.removeAll()
        // Ordinary keys. `keydown` is a shipped verb, so a crash between `keydown "a"` and its
        // `keyup` leaves that key physically down and auto-repeating — and neither the tracked
        // set nor `Mods.all` could ever name it, so nothing could release it. Only a key the
        // CALLER named in `force` is touched, and only after `keyState` confirms it is down.
        var keys: [String] = []
        for name in force where Keys.map[name.lowercased()] != nil && Mods.canonicalOrNil(name) == nil {
            let code = Keys.map[name.lowercased()]!
            guard CGEventSource.keyState(.combinedSessionState, key: code) else { continue }
            if (try? postKey(code, down: false, flags: flagsNow([]))) != nil { keys.append(name.lowercased()) }
        }
        return ["releasedButtons": buttons, "releasedMods": mods, "releasedKeys": keys]
    }
}

// MARK: - System queries

enum Sys {
    static func permissions(prompt: Bool) -> [String: Any] {
        let opts: [String: Any] = ["AXTrustedCheckOptionPrompt": prompt]
        let accessibility = AXIsProcessTrustedWithOptions(opts as CFDictionary)
        var screenRecording = CGPreflightScreenCaptureAccess()
        if !screenRecording && prompt {
            _ = CGRequestScreenCaptureAccess()
            screenRecording = CGPreflightScreenCaptureAccess()
        }
        return ["accessibility": accessibility, "screenRecording": screenRecording]
    }

    /// Every window of `pid`, each carrying its CGWindowLevel. Layer 0 is a normal window, but
    /// a Tauri app's real window is not guaranteed to be the only one there, and filtering the
    /// rest away hid exactly the panels a caller needs to see to pick correctly.
    static func windows(pid: pid_t, all: Bool) -> [[String: Any]] {
        let options: CGWindowListOption = all
            ? [.optionAll, .excludeDesktopElements]
            : [.optionOnScreenOnly, .excludeDesktopElements]
        guard let raw = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else { return [] }
        return raw.compactMap { info -> [String: Any]? in
            guard let owner = info[kCGWindowOwnerPID as String] as? Int, owner == Int(pid),
                  let layer = info[kCGWindowLayer as String] as? Int,
                  let windowId = info[kCGWindowNumber as String] as? Int,
                  let b = info[kCGWindowBounds as String] as? [String: Any],
                  let rect = CGRect(dictionaryRepresentation: b as CFDictionary) else { return nil }
            var out: [String: Any] = [
                "windowId": windowId,
                "layer": layer,
                "bounds": ["x": rect.origin.x, "y": rect.origin.y,
                           "w": rect.size.width, "h": rect.size.height],
                "onscreen": all ? ((info[kCGWindowIsOnscreen as String] as? Bool) ?? false) : true,
            ]
            if let name = info[kCGWindowName as String] as? String, !name.isEmpty { out["name"] = name }
            return out
        }
    }

    static func isFrontmost(pid: pid_t) -> Bool {
        NSWorkspace.shared.frontmostApplication?.processIdentifier == pid
    }

    /// Input source ids that translate a bare ANSI position to the character it names. Not
    /// exhaustive by design — anything else is reported as non-ANSI/US so the caller is warned
    /// rather than silently trusting a layout this list does not recognise.
    private static let ansiUsInputSourceIds: Set<String> = [
        "com.apple.keylayout.US", "com.apple.keylayout.ABC", "com.apple.keylayout.USExtended",
    ]

    /// The active input source. Never trapped: every step degrades to a benign default reply
    /// instead of killing the helper, exactly like every other query here.
    static func keyboardLayout() -> [String: Any] {
        guard let source = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue() else {
            return ["inputSourceId": "unknown", "isAnsiUs": false]
        }
        guard let raw = TISGetInputSourceProperty(source, kTISPropertyInputSourceID) else {
            return ["inputSourceId": "unknown", "isAnsiUs": false]
        }
        let id = Unmanaged<CFString>.fromOpaque(raw).takeUnretainedValue() as String
        return ["inputSourceId": id, "isAnsiUs": ansiUsInputSourceIds.contains(id) || id.hasSuffix(".US")]
    }

    static func focus(pid: pid_t) throws -> Bool {
        guard let app = NSRunningApplication(processIdentifier: pid) else {
            throw HelperError("WINDOW_NOT_FOUND", "no running application with pid \(pid)")
        }
        app.activate(options: [.activateIgnoringOtherApps])
        let deadline = Date().addingTimeInterval(0.5)
        repeat {
            if isFrontmost(pid: pid) { return true }
            RunLoop.current.run(until: Date().addingTimeInterval(0.02))
        } while Date() < deadline
        return isFrontmost(pid: pid)
    }
}

// MARK: - Accessibility (read-only)

/// AX locates; CGEvent is the only acceptance-grade actuator. The three verbs in `axActuationVerbs`
/// are the declared exception — see the comment on that set.
///
/// The division of labour is deliberate: accessibility LOCATES what the WebView does not own — a
/// native open/save panel, the app menu, a sheet, a permission dialog, the title-bar buttons — and
/// CGEvent then ACTS on what it located, as a user would. No verb here performs an AXAction.
/// `ax_snapshot` reports an element's action names so a caller can SEE that something is pressable;
/// pressing it is still a click.
///
/// Every walk is bounded. An AX read is synchronous IPC into the target application and can block
/// while that application is busy, and this helper answers one verb at a time, so an unbounded walk
/// would stall every later verb. Each walk carries a node cap, a visit cap, a depth cap and a
/// wall-clock deadline, and every application element gets `AXUIElementSetMessagingTimeout`.
///
/// COORDINATES. Measured on this machine rather than assumed: Google Chrome's `AXWindow` reports
/// `AXPosition`/`AXSize` (0, 33) / 1512 x 949, identical to the same window's `kCGWindowBounds`
/// from `CGWindowListCopyWindowInfo`. AX screen coordinates are therefore already global display
/// points, top-left origin, y down — the space every verb in this helper speaks — so an AX rect can
/// be handed straight to `click` with no conversion.

/// `_AXUIElementGetWindow` is a PRIVATE, unexported HIServices symbol that maps an AXUIElement to
/// the CGWindowID `windows` and `screencapture -l` speak; there is no public equivalent. It is
/// resolved at runtime rather than linked so a macOS release that drops it degrades to the
/// bounds+title fallback in `Ax.windowId` instead of failing to launch the helper at all.
private typealias AxGetWindowFn = @convention(c) (AXUIElement, UnsafeMutablePointer<CGWindowID>) -> AXError

private let axGetWindowFn: AxGetWindowFn? = {
    // RTLD_DEFAULT is ((void *)-2) on Darwin and is not re-exported into Swift.
    guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "_AXUIElementGetWindow") else {
        writeErr("_AXUIElementGetWindow unavailable; window ids fall back to bounds+title matching")
        return nil
    }
    return unsafeBitCast(sym, to: AxGetWindowFn.self)
}()

/// Generation-scoped element refs, mirroring the webview snapshot's `@s<gen>e<n>`.
///
/// Every snapshot bumps the generation and drops the previous map, so a ref from an older snapshot
/// is ABSENT rather than aliased onto whichever element now occupies that slot. That is the whole
/// point: a stale ref must fail loudly instead of resolving to the wrong element and handing a
/// caller a plausible-looking point in somebody else's button.
final class AxRefs {
    static let shared = AxRefs()

    struct Entry {
        let element: AXUIElement
        /// Role as captured. `ax_point` refuses when it has changed — an AX handle can outlive the
        /// thing it named, and a role change is the cheap evidence that it now names something else.
        let role: String?
        let pid: pid_t
    }

    private(set) var generation = 0
    private var map: [String: Entry] = [:]

    func begin() -> Int {
        generation += 1
        map.removeAll()
        return generation
    }

    func put(_ ref: String, _ entry: Entry) { map[ref] = entry }
    func get(_ ref: String) -> Entry? { map[ref] }

    /// `@a<generation>e<index>`. Total by construction: `Int(_:)` returns nil for an overlong or
    /// non-numeric run, so a hostile ref is refused where a fixed-width conversion would trap.
    static func parse(_ ref: String) -> (generation: Int, index: Int)? {
        guard ref.hasPrefix("@a") else { return nil }
        let body = ref.dropFirst(2)
        guard let e = body.firstIndex(of: "e") else { return nil }
        guard let g = Int(body[body.startIndex..<e]), let n = Int(body[body.index(after: e)...]),
              g > 0, n > 0 else { return nil }
        return (g, n)
    }
}

/// One bounded depth-first walk. `stopReason` is the honest account of why it ended.
private final class AxWalk {
    let maxNodes: Int
    let maxVisit: Int
    let maxDepth: Int
    let deadline: UInt64
    let generation: Int?

    var nodes: [[String: Any]] = []
    var visited = 0
    private var stop: String?
    private var prunedByDepth = false

    init(maxNodes: Int, maxVisit: Int, maxDepth: Int, budgetMs: Double, generation: Int?) {
        self.maxNodes = maxNodes
        self.maxVisit = maxVisit
        self.maxDepth = maxDepth
        // Same rule as every other numeric path here: a non-finite value would TRAP the
        // conversion below, so it degrades to the default budget instead.
        let ms = budgetMs.isFinite ? clamp(budgetMs, 0, maxSleepMs) : axWalkBudgetMs
        self.deadline = DispatchTime.now().uptimeNanoseconds + UInt64(ms * 1_000_000.0)
        self.generation = generation
    }

    /// `emit` returns the node dictionary, or nil for an element the caller filtered out. The ref is
    /// minted HERE, after the cap check, so a node that is dropped never consumes an index.
    func run(_ element: AXUIElement, _ pid: pid_t, _ emit: (AXUIElement, Int) -> [String: Any]?) {
        visit(element, 0, pid, emit)
    }

    private func visit(_ element: AXUIElement, _ depth: Int, _ pid: pid_t,
                       _ emit: (AXUIElement, Int) -> [String: Any]?) {
        if stop != nil { return }
        if DispatchTime.now().uptimeNanoseconds > deadline { stop = "deadline"; return }
        if visited >= maxVisit { stop = "maxVisit"; return }
        visited += 1
        if var node = emit(element, depth) {
            if nodes.count >= maxNodes { stop = "maxNodes"; return }
            if let generation {
                let ref = "@a\(generation)e\(nodes.count + 1)"
                node["ref"] = ref
                AxRefs.shared.put(ref, AxRefs.Entry(element: element, role: node["role"] as? String, pid: pid))
            }
            nodes.append(node)
        }
        if depth >= maxDepth { prunedByDepth = true; return }
        for child in Ax.elements(element, kAXChildrenAttribute) {
            visit(child, depth + 1, pid, emit)
            if stop != nil { return }
        }
    }

    /// Why the walk ended. A caller that searched for something and did not find it MUST consult
    /// this before saying "no such thing": a walk that hit a cap searched only part of the tree.
    var stopReason: String { stop ?? (prunedByDepth ? "depth" : "complete") }

    /// `total` counts elements VISITED, not elements that exist: counting the rest would cost the
    /// same IPC the caps exist to avoid. It equals the number found only when `truncated` is false.
    func report() -> [String: Any] {
        let reason = stopReason
        return ["nodes": nodes, "total": visited, "truncated": reason != "complete", "stopReason": reason]
    }
}

enum Ax {
    // MARK: attribute reads

    /// Every non-`.success` is treated as "attribute absent". An ordinary element without a subrole
    /// is not a failed request, and `AXUIElementCopyAttributeValue` reports both the same way.
    static func copyValue(_ element: AXUIElement, _ attribute: String) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else { return nil }
        return value
    }

    static func string(_ element: AXUIElement, _ attribute: String) -> String? {
        guard let raw = copyValue(element, attribute) as? String else { return nil }
        return clip(raw)
    }

    static func bool(_ element: AXUIElement, _ attribute: String) -> Bool? {
        (copyValue(element, attribute) as? NSNumber)?.boolValue
    }

    static func int(_ element: AXUIElement, _ attribute: String) -> Int? {
        (copyValue(element, attribute) as? NSNumber)?.intValue
    }

    /// A conditional downcast to a CoreFoundation class ALWAYS succeeds in Swift (the compiler says
    /// so outright), so the type id is the only real check and the force-cast below is safe only
    /// because it has already passed.
    static func element(_ element: AXUIElement, _ attribute: String) -> AXUIElement? {
        guard let raw = copyValue(element, attribute), CFGetTypeID(raw) == AXUIElementGetTypeID() else { return nil }
        return (raw as! AXUIElement)
    }

    static func elements(_ element: AXUIElement, _ attribute: String) -> [AXUIElement] {
        guard let raw = copyValue(element, attribute) as? [AnyObject] else { return [] }
        return raw.compactMap { item in
            guard CFGetTypeID(item) == AXUIElementGetTypeID() else { return nil }
            return (item as! AXUIElement)
        }
    }

    /// AX hands position and size back as `AXValue`s of `.cgPoint` / `.cgSize`, which have to be
    /// unwrapped with `AXValueGetValue`; the rect they describe is already global, top-left origin.
    static func rect(_ element: AXUIElement) -> CGRect? {
        guard let pv = copyValue(element, kAXPositionAttribute), CFGetTypeID(pv) == AXValueGetTypeID(),
              let sv = copyValue(element, kAXSizeAttribute), CFGetTypeID(sv) == AXValueGetTypeID() else { return nil }
        var origin = CGPoint.zero
        var size = CGSize.zero
        guard withUnsafeMutablePointer(to: &origin, { AXValueGetValue(pv as! AXValue, .cgPoint, $0) }),
              withUnsafeMutablePointer(to: &size, { AXValueGetValue(sv as! AXValue, .cgSize, $0) }),
              origin.x.isFinite, origin.y.isFinite, size.width.isFinite, size.height.isFinite else { return nil }
        return CGRect(origin: origin, size: size)
    }

    /// Action NAMES only. Reporting that an element answers to `AXPress` is evidence for the caller;
    /// reading a name never performs it. `ax_press` is the one verb that does, and it refuses any
    /// action that does not appear in this list.
    static func actions(_ element: AXUIElement) -> [String] {
        var names: CFArray?
        guard AXUIElementCopyActionNames(element, &names) == .success,
              let list = names as? [String] else { return [] }
        return Array(list.prefix(axMaxActions))
    }

    /// `AXValue` is whatever the element decided: a string, a number, a boolean, or a structure this
    /// helper will not pretend to understand. Anything else reports null rather than a description.
    static func valueJSON(_ element: AXUIElement) -> Any {
        guard let raw = copyValue(element, kAXValueAttribute) else { return NSNull() }
        if let text = raw as? String { return clip(text) }
        if let number = raw as? NSNumber { return number }
        return NSNull()
    }

    static func clip(_ text: String) -> String {
        text.count <= axMaxTextChars ? text : String(text.prefix(axMaxTextChars))
    }

    // MARK: process + application elements

    /// `kill(pid, 0)` is the only liveness check that works for a faceless CLI as well as for an
    /// app; EPERM means the process exists and belongs to somebody else. `NSRunningApplication` is
    /// not a substitute — it answers about applications, and a target here need not be one.
    static func requireLive(_ pid: pid_t) throws {
        guard kill(pid, 0) == 0 || errno == EPERM else {
            throw HelperError("WINDOW_NOT_FOUND", "no process with pid \(pid)")
        }
    }

    static func app(_ pid: pid_t) throws -> AXUIElement {
        try requireLive(pid)
        let element = AXUIElementCreateApplication(pid)
        // A busy target can block an AX read indefinitely; this helper answers one verb at a time.
        _ = AXUIElementSetMessagingTimeout(element, axMessagingTimeoutS)
        return element
    }

    // MARK: window ids

    fileprivate struct CgWindow {
        let id: Int
        let rect: CGRect
        let name: String?
    }

    /// Read straight from CoreGraphics rather than from `Sys.windows`' JSON: re-parsing this
    /// helper's own reply shape to match a rect would be one more thing to keep in step.
    fileprivate static func cgWindows(_ pid: pid_t) -> [CgWindow] {
        guard let raw = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID)
            as? [[String: Any]] else { return [] }
        return raw.compactMap { info in
            guard let owner = info[kCGWindowOwnerPID as String] as? Int, owner == Int(pid),
                  let id = info[kCGWindowNumber as String] as? Int,
                  let bounds = info[kCGWindowBounds as String] as? [String: Any],
                  let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { return nil }
            let name = info[kCGWindowName as String] as? String
            return CgWindow(id: id, rect: rect, name: (name?.isEmpty ?? true) ? nil : name)
        }
    }

    /// The CGWindowID for an AX window, and which path produced it, so a caller can tell a
    /// correlated id from a guessed one.
    ///
    /// The fallback is deliberately narrow. Two windows of one application routinely share a rect to
    /// the pixel (two Chrome windows on this machine both report 0,33 1512x949), and the CG name is
    /// a DECORATED, elided form of the AX title ("Meet – ftf-ywkw-zmq 🔊" against the full tab
    /// title), so neither a prefix match nor "pick the first" would be evidence of anything. When
    /// bounds leave more than one candidate and exact titles do not settle it, the answer is
    /// `ambiguous` and a null id — the same refusal as everywhere else in this helper.
    fileprivate static func windowId(_ window: AXUIElement, _ pid: pid_t, _ rect: CGRect?,
                                     _ title: String?, _ cache: [CgWindow]? = nil) -> (Int?, String) {
        if let fn = axGetWindowFn {
            var id: CGWindowID = 0
            if fn(window, &id) == .success, id != 0 { return (Int(id), "axPrivate") }
        }
        guard let rect else { return (nil, "none") }
        let sameRect = (cache ?? cgWindows(pid)).filter { near($0.rect, rect) }
        if sameRect.count == 1 { return (sameRect[0].id, "boundsMatch") }
        if sameRect.isEmpty { return (nil, "none") }
        // A title clipped by `Ax.clip` could agree on its first 256 characters and differ after,
        // so it is not evidence either.
        guard let title, title.count < axMaxTextChars else { return (nil, "ambiguous") }
        let sameTitle = sameRect.filter { $0.name == title }
        if sameTitle.count == 1 { return (sameTitle[0].id, "boundsMatch") }
        return (nil, "ambiguous")
    }

    private static func near(_ a: CGRect, _ b: CGRect) -> Bool {
        abs(a.origin.x - b.origin.x) <= 1 && abs(a.origin.y - b.origin.y) <= 1
            && abs(a.size.width - b.size.width) <= 1 && abs(a.size.height - b.size.height) <= 1
    }

    // MARK: windows

    struct Window {
        let element: AXUIElement
        let rect: CGRect?
        let row: [String: Any]
    }

    /// Every AX window of `pid`. Note that `AXWindows` is not guaranteed to hold `AXWindow`s —
    /// Finder reports its desktop as an `AXScrollArea` there — so the role is reported, never assumed.
    static func windows(_ pid: pid_t) throws -> [Window] {
        let appElement = try app(pid)
        let key = element(appElement, kAXFocusedWindowAttribute)
        let cache = cgWindows(pid)
        return elements(appElement, kAXWindowsAttribute).prefix(axMaxWindows).map { window in
            let frame = rect(window)
            let title = string(window, kAXTitleAttribute)
            let (id, source) = windowId(window, pid, frame, title, cache)
            let row: [String: Any] = [
                "role": j(string(window, kAXRoleAttribute)),
                "subrole": j(string(window, kAXSubroleAttribute)),
                "title": j(title),
                "bounds": j(frame),
                "main": j(bool(window, kAXMainAttribute)),
                "modal": j(bool(window, kAXModalAttribute)),
                // `AXFocused` is an element attribute and most windows do not answer it; the
                // application's own `AXFocusedWindow` is the authority on which window is key.
                "focused": key.map { CFEqual($0, window) } ?? false,
                "windowId": j(id),
                "windowIdSource": source,
            ]
            return Window(element: window, rect: frame, row: row)
        }
    }

    static func focusedWindow(_ pid: pid_t) throws -> Window? {
        try windows(pid).first { ($0.row["focused"] as? Bool) == true }
    }

    // MARK: nodes

    /// One rule for the node shape, so a consumer never has to ask whether a missing key means
    /// "AX did not expose it" or "the helper dropped it": every key is ALWAYS present, and a value
    /// AX does not expose is null. `actions` is always an array, `depth` always a number.
    static func node(_ element: AXUIElement, _ depth: Int) -> [String: Any] {
        [
            "role": j(string(element, kAXRoleAttribute)),
            "subrole": j(string(element, kAXSubroleAttribute)),
            "title": j(string(element, kAXTitleAttribute)),
            "value": valueJSON(element),
            "enabled": j(bool(element, kAXEnabledAttribute)),
            "focused": j(bool(element, kAXFocusedAttribute)),
            "bounds": j(rect(element)),
            "depth": depth,
            "actions": actions(element),
        ]
    }

    // MARK: displays

    /// `CGDisplayBounds` is already in the global, top-left-origin space AX and CGEvent share, which
    /// `NSScreen.frame` (bottom-left, AppKit) is not.
    static func displays() -> [CGRect] {
        var count: UInt32 = 0
        guard CGGetActiveDisplayList(0, nil, &count) == .success, count > 0 else { return [] }
        var ids = [CGDirectDisplayID](repeating: 0, count: Int(count))
        guard CGGetActiveDisplayList(count, &ids, &count) == .success else { return [] }
        return ids.prefix(Int(count)).map { CGDisplayBounds($0) }
    }

    // MARK: menus

    /// Carbon's menu modifier mask, as AX reports it in `AXMenuItemCmdModifiers`: bit 0 shift,
    /// bit 1 option, bit 2 control, and bit 3 meaning NO command — Command is implied when clear.
    /// Measured on this machine: Chrome's "New Incognito Window" (⇧⌘N) reports 1, and its
    /// "About Google Chrome", which has no key equivalent at all, reports 8.
    static func menuMods(_ raw: Int) -> [String] {
        var out: [String] = []
        if raw & 8 == 0 { out.append("Command") }
        if raw & 4 != 0 { out.append("Control") }
        if raw & 2 != 0 { out.append("Option") }
        if raw & 1 != 0 { out.append("Shift") }
        return out
    }

    /// The key equivalent AX exposes, or null when the item has none. `char` is the printable key,
    /// `virtualKey` the ANSI keycode AX uses for keys that have no character (arrows, function keys).
    static func menuKey(_ element: AXUIElement) -> Any {
        let char = string(element, kAXMenuItemCmdCharAttribute)
        let virtualKey = int(element, kAXMenuItemCmdVirtualKeyAttribute)
        guard char?.isEmpty == false || virtualKey != nil else { return NSNull() }
        let raw = int(element, kAXMenuItemCmdModifiersAttribute)
        return [
            "char": j(char),
            "virtualKey": j(virtualKey),
            "glyph": j(int(element, kAXMenuItemCmdGlyphAttribute)),
            "modifiersRaw": j(raw),
            "mods": menuMods(raw ?? 0),
        ] as [String: Any]
    }
}

/// A present argument of the wrong type is REFUSED, never ignored. Dropping a malformed `role`
/// would widen `ax_find` from "these elements" to "every element" and still answer `ok`, which is
/// the silently-wrong direction this helper exists to avoid.
func optStr(_ r: [String: Any], _ k: String) throws -> String? {
    guard let raw = r[k] else { return nil }
    guard let text = raw as? String else { throw badArgs("'\(k)' must be a string") }
    return text
}

/// JSON coercion for optionals. Written as overloads on purpose: assigning a `String?` into an
/// `Any` produces an `Any` wrapping `Optional.none`, which `JSONSerialization` rejects outright,
/// and the reply would degrade to the "unserializable reply" fallback.
func j(_ v: String?) -> Any { v ?? NSNull() }
func j(_ v: Bool?) -> Any { v ?? NSNull() }
func j(_ v: Int?) -> Any { v ?? NSNull() }
func j(_ v: CGRect?) -> Any { v.map(rectJSON) ?? NSNull() }

func rectJSON(_ r: CGRect) -> [String: Any] {
    ["x": r.origin.x, "y": r.origin.y, "w": r.size.width, "h": r.size.height]
}

// MARK: - Accessibility verbs

let axVerbs: Set<String> = [
    "ax_windows", "ax_snapshot", "ax_find", "ax_point", "ax_focused_window", "ax_modal", "ax_menu",
    "ax_press", "ax_set_value", "ax_menu_press",
]

/// The three verbs that ACT. Everything else in `axVerbs` only reads.
///
/// This is a deliberate amendment to the rule this file used to state absolutely ("AX reads,
/// never AX acts"). The rule it replaces is narrower and still binding: **CGEvent is the only
/// acceptance-grade actuator.** An AX action asks the control to perform itself, which is not
/// physical input — no cursor moves, no key is pressed, the application need not even be in
/// front — and the client labels every one of them `mode:"accessibility"` so a report cannot
/// present one as a real-user step. What it buys is the ability to drive native chrome, which
/// the WebView does not own, from a session that must not touch the user's desktop.
let axActuationVerbs: Set<String> = ["ax_press", "ax_set_value", "ax_menu_press"]

/// Window-chrome subroles whose press the USER sees: a Space switch, a resize, a vanished window.
let disruptiveSubroles: Set<String> = [
    "AXFullScreenButton", "AXZoomButton", "AXMinimizeButton", "AXCloseButton",
]

func handleAx(_ verb: String, _ r: [String: Any]) throws -> [String: Any]? {
    guard axVerbs.contains(verb) else { return nil }
    // Reading AX needs the same Accessibility grant that posting CGEvents does, and the client
    // already maps this code — so an ungranted machine reports the grant instead of an empty tree.
    try Engine.shared.requirePermission()
    guard var out = try axDispatch(verb, r) else { return nil }
    // Every reply says whether this verb CHANGED anything, so the client never has to infer
    // read-versus-act from the verb name.
    out["actuated"] = axActuationVerbs.contains(verb)
    return out
}

private func axDispatch(_ verb: String, _ r: [String: Any]) throws -> [String: Any]? {
    switch verb {
    case "ax_windows":
        let pid = try pidArg(r)
        return [
            "pid": Int(pid),
            "privateWindowIdApi": axGetWindowFn != nil,
            "windows": try Ax.windows(pid).map(\.row),
        ]
    case "ax_snapshot":
        return try axSnapshot(r)
    case "ax_find":
        return try axFind(r)
    case "ax_point":
        return try axPoint(r)
    case "ax_focused_window":
        let pid = try pidArg(r)
        let key = try Ax.focusedWindow(pid)
        return ["pid": Int(pid), "window": key?.row ?? NSNull()]
    case "ax_modal":
        return try axModal(r)
    case "ax_menu":
        return try axMenu(r)
    case "ax_press":
        return try axPress(r)
    case "ax_set_value":
        return try axSetValue(r)
    case "ax_menu_press":
        return try axMenuPress(r)
    default:
        return nil
    }
}

/// Performs an action on the element behind a ref.
///
/// Two gates, and BOTH are needed, because `AXUIElementPerformAction` returns `.success` for a
/// press that does nothing at all.
///
/// 1. The action must be one the element advertises. An unadvertised action gets a silent
///    `.actionUnsupported`.
/// 2. The element must not be DISABLED. This is the common case and the dangerous one: a disabled
///    control still advertises `AXPress` (a disabled Finder menu item advertises
///    `AXCancel, AXPress, AXPick`), `AXUIElementPerformAction` still answers `.success`, and
///    nothing happens. The motivating example is an `NSSavePanel` whose Save button stays disabled
///    until the filename field is non-empty — exactly the button an agent is most likely to press.
///
/// `AXEnabled` absent (nil) is NOT treated as disabled: plenty of elements never publish it, and
/// refusing on silence would block a legitimate press. Only an explicit `false` refuses.
private func axPress(_ r: [String: Any]) throws -> [String: Any] {
    let ref = try str(r, "ref")
    let action = try optStr(r, "action") ?? (kAXPressAction as String)
    let (entry, role) = try axValidated(ref)
    let available = Ax.actions(entry.element)
    guard available.contains(action) else {
        throw HelperError("INVALID_TARGET",
                          "'\(ref)' (\(role ?? "no role")) does not offer \(action); it offers "
                            + (available.isEmpty ? "no actions at all" : available.joined(separator: ", ")))
    }
    guard Ax.bool(entry.element, kAXEnabledAttribute) != false else {
        throw HelperError("INVALID_TARGET",
                          "'\(ref)' (\(role ?? "no role")) is disabled; \(action) on it would report "
                            + "success and do nothing")
    }
    // Window chrome is the one class of AX action whose effect the USER sees immediately: pressing
    // the full-screen or zoom button moves the app to its own Space or resizes it on top of
    // whatever they are doing, and closing or minimising takes the window the session is driving
    // out from under it. Nothing about these is "non-interfering", so they are refused unless the
    // caller names the action explicitly and accepts it.
    let subrole = Ax.string(entry.element, kAXSubroleAttribute)
    if let subrole, disruptiveSubroles.contains(subrole), try optStr(r, "acceptDisruption") == nil {
        throw HelperError("INVALID_TARGET",
                          "'\(ref)' is \(subrole) — pressing it changes the user's desktop "
                            + "(full screen, zoom, minimise or close), which a background session must "
                            + "not do silently. Pass acceptDisruption:\"yes\" to do it deliberately.")
    }
    let status = AXUIElementPerformAction(entry.element, action as CFString)
    guard status == .success else {
        throw HelperError("HELPER_FAILED",
                          "\(action) on '\(ref)' failed with AXError \(status.rawValue)")
    }
    return ["ref": ref, "action": action, "role": j(role)]
}

/// Sets an element's `AXValue`, for the one case a press cannot cover: a text field in a native
/// panel. Refuses when the attribute is not settable rather than reporting a write that the
/// application discarded.
private func axSetValue(_ r: [String: Any]) throws -> [String: Any] {
    let ref = try str(r, "ref")
    let value = try str(r, "value")
    let (entry, role) = try axValidated(ref)
    var settable: DarwinBoolean = false
    let ask = AXUIElementIsAttributeSettable(entry.element, kAXValueAttribute as CFString, &settable)
    guard ask == .success, settable.boolValue else {
        throw HelperError("INVALID_TARGET",
                          "AXValue is not settable on '\(ref)' (\(role ?? "no role")); "
                            + "click it and type instead")
    }
    let status = AXUIElementSetAttributeValue(entry.element, kAXValueAttribute as CFString, value as CFTypeRef)
    guard status == .success else {
        throw HelperError("HELPER_FAILED",
                          "setting AXValue on '\(ref)' failed with AXError \(status.rawValue)")
    }
    // Read back: an application is free to accept the write and store something else (a path
    // field that normalises, a field that rejects the characters), and the caller needs to know.
    //
    // The comparison is deliberately NOT against `Ax.string`, which clips at `axMaxTextChars` for
    // transport. Comparing a clipped read-back against the full request would report `matched:false`
    // for every value longer than 256 characters and blame the application for the helper's own
    // truncation. So the verdict is computed on the unclipped value, and only the reported string
    // is clipped. A non-String `AXValue` (a numeric stepper, say) cannot be compared at all — that
    // is reported as `null` with `matched:false`, which is the truth: we do not know.
    var rawBack: CFTypeRef?
    let got = AXUIElementCopyAttributeValue(entry.element, kAXValueAttribute as CFString, &rawBack)
    let full = got == .success ? (rawBack as? String) : nil
    return ["ref": ref, "role": j(role), "requested": value,
            "value": full.map { Ax.clip($0) as Any } ?? NSNull(),
            "matched": full == value,
            // True when the reported string was shortened for transport, so a caller comparing
            // `value` to what it sent knows why they differ.
            "valueClipped": (full?.count ?? 0) > axMaxTextChars]
}

/// Presses a menu-bar item named by its title path, e.g. ["File", "Save"].
///
/// Titles, not refs: `ax_menu` reports a path for every item and the menu bar is stable, so a
/// path is the addressable thing. A path that names no item, or more than one, refuses with the
/// paths that do exist — picking one would press whichever menu happened to sort first.
private func axMenuPress(_ r: [String: Any]) throws -> [String: Any] {
    let pid = try pidArg(r)
    guard let raw = r["path"] as? [Any] else { throw badArgs("'path' must be an array of menu titles") }
    let path = raw.compactMap { $0 as? String }
    guard path.count == raw.count, !path.isEmpty else {
        throw badArgs("'path' must be a non-empty array of menu item titles")
    }
    // A one-element path names a MENU BAR ITEM, not a command. `AXPress` on one is "click the menu
    // title": it opens the menu and enters AppKit menu tracking, which drops a menu over whatever
    // the user is looking at and swallows their next click — the opposite of what a background
    // session promises, and never what a caller actually wants. There is no legitimate use for it,
    // so it is refused rather than made conditional on the policy.
    guard path.count >= 2 else {
        throw HelperError("INVALID_TARGET",
                          "'\(path[0])' is a menu bar title, not a command; pressing it would open the "
                            + "menu on screen. Name the item too, e.g. [\"\(path[0])\", \"Save\"].")
    }
    guard let bar = Ax.element(try Ax.app(pid), kAXMenuBarAttribute) else {
        throw HelperError("ELEMENT_NOT_FOUND", "pid \(pid) has no menu bar")
    }
    var ancestors: [(depth: Int, title: String)] = []
    var matches: [(element: AXUIElement, enabled: Bool)] = []
    var known: [String] = []
    let walk = AxWalk(maxNodes: axMenuMaxNodes, maxVisit: axMenuMaxVisit, maxDepth: axMenuMaxDepth,
                      budgetMs: axWalkBudgetMs, generation: nil)
    // Same traversal and the same ancestor bookkeeping as `ax_menu`, so the path a caller reads
    // there is exactly the path that resolves here.
    walk.run(bar, pid) { element, depth in
        let title = Ax.string(element, kAXTitleAttribute)
        while let last = ancestors.last, last.depth >= depth { ancestors.removeLast() }
        if let title, !title.isEmpty { ancestors.append((depth, title)) }
        let here = ancestors.map(\.title)
        if here.count == path.count {
            let enabled = Ax.bool(element, kAXEnabledAttribute) != false
            if here == path { matches.append((element, enabled)) }
            if Ax.actions(element).contains(kAXPressAction as String) {
                known.append(here.joined(separator: " > "))
            }
        }
        return [:]
    }
    // A real menu bar is routinely ambiguous: AppKit publishes alternate items that share a title
    // (Finder's "File > Eject" appears three times), of which normally exactly one is enabled.
    // Narrowing by AXEnabled is a DETERMINATION, not a guess, and without it a large class of
    // ordinary menu commands would be unreachable. Only when that still leaves several — or none
    // at all — does this refuse.
    let enabled = matches.filter(\.enabled)
    let chosen = enabled.count == 1 ? enabled : matches
    guard chosen.count == 1, let item = chosen.first else {
        throw axMenuRefusal(path, matches.count, known, walk.stopReason)
    }
    guard item.enabled else {
        throw HelperError("INVALID_TARGET", "menu item \(path.joined(separator: " > ")) is disabled")
    }
    let status = AXUIElementPerformAction(item.element, kAXPressAction as CFString)
    guard status == .success else {
        throw HelperError("HELPER_FAILED",
                          "pressing \(path.joined(separator: " > ")) failed with AXError \(status.rawValue)")
    }
    return ["pid": Int(pid), "path": path]
}

/// Depth-first walk of one window (or of `root`), minting a fresh generation of refs.
private func axSnapshot(_ r: [String: Any]) throws -> [String: Any] {
    let pid = try pidArg(r)
    let maxNodes = try intArg(r, "maxNodes", 1, axMaxNodes) ?? axDefaultMaxNodes
    let (root, window, source) = try axRoot(r, pid)
    // The root ref is resolved against the CURRENT generation BEFORE the bump, so drilling into a
    // node from the snapshot you are holding works, while a ref from an older one is still stale.
    let walk = AxWalk(maxNodes: maxNodes, maxVisit: axMaxVisit, maxDepth: axMaxDepth,
                      budgetMs: axWalkBudgetMs, generation: AxRefs.shared.begin())
    walk.run(root, pid) { element, depth in Ax.node(element, depth) }
    var out = walk.report()
    out["pid"] = Int(pid)
    out["generation"] = AxRefs.shared.generation
    out["windowSource"] = source
    out["window"] = window?.row ?? NSNull()
    return out
}

/// The same walk, filtered. It is a SNAPSHOT: it bumps the generation exactly as `ax_snapshot`
/// does, so refs handed out before it are stale. Refs that silently survive a re-walk are the bug
/// generation scoping exists to prevent.
private func axFind(_ r: [String: Any]) throws -> [String: Any] {
    let pid = try pidArg(r)
    let role = try optStr(r, "role")
    let title = try optStr(r, "title")
    let value = try optStr(r, "value")
    // Rooted at the application element, so a match is found in any window (and in the menu bar),
    // not only in the focused one.
    let app = try Ax.app(pid)
    // The emit cap is the constant, not a caller argument: `ax_find` returns only matches, so what
    // actually bounds its cost is `axMaxVisit`, and a second knob for the cheap half would mislead.
    let walk = AxWalk(maxNodes: axDefaultMaxNodes, maxVisit: axMaxVisit, maxDepth: axMaxDepth,
                      budgetMs: axWalkBudgetMs, generation: AxRefs.shared.begin())
    walk.run(app, pid) { element, depth in
        let node = Ax.node(element, depth)
        // `role` is an exact match (AX roles are a fixed vocabulary); `title` and `value` are
        // case-insensitive substrings, because the text a caller knows is rarely the whole label.
        if let role, node["role"] as? String != role { return nil }
        if let title, !contains(node["title"], title) { return nil }
        if let value, !contains(node["value"], value) { return nil }
        return node
    }
    var out = walk.report()
    out["pid"] = Int(pid)
    out["generation"] = AxRefs.shared.generation
    return out
}

/// Substring match against a node field. A non-string field (an `AXValue` that came back as a
/// number) never matches, which is why `value` is documented as a TEXT filter.
private func contains(_ haystack: Any?, _ needle: String) -> Bool {
    guard let text = haystack as? String else { return false }
    if needle.isEmpty { return true }
    return text.range(of: needle, options: .caseInsensitive) != nil
}

/// The identity half of the ref ladder: the ref is in the CURRENT generation, the element still
/// answers, and it still names the role it was captured as.
///
/// Shared by every verb that touches a ref — `ax_point`, `ax_press`, `ax_set_value` — so a stale
/// handle can never be acted on by one path just because another path happened to check it. Its
/// two refusals are distinct codes (`ELEMENT_STALE` versus `ELEMENT_NOT_FOUND`) so a caller can
/// tell "re-snapshot" from "it is gone"; the geometry refusals belong to `ax_point`, below.
private func axValidated(_ ref: String) throws -> (entry: AxRefs.Entry, role: String?) {
    guard let parsed = AxRefs.parse(ref) else {
        throw badArgs("'ref' must look like @a<generation>e<index>")
    }
    let generation = AxRefs.shared.generation
    guard let entry = AxRefs.shared.get(ref) else {
        if parsed.generation != generation {
            throw HelperError("ELEMENT_STALE",
                              "ref '\(ref)' is from snapshot generation \(parsed.generation); "
                                + "the current generation is \(generation)")
        }
        throw HelperError("ELEMENT_NOT_FOUND", "no element '\(ref)' in snapshot generation \(generation)")
    }
    // A gone element answers `.invalidUIElement` (or stops answering at all) to everything, which is
    // how it is told apart from an element that merely has no role.
    var raw: CFTypeRef?
    let status = AXUIElementCopyAttributeValue(entry.element, kAXRoleAttribute as CFString, &raw)
    if status == .invalidUIElement || status == .cannotComplete || status == .notImplemented {
        throw HelperError("ELEMENT_NOT_FOUND", "the element behind '\(ref)' no longer responds (\(status.rawValue))")
    }
    let role = raw as? String
    if let was = entry.role, was != role {
        throw HelperError("ELEMENT_STALE", "element identity changed (\(was) → \(role ?? "none"))")
    }
    return (entry, role)
}

/// Re-validates a ref and reports where the element is NOW.
///
/// This is the AX half of the webview resolver's stale/moving ladder, and its job is to refuse
/// rather than guess: every failure below would otherwise produce a plausible coordinate that lands
/// in the wrong place, in the user's real session. Five distinguishable refusals — stale, not
/// found, moving, empty frame, and not on any display — the first two from `axValidated`.
private func axPoint(_ r: [String: Any]) throws -> [String: Any] {
    let ref = try str(r, "ref")
    // Captured BEFORE validation so the reported generation is the one the check ran against.
    let generation = AxRefs.shared.generation
    let (entry, role) = try axValidated(ref)
    guard let first = Ax.rect(entry.element) else {
        throw HelperError("ELEMENT_NOT_FOUND", "the element behind '\(ref)' no longer reports a position and size")
    }
    // Two samples, the same 50 ms apart and the same 0.5 pt tolerance the webview resolver uses
    // (`rectStable` in geometry/mapping.ts). A sheet sliding in has a perfectly plausible rect at
    // every instant of the animation, and clicking one of them lands somewhere else entirely.
    sleepMs(axSampleGapMs)
    guard let rect = Ax.rect(entry.element) else {
        throw HelperError("ELEMENT_NOT_FOUND", "the element behind '\(ref)' stopped reporting a position and size")
    }
    guard axRectStable(first, rect) else {
        throw HelperError("ELEMENT_MOVING",
                          "'\(ref)' moved between two samples \(Int(axSampleGapMs)) ms apart "
                            + "(\(rectText(first)) → \(rectText(rect)))")
    }
    guard rect.size.width > 0, rect.size.height > 0 else {
        throw HelperError("POINT_OUTSIDE_WINDOW", "the element behind '\(ref)' has an empty frame \(rectText(rect))")
    }
    let center = CGPoint(x: rect.midX, y: rect.midY)
    let displays = Ax.displays()
    guard let display = displays.first(where: { $0.contains(center) }) else {
        throw HelperError("POINT_OUTSIDE_WINDOW",
                          "the centre of '\(ref)' \(ptText(center)) is not on any of \(displays.count) display(s)")
    }

    // Which window the element sits in, so a caller can check it is about to click inside the window
    // it meant. The climb is bounded: an AX parent chain is not guaranteed to terminate.
    var container: AXUIElement? = entry.element
    var windowId: Any = NSNull()
    var windowTitle: Any = NSNull()
    for _ in 0..<axMaxParentHops {
        guard let current = container else { break }
        if Ax.string(current, kAXRoleAttribute) == kAXWindowRole {
            let title = Ax.string(current, kAXTitleAttribute)
            let (id, _) = Ax.windowId(current, entry.pid, Ax.rect(current), title)
            windowId = j(id)
            windowTitle = j(title)
            break
        }
        container = Ax.element(current, kAXParentAttribute)
    }

    return [
        "ref": ref,
        "generation": generation,
        "pid": Int(entry.pid),
        "role": j(role),
        "subrole": j(Ax.string(entry.element, kAXSubroleAttribute)),
        "title": j(Ax.string(entry.element, kAXTitleAttribute)),
        // Reported, never enforced: AX `enabled` is the application's own claim, and refusing on it
        // would block a legitimate click on something that merely mislabels itself.
        "enabled": j(Ax.bool(entry.element, kAXEnabledAttribute)),
        "focused": j(Ax.bool(entry.element, kAXFocusedAttribute)),
        "bounds": rectJSON(rect),
        "center": ptJSON(center),
        "display": rectJSON(display),
        "frontmost": Sys.isFrontmost(pid: entry.pid),
        "windowId": windowId,
        "windowTitle": windowTitle,
    ]
}

/// Whether a sheet or modal is covering the application, and which one.
///
/// Known limit, stated rather than hidden: a SANDBOXED application's open/save panel is hosted by
/// `com.apple.appkit.xpc.openAndSavePanelService`, a different process, and is therefore invisible
/// under this pid. OneCAD's panels are in-process, so they do appear here.
private func axModal(_ r: [String: Any]) throws -> [String: Any] {
    let pid = try pidArg(r)
    var blockers: [[String: Any]] = []
    for window in try Ax.windows(pid) {
        let subrole = window.row["subrole"] as? String
        let role = window.row["role"] as? String
        var reason: String?
        if (window.row["modal"] as? Bool) == true { reason = "modal" }
        else if subrole == kAXSheetRole || role == kAXSheetRole { reason = "sheet" }
        else if subrole == kAXDialogSubrole || subrole == kAXSystemDialogSubrole { reason = "dialog" }
        // A sheet is also reachable as an attribute of the window it is attached to. The literal is
        // deliberate: HIServices exports no constant for it, and an unsupported attribute simply
        // reads as absent.
        else if !Ax.elements(window.element, "AXSheets").isEmpty { reason = "sheet" }
        guard let reason else { continue }
        var row = window.row
        row["reason"] = reason
        blockers.append(row)
    }
    return ["pid": Int(pid), "modal": !blockers.isEmpty, "blockers": blockers]
}

/// The `AXMenuBar` tree, flattened.
///
/// Bounded harder than a window walk because a menu bar is big: a dozen top-level menus, each with
/// tens of items, each wrapped in an `AXMenu`. `path` carries the non-empty titles from the menu bar
/// down, which drops those wrappers from the path without pretending they are not in the tree.
///
/// Reading it asks the target application to validate its menus, which is what AX does for any
/// assistive client; nothing is opened and nothing is pressed.
private func axMenu(_ r: [String: Any]) throws -> [String: Any] {
    let pid = try pidArg(r)
    guard let bar = Ax.element(try Ax.app(pid), kAXMenuBarAttribute) else {
        return ["pid": Int(pid), "hasMenuBar": false, "items": [], "total": 0,
                "truncated": false, "stopReason": "complete"]
    }
    // Titled ancestors, keyed by their own depth: an `AXMenu` wrapper contributes no title, so
    // path length and depth diverge and only the recorded depth can unwind the stack correctly.
    var ancestors: [(depth: Int, title: String)] = []
    let walk = AxWalk(maxNodes: axMenuMaxNodes, maxVisit: axMenuMaxVisit, maxDepth: axMenuMaxDepth,
                      budgetMs: axWalkBudgetMs, generation: nil)
    walk.run(bar, pid) { element, depth in
        let title = Ax.string(element, kAXTitleAttribute)
        while let last = ancestors.last, last.depth >= depth { ancestors.removeLast() }
        if let title, !title.isEmpty { ancestors.append((depth, title)) }
        return [
            "title": j(title),
            "role": j(Ax.string(element, kAXRoleAttribute)),
            "enabled": j(Ax.bool(element, kAXEnabledAttribute)),
            "bounds": j(Ax.rect(element)),
            "depth": depth,
            "path": ancestors.map(\.title),
            "key": Ax.menuKey(element),
        ]
    }
    var out = walk.report()
    out["items"] = out.removeValue(forKey: "nodes") ?? []
    out["pid"] = Int(pid)
    out["hasMenuBar"] = true
    return out
}

/// Builds `ax_menu_press`'s refusal.
///
/// Two things it gets right that a bare "not found" does not.
///
/// **Truncation is not absence.** A walk that stopped on a node/visit/time cap searched only part
/// of the menu bar, so "no menu item at that path" would be a false statement about an item that
/// may well exist. That case says so and names the cap.
///
/// **The candidates are ranked by relevance, not alphabetically.** A global alphabetical sort with
/// a cap is worse than useless on a real menu bar — Finder has 132 distinct two-segment paths, so
/// an alphabetical list truncated at 40 stops inside "Edit >" and never shows a single "File >"
/// entry to somebody who mistyped a File command. Paths sharing the longest prefix with the
/// request come first, which is exactly where the correction lives.
private func axMenuRefusal(_ path: [String], _ matchCount: Int, _ known: [String],
                           _ stopReason: String) -> HelperError {
    let asked = path.joined(separator: " > ")
    if matchCount == 0, stopReason != "complete" {
        return HelperError("ELEMENT_NOT_FOUND",
                           "the menu walk stopped early (\(stopReason)) before finding \(asked); "
                             + "the item may exist beyond the walk's limit")
    }
    let shared = { (candidate: String) -> Int in
        let parts = candidate.components(separatedBy: " > ")
        var n = 0
        while n < parts.count, n < path.count, parts[n] == path[n] { n += 1 }
        return n
    }
    let ranked = Array(Set(known))
        .sorted { a, b in shared(a) == shared(b) ? a < b : shared(a) > shared(b) }
        .prefix(40)
    let available = ranked.isEmpty ? "none at this depth" : ranked.joined(separator: " | ")
    if matchCount == 0 {
        return HelperError("ELEMENT_NOT_FOUND", "no menu item at path \(asked); available: \(available)")
    }
    return HelperError("INVALID_TARGET",
                       "\(matchCount) menu items share the path \(asked) and more than one is enabled, "
                         + "so pressing one would be a guess; available: \(available)")
}

/// Which element a snapshot walks from: an explicit `root` ref, an explicit `window` CGWindowID, or
/// the focused window. `main` and a sole window are the only fallbacks — picking one of several
/// unfocused windows would be exactly the guess this helper refuses to make everywhere else.
private func axRoot(_ r: [String: Any], _ pid: pid_t) throws -> (AXUIElement, Ax.Window?, String) {
    if let ref = try optStr(r, "root") {
        guard let parsed = AxRefs.parse(ref) else { throw badArgs("'root' must look like @a<generation>e<index>") }
        guard let entry = AxRefs.shared.get(ref) else {
            throw HelperError("ELEMENT_STALE",
                              "root ref '\(ref)' is from snapshot generation \(parsed.generation); "
                                + "the current generation is \(AxRefs.shared.generation)")
        }
        return (entry.element, nil, "root")
    }
    let windows = try Ax.windows(pid)
    if let wanted = try intArg(r, "window", 0, axMaxWindowId) {
        guard let match = windows.first(where: { ($0.row["windowId"] as? Int) == wanted }) else {
            let available = windows.compactMap { $0.row["windowId"] as? Int }
            throw HelperError("WINDOW_NOT_FOUND",
                              "pid \(pid) has no AX window with id \(wanted) (available: \(available))")
        }
        return (match.element, match, "requested")
    }
    if let focused = windows.first(where: { ($0.row["focused"] as? Bool) == true }) {
        return (focused.element, focused, "focused")
    }
    if let main = windows.first(where: { ($0.row["main"] as? Bool) == true }) {
        return (main.element, main, "main")
    }
    if windows.count == 1 { return (windows[0].element, windows[0], "only") }
    throw HelperError("WINDOW_NOT_FOUND",
                      "pid \(pid) has no focused, main or single AX window (\(windows.count) window(s))")
}

func axRectStable(_ a: CGRect, _ b: CGRect) -> Bool {
    abs(a.origin.x - b.origin.x) <= axRectTolerance && abs(a.origin.y - b.origin.y) <= axRectTolerance
        && abs(a.size.width - b.size.width) <= axRectTolerance
        && abs(a.size.height - b.size.height) <= axRectTolerance
}

/// `Int(_: Double)` TRAPS on a finite value outside `Int64` — and a trap exits 133 and takes the
/// whole session down, which this helper's iron rule forbids. `Ax.rect` guards `.isFinite` but not
/// magnitude, so a hostile or merely broken application could hand us 1e19 and kill the agent from
/// inside an error message. Clamping keeps the diagnostic readable and the process alive; the same
/// discipline `int32Arg`, `sleepMs` and `pidArg` already follow.
private func intText(_ v: Double) -> String {
    guard v.isFinite else { return v.isNaN ? "NaN" : (v > 0 ? "+inf" : "-inf") }
    return String(Int(clamp(v, -9.0e18, 9.0e18)))
}

func rectText(_ r: CGRect) -> String {
    "\(intText(r.origin.x)),\(intText(r.origin.y)) \(intText(r.size.width))x\(intText(r.size.height))"
}

func ptText(_ p: CGPoint) -> String { "\(intText(p.x)),\(intText(p.y))" }

// MARK: - Dispatch

func handle(_ verb: String, _ r: [String: Any]) throws -> [String: Any] {
    let mods = try Mods.parse(r)
    if let out = try handleSystem(verb, r) { return out }
    if let out = try handleAx(verb, r) { return out }
    if let out = try handlePointer(verb, r, mods) { return out }
    if let out = try handleKeyboard(verb, r, mods) { return out }
    throw HelperError("INVALID_VERB", "unknown verb '\(verb)'")
}

func handleSystem(_ verb: String, _ r: [String: Any]) throws -> [String: Any]? {
    switch verb {
    case "permissions":
        return Sys.permissions(prompt: (r["prompt"] as? Bool) ?? false)
    case "windows":
        return ["windows": Sys.windows(pid: try pidArg(r), all: (r["all"] as? Bool) ?? false)]
    case "focus":
        return ["frontmost": try Sys.focus(pid: try pidArg(r))]
    case "frontmost":
        return ["frontmost": Sys.isFrontmost(pid: try pidArg(r))]
    case "version":
        return ["version": helperVersion, "protocol": helperProtocol]
    case "layout":
        return Sys.keyboardLayout()
    default:
        return nil
    }
}

func handlePointer(_ verb: String, _ r: [String: Any], _ mods: Set<String>) throws -> [String: Any]? {
    let e = Engine.shared
    switch verb {
    case "move":
        let target = try point(r)
        try e.move(to: target, durationMs: try msArg(r, "durationMs", 0),
                   steps: try intArg(r, "steps", 1, maxSteps), mods: mods)
        return ["cursor": ptJSON(target)]
    case "down", "up":
        let p = try point(r)
        let cs = Int64(try intArg(r, "clickState", 1, 3) ?? 1)
        let button = try str(r, "button")
        if verb == "down" { try e.down(button, at: p, clickState: cs, mods: mods) }
        else { try e.up(button, at: p, clickState: cs, mods: mods) }
        return ["cursor": ptJSON(p)]
    case "click":
        let at = try point(r)
        try e.click(try str(r, "button"), at: at, count: try intArg(r, "count", 1, 3) ?? 1,
                    intervalMs: try msArg(r, "intervalMs", 80), mods: mods)
        return ["cursor": ptJSON(at)]
    case "path":
        guard let raw = r["points"] as? [[String: Any]], !raw.isEmpty else {
            throw badArgs("'points' must be a non-empty array of {x,y}")
        }
        let pts = try raw.map(point)
        try e.path(try str(r, "button"), points: pts,
                   durationMs: try msArg(r, "durationMs", 0), holdMs: try msArg(r, "holdMs", 0),
                   dwellMs: try msArg(r, "dwellMs", 0), mods: mods)
        return ["cursor": ptJSON(pts[pts.count - 1])]
    case "scroll":
        let sp = try point(r)
        try e.scroll(at: sp, dy: try int32Arg(try num(r, "dy"), "dy"),
                     dx: try int32Arg(try optNum(r, "dx") ?? 0, "dx"), mods: mods)
        return ["cursor": ptJSON(sp)]
    default:
        return nil
    }
}

func handleKeyboard(_ verb: String, _ r: [String: Any], _ mods: Set<String>) throws -> [String: Any]? {
    let e = Engine.shared
    switch verb {
    case "keydown":
        try e.keyDown(try str(r, "key"), mods: mods)
        return ["heldMods": Array(e.heldMods)]
    case "keyup":
        try e.keyUp(try str(r, "key"), mods: mods)
        return ["heldMods": Array(e.heldMods)]
    case "press":
        try e.press(try str(r, "key"), mods: mods)
        return [:]
    case "type":
        try e.type(try str(r, "text"), perCharMs: try msArg(r, "perCharMs", 8))
        return [:]
    case "release_all":
        return e.releaseAll(osState: (r["osState"] as? Bool) ?? false, force: forceSet(r, e))
    case "cursor":
        return ptJSON(e.cursor())
    default:
        return nil
    }
}

/// Canonical button and modifier names out of a `release_all` `force` list.
///
/// A name that is neither is DROPPED, not rejected: `release_all` is the recovery verb, and
/// failing the whole request over one bad name would leave genuinely held input down — the same
/// reason every numeric argument is clamped instead of trapped.
func forceSet(_ r: [String: Any], _ e: Engine) -> Set<String> {
    guard let list = r["force"] as? [String] else { return [] }
    return Set(list.compactMap { name -> String? in
        if let button = try? e.canonicalButton(name) { return button }
        if let mod = Mods.canonicalOrNil(name) { return mod }
        // An ordinary key name passes through lowercased; `releaseAll` looks it up in `Keys.map`
        // and ignores anything that is not there, so an unknown name is still dropped.
        let lowered = name.lowercased()
        return Keys.map[lowered] != nil ? lowered : nil
    })
}

func ptJSON(_ p: CGPoint) -> [String: Any] { ["x": p.x, "y": p.y] }

// MARK: - Entry points

func selftest() -> Never {
    let perms = Sys.permissions(prompt: false)
    let pid = ProcessInfo.processInfo.processIdentifier
    let engine = Engine.shared
    let before = engine.cursor()
    let target = CGPoint(x: before.x + 1, y: before.y + 1)
    var moveOk = false
    var moveError: String? = nil
    var after = before
    do {
        try engine.move(to: target, durationMs: 0, steps: 1, mods: [])
        usleep(80_000)
        after = engine.cursor()
        moveOk = abs(after.x - target.x) <= 1.0 && abs(after.y - target.y) <= 1.0
        try? engine.move(to: before, durationMs: 0, steps: 1, mods: [])
    } catch let err as HelperError {
        moveError = "\(err.code): \(err.message)"
    } catch {
        moveError = "\(error)"
    }
    let ok = ((perms["accessibility"] as? Bool) ?? false) && moveOk
    var out: [String: Any] = [
        "ok": ok,
        "version": helperVersion,
        "protocol": helperProtocol,
        "pid": Int(pid),
        "permissions": perms,
        "windows": Sys.windows(pid: pid, all: false),
        "cursor": ["before": ptJSON(before), "target": ptJSON(target), "after": ptJSON(after)],
        "moveOk": moveOk,
    ]
    if let moveError { out["moveError"] = moveError }
    writeJSON(out)
    exit(ok ? 0 : 1)
}

func serve() {
    writeErr("ready v\(helperVersion) pid=\(ProcessInfo.processInfo.processIdentifier)")
    while let line = readLine(strippingNewline: true) {
        if line.trimmingCharacters(in: .whitespaces).isEmpty { continue }
        var id: Any = NSNull()
        do {
            guard let data = line.data(using: .utf8),
                  let req = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                throw badArgs("request is not a JSON object")
            }
            id = req["id"] ?? NSNull()
            let result = try handle(try str(req, "verb"), req)
            writeJSON(["id": id, "ok": true, "result": result])
        } catch let err as HelperError {
            writeJSON(["id": id, "ok": false, "code": err.code, "message": err.message])
        } catch {
            writeJSON(["id": id, "ok": false, "code": "INTERNAL", "message": "\(error)"])
        }
    }
    // EOF is a shutdown, not a crash: release what this helper pressed, never the user's own.
    _ = Engine.shared.releaseAll(osState: false)
}

let argv = CommandLine.arguments
if argv.contains("--version") {
    writeJSON(["version": helperVersion, "protocol": helperProtocol])
    exit(0)
} else if argv.contains("--selftest") {
    selftest()
} else {
    serve()
}
