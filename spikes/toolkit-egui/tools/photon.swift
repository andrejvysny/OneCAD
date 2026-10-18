// Input-to-photon proxy. Captures the app window with ScreenCaptureKit, posts one
// Shift+right-drag step (OneCAD's orbit gesture) at a time, and reports, per step, the delay
// from the CGEvent post to the display time of the first captured frame whose viewport pixels
// differ from the last frame before the post. Both clocks are mach host time
// (CLOCK_UPTIME_RAW), so no cross-process clock join is needed. `displayTime` is when the
// compositor put the frame on screen: scanout of the panel is not included.
//
// usage: photon <pid> <steps> <gapMs> <dx> <vx> <vy> <vw> <vh>
//   v* = viewport rect in window points (sampled region); the drag starts at its center.
// build: swiftc -O -parse-as-library photon.swift -o photon
import AppKit
import CoreGraphics
import CoreMedia
import Foundation
import ScreenCaptureKit

final class Sink: NSObject, SCStreamOutput, @unchecked Sendable {
    let lock = NSLock()
    var frames: [(UInt64, UInt64)] = []  // (display ns, viewport hash)
    let region: CGRect
    let timebase: mach_timebase_info_data_t

    init(region: CGRect) {
        self.region = region
        var tb = mach_timebase_info_data_t()
        mach_timebase_info(&tb)
        timebase = tb
    }

    func stream(_ s: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen,
              let atts = CMSampleBufferGetSampleAttachmentsArray(sb, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let info = atts.first,
              let raw = info[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete,
              let display = info[.displayTime] as? UInt64,
              let px = CMSampleBufferGetImageBuffer(sb) else { return }
        CVPixelBufferLockBaseAddress(px, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(px, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(px) else { return }
        let w = CVPixelBufferGetWidth(px), h = CVPixelBufferGetHeight(px)
        let stride = CVPixelBufferGetBytesPerRow(px)
        let p = base.assumingMemoryBound(to: UInt8.self)
        // FNV-1a over a 96x64 sample grid of the viewport region.
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for j in 0..<64 {
            let y = Int(region.minY + region.height * (Double(j) + 0.5) / 64)
            for i in 0..<96 {
                let x = Int(region.minX + region.width * (Double(i) + 0.5) / 96)
                guard x < w, y < h else { continue }
                let o = y * stride + x * 4
                for k in 0..<3 { hash = (hash ^ UInt64(p[o + k])) &* 0x100_0000_01b3 }
            }
        }
        let ns = display * UInt64(timebase.numer) / UInt64(timebase.denom)
        lock.withLock { frames.append((ns, hash)) }
    }

    func snapshot() -> [(UInt64, UInt64)] { lock.withLock { frames } }
}

func post(_ type: CGEventType, _ p: CGPoint, _ src: CGEventSource?) -> UInt64 {
    let e = CGEvent(mouseEventSource: src, mouseType: type, mouseCursorPosition: p, mouseButton: .right)!
    e.flags = .maskShift
    let t = clock_gettime_nsec_np(CLOCK_UPTIME_RAW)
    e.post(tap: .cghidEventTap)
    return t
}

@main
struct Photon {
    static func main() async throws {
        let a = CommandLine.arguments
        guard a.count == 9, let pid = Int32(a[1]), let steps = Int(a[2]), let gap = Double(a[3]),
              let dx = Double(a[4]), let vx = Double(a[5]), let vy = Double(a[6]),
              let vw = Double(a[7]), let vh = Double(a[8]) else {
            FileHandle.standardError.write("usage: photon pid steps gapMs dx vx vy vw vh\n".data(using: .utf8)!)
            exit(2)
        }
        // ScreenCaptureKit asserts without a WindowServer connection (CGS_REQUIRE_INIT).
        _ = await MainActor.run { NSApplication.shared }
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard let win = content.windows.first(where: { $0.owningApplication?.processID == pid && $0.frame.width > 400 }) else {
            print("[photon] window for pid \(pid) not found"); exit(3)
        }
        let cfg = SCStreamConfiguration()
        cfg.width = Int(win.frame.width)
        cfg.height = Int(win.frame.height)
        cfg.minimumFrameInterval = CMTime(value: 1, timescale: 240)
        cfg.queueDepth = 8
        cfg.pixelFormat = kCVPixelFormatType_32BGRA
        cfg.showsCursor = false
        let sink = Sink(region: CGRect(x: vx, y: vy, width: vw, height: vh))
        let stream = SCStream(filter: SCContentFilter(desktopIndependentWindow: win), configuration: cfg, delegate: nil)
        try stream.addStreamOutput(sink, type: .screen, sampleHandlerQueue: DispatchQueue(label: "photon"))
        try await stream.startCapture()
        print("[photon] window \(win.windowID) frame=\(win.frame) capture=\(cfg.width)x\(cfg.height)")
        let src = CGEventSource(stateID: .hidSystemState)
        var p = CGPoint(x: win.frame.minX + vx + vw / 2, y: win.frame.minY + vy + vh / 2)
        let move = CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left)!
        move.post(tap: .cghidEventTap)
        usleep(300_000)
        let shift = CGEvent(keyboardEventSource: src, virtualKey: 0x38, keyDown: true)!
        shift.flags = .maskShift
        shift.post(tap: .cghidEventTap)
        usleep(60_000)
        _ = post(.rightMouseDown, p, src)
        usleep(400_000)
        var posts: [UInt64] = []
        for _ in 0..<steps {
            p.x += dx
            posts.append(post(.rightMouseDragged, p, src))
            usleep(useconds_t(gap * 1000))
        }
        _ = post(.rightMouseUp, p, src)
        let up = CGEvent(keyboardEventSource: src, virtualKey: 0x38, keyDown: false)!
        up.post(tap: .cghidEventTap)
        usleep(300_000)
        try await stream.stopCapture()
        let frames = sink.snapshot()
        var lat: [Double] = []
        for t in posts {
            guard let before = frames.last(where: { $0.0 <= t }) else { continue }
            if let hit = frames.first(where: { $0.0 > t && $0.1 != before.1 }) {
                lat.append(Double(hit.0 - t) / 1e6)
                print("[photon] post_ns=\(t) display_ns=\(hit.0) ms=\(String(format: "%.2f", Double(hit.0 - t) / 1e6))")
            }
        }
        let s = lat.sorted()
        guard !s.isEmpty else { print("[photon] frames=\(frames.count) matched=0"); exit(4) }
        let q = { (f: Double) in s[Int((Double(s.count - 1) * f).rounded())] }
        let mean = s.reduce(0, +) / Double(s.count)
        print(String(format: "[photon] frames=%d posts=%d matched=%d event_to_display_ms mean=%.2f p50=%.2f p95=%.2f min=%.2f max=%.2f",
                     frames.count, posts.count, s.count, mean, q(0.5), q(0.95), s[0], s[s.count - 1]))
    }
}
