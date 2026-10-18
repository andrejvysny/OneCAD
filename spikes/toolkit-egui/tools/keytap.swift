// Listen-only session event tap: logs every key/mouse-button event with the posting process,
// so a test run on a shared desktop can prove which synthetic events were its own.
// usage: keytap <seconds>
import CoreGraphics
import Darwin
import Foundation
let types: [CGEventType] = [.keyDown, .keyUp, .flagsChanged, .leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp]
let mask = types.reduce(0) { $0 | (1 << $1.rawValue) }
func procName(_ pid: Int64) -> String {
    var buf = [CChar](repeating: 0, count: 4096)
    return proc_pidpath(Int32(pid), &buf, 4096) > 0 ? String(cString: buf).split(separator: "/").last.map(String.init) ?? "?" : "pid\(pid)"
}
let cb: CGEventTapCallBack = { _, type, ev, _ in
    let pid = ev.getIntegerValueField(.eventSourceUnixProcessID)
    print("[tap] type=\(type.rawValue) code=\(ev.getIntegerValueField(.keyboardEventKeycode)) src=\(procName(pid)) uptime_ns=\(clock_gettime_nsec_np(CLOCK_UPTIME_RAW))")
    fflush(stdout)
    return Unmanaged.passUnretained(ev)
}
guard let tap = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .headInsertEventTap, options: .listenOnly,
                                  eventsOfInterest: CGEventMask(mask), callback: cb, userInfo: nil) else { exit(1) }
CFRunLoopAddSource(CFRunLoopGetCurrent(), CFMachPortCreateRunLoopSource(nil, tap, 0), .commonModes)
CGEvent.tapEnable(tap: tap, enable: true)
let secs = Double(CommandLine.arguments.dropFirst().first ?? "60") ?? 60
DispatchQueue.main.asyncAfter(deadline: .now() + secs) { exit(0) }
CFRunLoopRun()
