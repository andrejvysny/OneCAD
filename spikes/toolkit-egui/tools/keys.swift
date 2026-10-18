// Minimal key poster: `keys <code>[:mods] ...` posts each virtual key code as down/up with a
// 40 ms hold and 120 ms gap (mods: c=Command s=Shift o=Option). Codes are ANSI positions.
import CoreGraphics
import Foundation
let src = CGEventSource(stateID: .hidSystemState)
func post(_ code: CGKeyCode, _ down: Bool, _ flags: CGEventFlags) {
    let e = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: down)!
    e.flags = flags
    e.post(tap: .cghidEventTap)
}
let modKeys: [Character: (CGKeyCode, CGEventFlags)] = ["c": (0x37, .maskCommand), "s": (0x38, .maskShift), "o": (0x3A, .maskAlternate)]
for arg in CommandLine.arguments.dropFirst() {
    let parts = arg.split(separator: ":")
    guard let code = CGKeyCode(parts[0]) else { continue }
    var flags: CGEventFlags = []
    let mods = parts.count > 1 ? Array(parts[1]) : []
    for m in mods { if let (k, f) = modKeys[m] { flags.insert(f); post(k, true, flags); usleep(20_000) } }
    post(code, true, flags); usleep(40_000); post(code, false, flags)
    for m in mods.reversed() { if let (k, f) = modKeys[m] { flags.remove(f); usleep(20_000); post(k, false, flags) } }
    print("[keys] posted \(arg) pid=\(getpid()) uptime_ns=\(clock_gettime_nsec_np(CLOCK_UPTIME_RAW))"); fflush(stdout)
    usleep(120_000)
}
