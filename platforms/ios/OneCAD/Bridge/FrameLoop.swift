import QuartzCore
import UIKit
import os

/// Runs one Rust frame per display refresh, but only while a frame is needed.
///
/// CADisplayLink is used rather than UIUpdateLink: wgpu presents drawables
/// itself, and the display link's pause/resume semantics with an externally
/// presented CAMetalLayer are well understood. The link is paused whenever no
/// frame is pending, so an idle editor costs no refresh callbacks.
@MainActor
final class FrameLoop {
    private var link: CADisplayLink?
    private var repaintTimer: Timer?
    private(set) var needsFrame = false
    /// True while backgrounded or terminal: Metal work is not allowed then.
    private var suspended = false
    private let runFrame: () -> Void
    private var refreshInterval: CFTimeInterval = 1.0 / 60.0
    var onDisplayLink: ((CFTimeInterval, CFTimeInterval, CFTimeInterval) -> Void)?

    init(runFrame: @escaping () -> Void) {
        self.runFrame = runFrame
    }

    func start() {
        guard link == nil else { return }
        let link = CADisplayLink(target: DisplayLinkProxy(owner: self),
                                 selector: #selector(DisplayLinkProxy.tick(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 60, preferred: 60)
        link.isPaused = !needsFrame || suspended
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    /// Must be called before the owner goes away; the link retains its target.
    func stop() {
        link?.invalidate()
        link = nil
        repaintTimer?.invalidate()
        repaintTimer = nil
        needsFrame = false
    }

    func requestFrame() {
        needsFrame = true
        if !suspended { link?.isPaused = false }
    }

    /// Honors `repaint_after_s`: 0 means next refresh, > 0 a one-shot timer so
    /// the display link stays paused in between (e.g. caret blink).
    func scheduleRepaint(after seconds: Double) {
        repaintTimer?.invalidate()
        repaintTimer = nil
        guard seconds.isFinite, seconds >= 0 else { return }
        // A short retry belongs to the outstanding display refresh, not a timer
        // that would fire after it and postpone work by another refresh.
        if needsFrame || seconds <= refreshInterval + 0.001 {
            requestFrame()
            return
        }
        let timer = Timer(timeInterval: seconds, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.requestFrame() }
        }
        repaintTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    func setSuspended(_ value: Bool) {
        suspended = value
        link?.isPaused = value || !needsFrame
    }

    func tick(timestamp: CFTimeInterval = 0, targetTimestamp: CFTimeInterval = 0) {
        guard needsFrame, !suspended else {
            link?.isPaused = true
            return
        }
        // Cleared first so a wake or input arriving during the frame re-arms it.
        needsFrame = false
        let period = targetTimestamp - timestamp
        if period.isFinite, period > 0, period < 1 { refreshInterval = period }
        onDisplayLink?(CACurrentMediaTime(), timestamp, targetTimestamp)
        runFrame()
        if !needsFrame { link?.isPaused = true }
    }
}

/// Breaks the CADisplayLink -> target retain cycle.
@MainActor
private final class DisplayLinkProxy: NSObject {
    weak var owner: FrameLoop?

    init(owner: FrameLoop) {
        self.owner = owner
    }

    @objc func tick(_ link: CADisplayLink) {
        owner?.tick(timestamp: link.timestamp, targetTimestamp: link.targetTimestamp)
    }
}

/// Host clocks stay in their own monotonic domain; Rust assigns trace arrival time.
@MainActor
enum HostPerformance {
    static let enabled = ProcessInfo.processInfo.environment["ONECAD_PERF_TRACE"] == "1"
    private static let log = OSLog(subsystem: "com.onecad.ipad", category: "performance")

    struct Interval {
        let start: CFTimeInterval
        let name: StaticString
        let signpost: OSSignpostID
    }

    static func nanoseconds(_ seconds: Double) -> UInt64 {
        guard seconds.isFinite, seconds > 0 else { return 0 }
        return UInt64(min(seconds * 1_000_000_000, Double(UInt64.max / 2)))
    }

    static func begin(_ name: StaticString) -> Interval? {
        guard enabled else { return nil }
        let signpost = OSSignpostID(log: log)
        os_signpost(.begin, log: log, name: name, signpostID: signpost)
        return Interval(start: CACurrentMediaTime(), name: name, signpost: signpost)
    }

    static func end(_ interval: Interval?, bridge: RustBridge, stage: UInt32,
                    correlation: UInt64, values: (UInt64, UInt64, UInt64) = (0, 0, 0)) {
        guard let interval else { return }
        let duration = nanoseconds(CACurrentMediaTime() - interval.start)
        os_signpost(.end, log: log, name: interval.name, signpostID: interval.signpost)
        bridge.recordPerformance(stage: stage, correlation: correlation, duration: duration, values: values)
    }
}
