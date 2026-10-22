import UIKit

/// Copies UITouch state into `onecad_ios_touch_sample` values. No UIKit object
/// survives past the event callback.
@MainActor
struct TouchSampleBuilder {
    /// Raised flags the capability tracker should learn about.
    private(set) var observedCapabilities: UInt32 = 0

    static func phase(_ phase: UITouch.Phase) -> onecad_ios_phase {
        switch phase {
        case .began: ONECAD_IOS_PHASE_BEGAN
        case .moved: ONECAD_IOS_PHASE_MOVED
        case .stationary: ONECAD_IOS_PHASE_STATIONARY
        case .ended: ONECAD_IOS_PHASE_ENDED
        case .cancelled: ONECAD_IOS_PHASE_CANCELLED
        case .regionEntered, .regionMoved: ONECAD_IOS_PHASE_HOVER_MOVED
        case .regionExited: ONECAD_IOS_PHASE_HOVER_EXITED
        @unknown default: ONECAD_IOS_PHASE_MOVED
        }
    }

    static func kind(_ type: UITouch.TouchType) -> onecad_ios_contact_kind {
        switch type {
        case .pencil: ONECAD_IOS_CONTACT_PENCIL
        case .indirectPointer, .indirect: ONECAD_IOS_CONTACT_INDIRECT_POINTER
        case .direct: ONECAD_IOS_CONTACT_FINGER
        @unknown default: ONECAD_IOS_CONTACT_FINGER
        }
    }

    static func propertyMask(_ properties: UITouch.Properties) -> UInt32 {
        var mask: UInt32 = 0
        if properties.contains(.force) { mask |= ONECAD_IOS_HAS_FORCE }
        if properties.contains(.altitude) { mask |= ONECAD_IOS_HAS_ALTITUDE }
        if properties.contains(.azimuth) { mask |= ONECAD_IOS_HAS_AZIMUTH }
        if properties.contains(.location) { mask |= ONECAD_IOS_HAS_LOCATION }
        if properties.contains(.roll) { mask |= ONECAD_IOS_HAS_ROLL }
        return mask
    }

    /// One sample for `touch` (actual, coalesced or predicted), attributed to
    /// `contact`. Optional fields are present only when the hardware has them.
    mutating func sample(
        _ touch: UITouch, contact: UInt64, provenance: onecad_ios_provenance,
        phase: onecad_ios_phase, in view: UIView, registry: inout ContactRegistry
    ) -> onecad_ios_touch_sample {
        var s = onecad_ios_touch_sample()
        s.contact_id = contact
        s.kind = Self.kind(touch.type).rawValue
        s.phase = phase.rawValue
        s.provenance = provenance.rawValue
        s.timestamp_s = touch.timestamp
        let location = touch.preciseLocation(in: view)
        s.x_pt = Double(location.x)
        s.y_pt = Double(location.y)
        s.tap_count = UInt32(clamping: touch.tapCount)
        s.estimated = Self.propertyMask(touch.estimatedProperties)
        s.expects_update = Self.propertyMask(touch.estimatedPropertiesExpectingUpdates)
        s.present = ONECAD_IOS_HAS_LOCATION
        fillForce(&s, touch, view: view)
        if touch.type == .pencil { fillPencilPose(&s, touch, view: view, registry: &registry) }
        if let index = touch.estimationUpdateIndex?.intValue {
            s.present |= ONECAD_IOS_HAS_ESTIMATION_INDEX
            s.estimation_index = UInt64(clamping: index)
        }
        return s
    }

    /// Force only when the hardware measures it: Pencil with a nonzero range,
    /// or a finger on force-touch hardware.
    private mutating func fillForce(_ s: inout onecad_ios_touch_sample, _ touch: UITouch, view: UIView) {
        let fingerForce = touch.type == .direct && view.traitCollection.forceTouchCapability == .available
        guard (touch.type == .pencil || fingerForce), touch.maximumPossibleForce > 0 else { return }
        s.present |= ONECAD_IOS_HAS_FORCE
        s.force = Double(touch.force)
        s.max_force = Double(touch.maximumPossibleForce)
        if touch.type == .pencil { observedCapabilities |= ONECAD_IOS_CAP_PRESSURE }
    }

    private mutating func fillPencilPose(
        _ s: inout onecad_ios_touch_sample, _ touch: UITouch, view: UIView,
        registry: inout ContactRegistry
    ) {
        observedCapabilities |= ONECAD_IOS_CAP_PENCIL_CONNECTED
        s.present |= ONECAD_IOS_HAS_ALTITUDE | ONECAD_IOS_HAS_AZIMUTH
        s.altitude_rad = Double(touch.altitudeAngle)
        s.azimuth_rad = Double(touch.azimuthAngle(in: view))
        if registry.reportsRoll(s.contact_id, roll: touch.rollAngle) {
            s.present |= ONECAD_IOS_HAS_ROLL
            s.roll_rad = Double(touch.rollAngle)
            observedCapabilities |= ONECAD_IOS_CAP_ROLL
        }
    }

    mutating func takeObservedCapabilities() -> UInt32 {
        defer { observedCapabilities = 0 }
        return observedCapabilities
    }
}
