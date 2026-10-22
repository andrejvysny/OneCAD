import QuartzCore
import UIKit

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

    init(runFrame: @escaping () -> Void) {
        self.runFrame = runFrame
    }

    func start() {
        guard link == nil else { return }
        let link = CADisplayLink(target: DisplayLinkProxy(owner: self),
                                 selector: #selector(DisplayLinkProxy.tick(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 30, maximum: 120, preferred: 120)
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
        guard seconds >= 0 else { return }
        if seconds == 0 {
            requestFrame()
            return
        }
        repaintTimer = Timer.scheduledTimer(withTimeInterval: seconds, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.requestFrame() }
        }
    }

    func setSuspended(_ value: Bool) {
        suspended = value
        link?.isPaused = value || !needsFrame
    }

    fileprivate func tick() {
        guard needsFrame, !suspended else {
            link?.isPaused = true
            return
        }
        // Cleared first so a wake or input arriving during the frame re-arms it.
        needsFrame = false
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
        owner?.tick()
    }
}
