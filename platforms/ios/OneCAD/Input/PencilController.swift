import UIKit

/// Tracks capabilities as they are observed at runtime and reports changes,
/// together with the user's Pencil preferences from Settings.
@MainActor
final class CapabilityTracker {
    private var flags: UInt32 = 0
    private var lastSent: onecad_ios_capabilities?
    private let send: (onecad_ios_capabilities) -> Void

    init(send: @escaping (onecad_ios_capabilities) -> Void) {
        self.send = send
    }

    func observe(_ newFlags: UInt32) {
        guard flags | newFlags != flags else { return }
        flags |= newFlags
        publish()
    }

    /// Preferences can change in Settings while the app is in the background.
    func publish() {
        var caps = onecad_ios_capabilities()
        caps.flags = flags
        caps.prefers_pencil_only_drawing = UIPencilInteraction.prefersPencilOnlyDrawing ? 1 : 0
        caps.preferred_tap_action = Self.preferred(UIPencilInteraction.preferredTapAction).rawValue
        caps.preferred_squeeze_action = Self.preferred(UIPencilInteraction.preferredSqueezeAction).rawValue
        if let lastSent, lastSent.flags == caps.flags,
           lastSent.prefers_pencil_only_drawing == caps.prefers_pencil_only_drawing,
           lastSent.preferred_tap_action == caps.preferred_tap_action,
           lastSent.preferred_squeeze_action == caps.preferred_squeeze_action {
            return
        }
        lastSent = caps
        send(caps)
    }

    static func preferred(_ action: UIPencilPreferredAction) -> onecad_ios_preferred_action {
        switch action {
        case .ignore: ONECAD_IOS_PREF_IGNORE
        case .switchEraser: ONECAD_IOS_PREF_SWITCH_ERASER
        case .switchPrevious: ONECAD_IOS_PREF_SWITCH_PREVIOUS
        case .showColorPalette: ONECAD_IOS_PREF_SHOW_COLOR_PALETTE
        case .showInkAttributes: ONECAD_IOS_PREF_SHOW_INK_ATTRIBUTES
        case .showContextualPalette: ONECAD_IOS_PREF_SHOW_CONTEXTUAL_PALETTE
        case .runSystemShortcut: ONECAD_IOS_PREF_RUN_SYSTEM_SHORTCUT
        @unknown default: ONECAD_IOS_PREF_IGNORE
        }
    }
}

/// Barrel double-tap and Pencil Pro squeeze through `UIPencilInteraction`.
/// The deployment target (iPadOS 18) always has the iOS 17.5 tap/squeeze API,
/// so the deprecated `pencilInteractionDidTap(_:)` is not implemented.
@MainActor
final class PencilController: NSObject, UIPencilInteractionDelegate {
    private let submit: (onecad_ios_pencil_event) -> Void
    private let capabilities: CapabilityTracker

    init(view: UIView, capabilities: CapabilityTracker,
         submit: @escaping (onecad_ios_pencil_event) -> Void) {
        self.capabilities = capabilities
        self.submit = submit
        super.init()
        view.addInteraction(UIPencilInteraction(delegate: self))
    }

    func pencilInteraction(_ interaction: UIPencilInteraction,
                           didReceiveTap tap: UIPencilInteraction.Tap) {
        capabilities.observe(ONECAD_IOS_CAP_BARREL_TAP | ONECAD_IOS_CAP_PENCIL_CONNECTED)
        var event = makeEvent(action: ONECAD_IOS_PENCIL_BARREL_DOUBLE_TAP,
                              preferred: UIPencilInteraction.preferredTapAction,
                              pose: tap.hoverPose, timestamp: tap.timestamp)
        event.phase = ONECAD_IOS_PHASE_ENDED.rawValue
        submit(event)
    }

    func pencilInteraction(_ interaction: UIPencilInteraction,
                           didReceiveSqueeze squeeze: UIPencilInteraction.Squeeze) {
        capabilities.observe(ONECAD_IOS_CAP_SQUEEZE | ONECAD_IOS_CAP_PENCIL_CONNECTED)
        var event = makeEvent(action: ONECAD_IOS_PENCIL_SQUEEZE,
                              preferred: UIPencilInteraction.preferredSqueezeAction,
                              pose: squeeze.hoverPose, timestamp: squeeze.timestamp)
        event.phase = Self.phase(squeeze.phase).rawValue
        submit(event)
    }

    private static func phase(_ phase: UIPencilInteraction.Phase) -> onecad_ios_phase {
        switch phase {
        case .began: ONECAD_IOS_PHASE_BEGAN
        case .changed: ONECAD_IOS_PHASE_MOVED
        case .ended: ONECAD_IOS_PHASE_ENDED
        case .cancelled: ONECAD_IOS_PHASE_CANCELLED
        @unknown default: ONECAD_IOS_PHASE_CANCELLED
        }
    }

    /// The hover pose is absent when the Pencil is out of hover range or the
    /// device has no hover; then only the action is reported.
    private func makeEvent(action: onecad_ios_pencil_action, preferred: UIPencilPreferredAction,
                           pose: UIPencilHoverPose?, timestamp: TimeInterval) -> onecad_ios_pencil_event {
        var event = onecad_ios_pencil_event()
        event.action = action.rawValue
        event.preferred = CapabilityTracker.preferred(preferred).rawValue
        event.timestamp_s = timestamp
        // Pose coordinates are already in the interaction's view (the canvas).
        guard let pose else { return event }
        let location = pose.location
        event.present = ONECAD_IOS_HAS_LOCATION | ONECAD_IOS_HAS_HOVER_Z
            | ONECAD_IOS_HAS_ALTITUDE | ONECAD_IOS_HAS_AZIMUTH
        event.x_pt = Double(location.x)
        event.y_pt = Double(location.y)
        event.hover_z = Double(pose.zOffset)
        event.altitude_rad = Double(pose.altitudeAngle)
        event.azimuth_rad = Double(pose.azimuthAngle)
        if pose.rollAngle != 0 {
            event.present |= ONECAD_IOS_HAS_ROLL
            event.roll_rad = Double(pose.rollAngle)
        }
        return event
    }
}
