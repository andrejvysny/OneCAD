import UIKit

@MainActor
protocol InputDelivery: AnyObject {
    func deliver(samples: [onecad_ios_touch_sample])
    func observeCapabilities(_ flags: UInt32)
}

/// Converts raw UIKit touch callbacks into ordered sample batches:
/// per touch, earlier coalesced samples, then the actual touch, then
/// predictions. Pencil hover arrives through a UIHoverGestureRecognizer.
@MainActor
final class TouchInputController {
    private var registry = ContactRegistry()
    private var builder = TouchSampleBuilder()
    private var hoverContact: UInt64?
    weak var delivery: InputDelivery?

    func touches(_ touches: Set<UITouch>, event: UIEvent?, in view: UIView) {
        var samples: [onecad_ios_touch_sample] = []
        // Set order is arbitrary; contact order keeps batches deterministic.
        let ordered = touches.map { ($0, registry.contactID(for: $0)) }.sorted { $0.1 < $1.1 }
        for (touch, contact) in ordered {
            appendSamples(for: touch, contact: contact, event: event, in: view, into: &samples)
            if touch.phase == .ended || touch.phase == .cancelled { registry.end(touch) }
        }
        flush(samples)
    }

    private func appendSamples(
        for touch: UITouch, contact: UInt64, event: UIEvent?, in view: UIView,
        into samples: inout [onecad_ios_touch_sample]
    ) {
        var coalesced = event?.coalescedTouches(for: touch) ?? []
        // The last coalesced sample is the actual touch; it is sent once, as ACTUAL.
        if let last = coalesced.last, last.timestamp == touch.timestamp { coalesced.removeLast() }
        for sample in coalesced {
            append(sample, contact: contact, provenance: ONECAD_IOS_SAMPLE_COALESCED,
                   phase: TouchSampleBuilder.phase(sample.phase), in: view, into: &samples)
        }
        append(touch, contact: contact, provenance: ONECAD_IOS_SAMPLE_ACTUAL,
               phase: TouchSampleBuilder.phase(touch.phase), in: view, into: &samples)
        guard touch.phase == .began || touch.phase == .moved else { return }
        for predicted in event?.predictedTouches(for: touch) ?? [] {
            append(predicted, contact: contact, provenance: ONECAD_IOS_SAMPLE_PREDICTED,
                   phase: ONECAD_IOS_PHASE_MOVED, in: view, into: &samples)
        }
    }

    private func append(
        _ touch: UITouch, contact: UInt64, provenance: onecad_ios_provenance,
        phase: onecad_ios_phase, in view: UIView, into samples: inout [onecad_ios_touch_sample]
    ) {
        let sample = builder.sample(touch, contact: contact, provenance: provenance,
                                    phase: phase, in: view, registry: &registry)
        if sample.present & ONECAD_IOS_HAS_ESTIMATION_INDEX != 0, provenance != ONECAD_IOS_SAMPLE_PREDICTED {
            registry.noteEstimation(index: Int(sample.estimation_index), contact: contact,
                                    expectsUpdate: sample.expects_update != 0)
        }
        samples.append(sample)
    }

    /// Late refinements (Pencil force/angles, sometimes location). The touch
    /// may already have ended, so the id is looked up by estimation index.
    func estimatedPropertiesUpdated(_ touches: Set<UITouch>, in view: UIView) {
        var samples: [onecad_ios_touch_sample] = []
        for touch in touches {
            guard let index = touch.estimationUpdateIndex?.intValue,
                  let contact = registry.contactForEstimation(index: index) ?? registry.liveID(for: touch)
            else { continue }
            var sample = builder.sample(touch, contact: contact, provenance: ONECAD_IOS_SAMPLE_ESTIMATION_UPDATE,
                                        phase: TouchSampleBuilder.phase(touch.phase), in: view,
                                        registry: &registry)
            if touch.estimatedPropertiesExpectingUpdates.contains(.location) {
                sample.present &= ~ONECAD_IOS_HAS_LOCATION
            }
            registry.noteEstimation(index: index, contact: contact,
                                    expectsUpdate: !touch.estimatedPropertiesExpectingUpdates.isEmpty)
            samples.append(sample)
        }
        flush(samples.sorted { $0.contact_id < $1.contact_id })
    }

    /// Interruption already reported through `input_cancel_all`.
    func resetContacts() {
        registry.reset()
        hoverContact = nil
    }

    // MARK: Hover

    func hover(_ recognizer: UIHoverGestureRecognizer, in view: UIView) {
        let exited = recognizer.state == .ended || recognizer.state == .cancelled
            || recognizer.state == .failed
        guard exited || recognizer.state == .began || recognizer.state == .changed else { return }
        let contact = hoverContact ?? registry.allocate()
        hoverContact = exited ? nil : contact
        let sample = hoverSample(recognizer, contact: contact, exited: exited, in: view)
        if exited { registry.forget(contact) }
        flush([sample])
    }

    /// Pencil hover reports a positive zOffset; trackpad/mouse pointer hover
    /// reports zero, which is how the two are told apart.
    private func hoverSample(
        _ recognizer: UIHoverGestureRecognizer, contact: UInt64, exited: Bool, in view: UIView
    ) -> onecad_ios_touch_sample {
        let location = recognizer.location(in: view)
        let pencil = recognizer.zOffset > 0
        var s = onecad_ios_touch_sample()
        s.contact_id = contact
        s.kind = (pencil ? ONECAD_IOS_CONTACT_PENCIL : ONECAD_IOS_CONTACT_INDIRECT_POINTER).rawValue
        s.phase = (exited ? ONECAD_IOS_PHASE_HOVER_EXITED : ONECAD_IOS_PHASE_HOVER_MOVED).rawValue
        s.provenance = ONECAD_IOS_SAMPLE_ACTUAL.rawValue
        s.timestamp_s = CACurrentMediaTime()
        s.x_pt = Double(location.x)
        s.y_pt = Double(location.y)
        s.present = ONECAD_IOS_HAS_LOCATION
        guard pencil else { return s }
        s.present |= ONECAD_IOS_HAS_HOVER_Z | ONECAD_IOS_HAS_ALTITUDE | ONECAD_IOS_HAS_AZIMUTH
        s.hover_z = Double(recognizer.zOffset)
        s.altitude_rad = Double(recognizer.altitudeAngle)
        s.azimuth_rad = Double(recognizer.azimuthAngle(in: view))
        if registry.reportsRoll(contact, roll: recognizer.rollAngle) {
            s.present |= ONECAD_IOS_HAS_ROLL
            s.roll_rad = Double(recognizer.rollAngle)
        }
        delivery?.observeCapabilities(ONECAD_IOS_CAP_HOVER | ONECAD_IOS_CAP_PENCIL_CONNECTED)
        return s
    }

    private func flush(_ samples: [onecad_ios_touch_sample]) {
        let caps = builder.takeObservedCapabilities()
        if caps != 0 { delivery?.observeCapabilities(caps) }
        guard !samples.isEmpty else { return }
        delivery?.deliver(samples: samples)
    }
}
