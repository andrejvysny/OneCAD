// Latency probe injector: posts a Shift+right-button drag (OneCAD's orbit gesture) as
// CGEvents at a fixed cadence and prints each event's CLOCK_UPTIME_RAW post time, so it can be
// joined with the app's `[latency] x=.. prepare_ns=..` lines (same clock, same process host).
//
// usage: inject_drag <startX> <startY> <steps> <intervalMs> <dxPerStep>   (global points)
import CoreGraphics
import Foundation

let a = CommandLine.arguments
guard a.count == 6, let x0 = Double(a[1]), let y0 = Double(a[2]), let steps = Int(a[3]),
      let interval = Double(a[4]), let dx = Double(a[5]) else {
    FileHandle.standardError.write("usage: inject_drag x y steps intervalMs dx\n".data(using: .utf8)!)
    exit(2)
}
let src = CGEventSource(stateID: .hidSystemState)
func post(_ type: CGEventType, _ p: CGPoint, shift: Bool) -> UInt64 {
    let e = CGEvent(mouseEventSource: src, mouseType: type, mouseCursorPosition: p, mouseButton: .right)!
    if shift { e.flags = .maskShift }
    let t = clock_gettime_nsec_np(CLOCK_UPTIME_RAW)
    e.post(tap: .cghidEventTap)
    return t
}
func sleepMs(_ ms: Double) { usleep(useconds_t(ms * 1000)) }

var p = CGPoint(x: x0, y: y0)
_ = post(.mouseMoved, p, shift: false)
sleepMs(150)
// Shift down first so the app sees the modifier with the button press.
let shiftDown = CGEvent(keyboardEventSource: src, virtualKey: 0x38, keyDown: true)!
shiftDown.flags = .maskShift
shiftDown.post(tap: .cghidEventTap)
sleepMs(50)
_ = post(.rightMouseDown, p, shift: true)
sleepMs(100)
for i in 0..<steps {
    p.x += dx
    let t = post(.rightMouseDragged, p, shift: true)
    print("[inject] i=\(i) x=\(p.x) y=\(p.y) post_ns=\(t)")
    sleepMs(interval)
}
_ = post(.rightMouseUp, p, shift: true)
let shiftUp = CGEvent(keyboardEventSource: src, virtualKey: 0x38, keyDown: false)!
shiftUp.post(tap: .cghidEventTap)
