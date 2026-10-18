// Posts real CGEvents and prints the CLOCK_UPTIME_RAW time of each post, so the app's own
// `[spike]` log (same clock) gives OS-delivery and post-to-present latency.
//   swift tools/inject.swift drag  x0 y0 x1 y1 steps interval_ms
//   swift tools/inject.swift scroll x y steps dy_px interval_ms        (trackpad-style pixel scroll)
//   swift tools/inject.swift wheel x y steps dy_lines interval_ms      (mouse wheel, line units)
import CoreGraphics
import Foundation

func now() -> UInt64 { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) }
func post(_ e: CGEvent?) { e?.post(tap: .cghidEventTap) }
let a = CommandLine.arguments
let f = { (i: Int) in Double(a[i])! }

switch a[1] {
case "drag":
    let (x0, y0, x1, y1, steps, ms) = (f(2), f(3), f(4), f(5), Int(a[6])!, f(7))
    let p0 = CGPoint(x: x0, y: y0)
    post(CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: p0, mouseButton: .left))
    usleep(50_000)
    post(CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: p0, mouseButton: .left))
    usleep(50_000)
    for i in 1...steps {
        let t = Double(i) / Double(steps)
        let p = CGPoint(x: x0 + (x1 - x0) * t, y: y0 + (y1 - y0) * t)
        let ts = now()
        post(CGEvent(mouseEventSource: nil, mouseType: .leftMouseDragged, mouseCursorPosition: p, mouseButton: .left))
        print("post drag \(i) uptime_ns=\(ts)")
        usleep(useconds_t(ms * 1000))
    }
    post(CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: CGPoint(x: x1, y: y1), mouseButton: .left))
case "scroll":
    let (x, y, steps, dy, ms) = (f(2), f(3), Int(a[4])!, Int32(a[5])!, f(6))
    post(CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: CGPoint(x: x, y: y), mouseButton: .left))
    usleep(50_000)
    for i in 0..<steps {
        let e = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2, wheel1: dy, wheel2: 0, wheel3: 0)
        e?.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
        // CGScrollPhase: 1 began, 2 changed, 4 ended.
        e?.setIntegerValueField(.scrollWheelEventScrollPhase, value: i == 0 ? 1 : 2)
        let ts = now()
        post(e)
        print("post scroll \(i) uptime_ns=\(ts)")
        usleep(useconds_t(ms * 1000))
    }
    let end = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2, wheel1: 0, wheel2: 0, wheel3: 0)
    end?.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
    end?.setIntegerValueField(.scrollWheelEventScrollPhase, value: 4)
    post(end)
case "wheel":
    let (x, y, steps, dy, ms) = (f(2), f(3), Int(a[4])!, Int32(a[5])!, f(6))
    post(CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: CGPoint(x: x, y: y), mouseButton: .left))
    usleep(50_000)
    for i in 0..<steps {
        let ts = now()
        post(CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1, wheel1: dy, wheel2: 0, wheel3: 0))
        print("post wheel \(i) uptime_ns=\(ts)")
        usleep(useconds_t(ms * 1000))
    }
default:
    print("unknown mode")
}
