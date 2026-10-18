// Platform floor for photon.swift: a plain AppKit window whose view repaints (a fill whose
// colour follows the pointer X) on every right-mouse drag. Same window size and position as
// the spike, so photon.swift can drive both identically.
// usage: native_baseline   (window at 100,120 top-left, 1520x920)
// build: swiftc -O native_baseline.swift -o native_baseline
import AppKit

final class DragView: NSView {
    var x: CGFloat = 0
    override var acceptsFirstResponder: Bool { true }
    override func rightMouseDragged(with e: NSEvent) {
        x = convert(e.locationInWindow, from: nil).x
        needsDisplay = true
    }
    override func draw(_ r: NSRect) {
        NSColor(hue: (x / bounds.width).truncatingRemainder(dividingBy: 1), saturation: 0.5,
                brightness: 0.5, alpha: 1).setFill()
        bounds.fill()
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let screenH = NSScreen.screens[0].frame.height
let win = NSWindow(contentRect: NSRect(x: 100, y: screenH - 120 - 920, width: 1520, height: 920),
                   styleMask: [.titled, .fullSizeContentView], backing: .buffered, defer: false)
win.titlebarAppearsTransparent = true
win.contentView = DragView()
win.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)
app.run()
