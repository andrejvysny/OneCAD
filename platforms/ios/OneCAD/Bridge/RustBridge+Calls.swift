import Foundation

/// Typed wrappers for the per-subsystem ABI calls. Inputs are copied by Rust
/// before each call returns, so borrowed pointers never outlive the call.
extension RustBridge {
    static let maxSamplesPerBatch = 4096

    // MARK: Surface (attach/detach own the layer reference, see RustBridge)

    @discardableResult
    func resizeSurface(_ desc: onecad_ios_surface_desc) -> onecad_ios_status {
        var desc = desc
        return call("surface_resize") { onecad_ios_surface_resize($0, &desc) }
    }

    // MARK: Frame

    func frame(time: Double) -> (onecad_ios_status, onecad_ios_frame_result) {
        var result = onecad_ios_frame_result()
        result.struct_size = UInt32(MemoryLayout<onecad_ios_frame_result>.size)
        let status = call("frame") { onecad_ios_frame($0, time, &result) }
        return (status, result)
    }

    // MARK: Input

    /// Splits at the ABI batch limit instead of dropping samples; ordering is
    /// preserved because batches are submitted sequentially.
    func submit(samples: [onecad_ios_touch_sample], viewGeneration: UInt64) {
        var start = 0
        while start < samples.count {
            let end = min(start + Self.maxSamplesPerBatch, samples.count)
            samples[start..<end].withUnsafeBufferPointer { buffer in
                var batch = onecad_ios_input_batch()
                batch.struct_size = UInt32(MemoryLayout<onecad_ios_input_batch>.size)
                batch.count = UInt32(buffer.count)
                batch.samples = buffer.baseAddress
                batch.view_generation = viewGeneration
                _ = call("input_submit") { onecad_ios_input_submit($0, &batch) }
            }
            start = end
        }
    }

    func cancelAllInput(reason: UInt32) {
        _ = call("input_cancel_all") { onecad_ios_input_cancel_all($0, reason) }
    }

    func submitNavigation(_ nav: onecad_ios_navigation) {
        var nav = nav
        nav.struct_size = UInt32(MemoryLayout<onecad_ios_navigation>.size)
        _ = call("navigation_submit") { onecad_ios_navigation_submit($0, &nav) }
    }

    func submitPencilEvent(_ event: onecad_ios_pencil_event) {
        var event = event
        event.struct_size = UInt32(MemoryLayout<onecad_ios_pencil_event>.size)
        _ = call("pencil_event_submit") { onecad_ios_pencil_event_submit($0, &event) }
    }

    func setCapabilities(_ caps: onecad_ios_capabilities) {
        var caps = caps
        caps.struct_size = UInt32(MemoryLayout<onecad_ios_capabilities>.size)
        _ = call("capabilities_set") { onecad_ios_capabilities_set($0, &caps) }
    }

    // MARK: Keyboard and text

    func submitKey(_ event: onecad_ios_key_event) {
        var event = event
        event.struct_size = UInt32(MemoryLayout<onecad_ios_key_event>.size)
        _ = call("key_submit") { onecad_ios_key_submit($0, &event) }
    }

    func insertText(_ text: String) {
        _ = call("text_insert") { app in text.withCString { onecad_ios_text_insert(app, $0) } }
    }

    func deleteBackward() {
        _ = call("text_delete_backward") { onecad_ios_text_delete_backward($0) }
    }

    /// `nil` ends composition without committing (ABI: NULL or "").
    func setMarkedText(_ text: String?) {
        _ = call("text_set_marked") { app in
            guard let text else { return onecad_ios_text_set_marked(app, nil) }
            return text.withCString { onecad_ios_text_set_marked(app, $0) }
        }
    }

    func setKeyboardOverlap(_ points: Float) {
        _ = call("keyboard_frame") { onecad_ios_keyboard_frame($0, points) }
    }

    // MARK: Lifecycle

    /// Exempt from the terminal latch: DID_ENTER_BACKGROUND / WILL_TERMINATE
    /// drive the autosave flush, which still matters in relaunch-required.
    func lifecycle(_ event: onecad_ios_lifecycle_event) {
        _ = call("lifecycle", allowWhenTerminal: true) { onecad_ios_lifecycle($0, event.rawValue) }
    }

    // MARK: Documents

    func openPath(_ path: String) {
        _ = call("open_path") { app in path.withCString { onecad_ios_open_path(app, $0) } }
    }
}
