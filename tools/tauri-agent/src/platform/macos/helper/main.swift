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

let helperVersion = "1.1.0"
let helperProtocol = 2

/// Longest sleep any single duration/interval/hold/dwell argument may ask for.
let maxSleepMs = 60_000.0
/// Widest interpolation any one motion verb may run.
let maxSteps = 2_000
/// Widest scroll a single verb may post, in lines.
let maxScrollLines = 100.0

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

    /// Never throws. Releases what this helper is holding; `osState` additionally releases what
    /// the OS still reports held.
    ///
    /// `osState` is for one caller only — the client draining a CRASHED helper, whose tracked
    /// state died with it. On the normal path the OS report includes the USER's own hand on the
    /// mouse and their own held modifiers, and synthesising ups for those fights the human.
    func releaseAll(osState: Bool) -> [String: Any] {
        let p = cursor()
        var buttons: [String] = []
        var mods: [String] = []
        for name in ["left", "right", "middle"] {
            let cg: CGMouseButton = name == "left" ? .left : (name == "right" ? .right : .center)
            let held = heldButtons.contains(name)
                || (osState && CGEventSource.buttonState(.combinedSessionState, button: cg))
            guard held else { continue }
            if (try? up(name, at: p, clickState: 1, mods: [])) != nil { buttons.append(name) }
        }
        heldButtons.removeAll()
        let osFlags: CGEventFlags = osState ? CGEventSource.flagsState(.combinedSessionState) : []
        for name in Mods.all where heldMods.contains(name) || osFlags.contains(Mods.mask(name)) {
            heldMods.remove(name)
            if (try? postKey(Mods.keyCode(name), down: false, flags: flagsNow([]))) != nil { mods.append(name) }
        }
        heldMods.removeAll()
        return ["releasedButtons": buttons, "releasedMods": mods]
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

// MARK: - Dispatch

func handle(_ verb: String, _ r: [String: Any]) throws -> [String: Any] {
    let mods = try Mods.parse(r)
    if let out = try handleSystem(verb, r) { return out }
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
        return e.releaseAll(osState: (r["osState"] as? Bool) ?? false)
    case "cursor":
        return ptJSON(e.cursor())
    default:
        return nil
    }
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
