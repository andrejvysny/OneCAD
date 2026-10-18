// Posts scroll events at the cursor: `scroll line <n>` (a mouse wheel: line units) or
// `scroll pixel <n>` (a trackpad-like continuous pixel scroll), 5 events 30 ms apart.
import CoreGraphics
import Foundation
let a = CommandLine.arguments
let units: CGScrollEventUnit = a[1] == "pixel" ? .pixel : .line
let n = Int32(a[2]) ?? 1
for _ in 0..<5 {
    let e = CGEvent(scrollWheelEvent2Source: nil, units: units, wheelCount: 1, wheel1: n, wheel2: 0, wheel3: 0)!
    if units == .pixel { e.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1) }
    e.post(tap: .cghidEventTap)
    usleep(30_000)
}
print("[scroll] \(a[1]) \(n) pid=\(getpid())")
