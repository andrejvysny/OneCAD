import UIKit

/// Two-finger pan, pinch and rotation recognized natively and merged into one
/// navigation session: BEGAN when the first recognizer starts, MOVED for every
/// delta, ENDED (or CANCELLED) when the last one finishes.
///
/// Recognizers accept direct (finger) touches only and never cancel or delay
/// the view's touches, so Pencil contacts are untouched and Rust still sees
/// the raw finger contacts for its own arbitration.
@MainActor
final class NavigationGestures: NSObject, UIGestureRecognizerDelegate {
    private weak var view: UIView?
    private let submit: (onecad_ios_navigation) -> Void
    private var active: Set<ObjectIdentifier> = []
    private var sessionCancelled = false

    init(view: UIView, submit: @escaping (onecad_ios_navigation) -> Void) {
        self.view = view
        self.submit = submit
        super.init()
        let pan = UIPanGestureRecognizer(target: self, action: #selector(handle(_:)))
        pan.minimumNumberOfTouches = 2
        pan.maximumNumberOfTouches = 2
        let recognizers: [UIGestureRecognizer] = [
            pan,
            UIPinchGestureRecognizer(target: self, action: #selector(handle(_:))),
            UIRotationGestureRecognizer(target: self, action: #selector(handle(_:))),
        ]
        for recognizer in recognizers {
            recognizer.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.direct.rawValue)]
            recognizer.cancelsTouchesInView = false
            recognizer.delaysTouchesBegan = false
            recognizer.delaysTouchesEnded = false
            recognizer.delegate = self
            view.addGestureRecognizer(recognizer)
        }
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
    ) -> Bool {
        true
    }

    @objc private func handle(_ recognizer: UIGestureRecognizer) {
        guard let view else { return }
        var nav = onecad_ios_navigation()
        let anchor = recognizer.location(in: view)
        nav.anchor_x_pt = Double(anchor.x)
        nav.anchor_y_pt = Double(anchor.y)
        nav.scale_delta = 1
        takeDelta(from: recognizer, in: view, into: &nav)
        guard let phase = sessionPhase(for: recognizer) else { return }
        nav.phase = phase.rawValue
        submit(nav)
    }

    /// Reads and resets the recognizer so each event carries an increment.
    private func takeDelta(from recognizer: UIGestureRecognizer, in view: UIView,
                           into nav: inout onecad_ios_navigation) {
        switch recognizer {
        case let pan as UIPanGestureRecognizer:
            let t = pan.translation(in: view)
            nav.pan_dx_pt = Double(t.x)
            nav.pan_dy_pt = Double(t.y)
            pan.setTranslation(.zero, in: view)
        case let pinch as UIPinchGestureRecognizer:
            nav.scale_delta = Double(pinch.scale)
            pinch.scale = 1
        case let rotation as UIRotationGestureRecognizer:
            nav.rotation_delta_rad = Double(rotation.rotation)
            rotation.rotation = 0
        default:
            break
        }
    }

    private func sessionPhase(for recognizer: UIGestureRecognizer) -> onecad_ios_phase? {
        let key = ObjectIdentifier(recognizer)
        switch recognizer.state {
        case .began:
            let first = active.isEmpty
            active.insert(key)
            if first { sessionCancelled = false }
            return first ? ONECAD_IOS_PHASE_BEGAN : ONECAD_IOS_PHASE_MOVED
        case .changed:
            return active.contains(key) ? ONECAD_IOS_PHASE_MOVED : nil
        case .ended, .cancelled, .failed:
            guard active.remove(key) != nil else { return nil }
            if recognizer.state != .ended { sessionCancelled = true }
            guard active.isEmpty else { return ONECAD_IOS_PHASE_MOVED }
            return sessionCancelled ? ONECAD_IOS_PHASE_CANCELLED : ONECAD_IOS_PHASE_ENDED
        default:
            return nil
        }
    }
}
