import QuartzCore
import simd
import UIKit
import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Authors models through the editor's controls and public input ABI. Semantic
/// hooks only observe geometry and save the editor's document artifacts.
@MainActor
struct ModelDriver {
    let documents: DocumentDriver

    private var test: XCTestCase { documents.test }
    private var app: OpaquePointer { documents.hooks.app }

    func waitForText(_ label: String) async throws {
        try await test.waitFor(label, timeout: 30) { documents.node { $0.label == label } != nil }
    }

    func tool(_ name: String) async throws {
        let identifier = "onecad.tool.\(name)"
        if !documents.exists(identifier) { try await documents.activate("onecad.tool.more") }
        try await documents.activate(identifier)
    }

    func key(_ usage: UInt32, modifiers: UInt32 = 0) async throws {
        for pressed: UInt32 in [1, 0] {
            var event = onecad_ios_key_event()
            event.struct_size = UInt32(MemoryLayout<onecad_ios_key_event>.size)
            event.hid_usage = usage
            event.modifiers = modifiers
            event.pressed = pressed
            try checked(onecad_ios_key_submit(app, &event), "key_submit")
        }
        try await test.nextRunLoopTick()
    }

    func replaceField(_ identifier: String, with text: String) async throws {
        try await test.waitFor("\(identifier) visible", timeout: 15) { documents.exists(identifier) }
        let frame = try XCTUnwrap(documents.node { $0.identifier == identifier }).frame
        try await tap(CGPoint(x: frame.midX, y: frame.midY))
        try await replaceFocusedField(identifier, with: text)
    }

    private func replaceFocusedField(_ identifier: String, with text: String) async throws {
        try await test.waitFor("\(identifier) focused", timeout: 15) {
            documents.node { $0.identifier == identifier && $0.flags & ONECAD_IOS_A11Y_FOCUSED != 0 } != nil
        }
        try await key(0x04, modifiers: ONECAD_IOS_MOD_COMMAND)
        try checked(text.withCString { onecad_ios_text_insert(app, $0) }, "text_insert")
        try await test.nextRunLoopTick()
        try await test.waitFor("\(identifier) shows \(text)", timeout: 15) {
            documents.node { $0.identifier == identifier && $0.value == text } != nil
        }
    }

    func numeric(_ fields: [(String, String)]) async throws {
        try await documents.activate(label: "Numeric input · Tab")
        for (index, field) in fields.enumerated() {
            if index > 0 { try await key(0x2B) }
            try await replaceFocusedField(field.0, with: field.1)
        }
        try await key(0x28)
    }

    /// Samples use the live canvas generation, as the host's UIKit input does.
    /// These are simulator-synthesized finger taps, never Pencil evidence.
    func tap(_ point: CGPoint) async throws {
        let canvas = try XCTUnwrap(Self.canvas(), "no live Metal canvas")
        var sample = onecad_ios_touch_sample()
        sample.contact_id = UInt64.random(in: 1_000_000...UInt64.max)
        sample.kind = ONECAD_IOS_CONTACT_FINGER.rawValue
        sample.provenance = ONECAD_IOS_SAMPLE_ACTUAL.rawValue
        sample.present = ONECAD_IOS_HAS_LOCATION
        sample.x_pt = Double(point.x)
        sample.y_pt = Double(point.y)
        sample.tap_count = 1
        for phase in [ONECAD_IOS_PHASE_BEGAN, ONECAD_IOS_PHASE_ENDED] {
            sample.phase = phase.rawValue
            sample.timestamp_s = CACurrentMediaTime()
            let status = withUnsafePointer(to: &sample) { pointer in
                var batch = onecad_ios_input_batch()
                batch.struct_size = UInt32(MemoryLayout<onecad_ios_input_batch>.size)
                batch.count = 1
                batch.samples = pointer
                batch.view_generation = canvas.viewGeneration
                return onecad_ios_input_submit(app, &batch)
            }
            try checked(status, "input_submit")
            try await test.nextRunLoopTick()
        }
    }

    private func checked(_ status: onecad_ios_status, _ call: String) throws {
        guard status == ONECAD_IOS_OK else {
            throw SemanticHooks.Failure(description: "\(call): \(status.rawValue): \(String(cString: onecad_ios_last_error(app)))")
        }
    }

    private static func canvas() -> MetalCanvasView? {
        func find(_ view: UIView) -> MetalCanvasView? {
            if let canvas = view as? MetalCanvasView { return canvas }
            return view.subviews.lazy.compactMap(find).first
        }
        return UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).lazy.compactMap { find($0) }.first
    }

    private func viewport() throws -> CGRect {
        try XCTUnwrap(documents.node { $0.identifier == "onecad.viewport" }).frame
    }

    /// The XY sketch camera aligns screen right/up with sketch X/Y. Its
    /// orthographic size follows offset length and the shared 76° camera FOV.
    func sketchPoint(_ x: Double, _ y: Double) async throws -> CGPoint {
        let current = await documents.summary()
        let summary = try XCTUnwrap(current)
        let offset = try Self.vector(DocumentDriver.view(summary), "cameraOffset")
        let frame = try viewport()
        let scale = Double(frame.height) / (2 * simd_length(offset) * tan(38 * .pi / 180))
        return CGPoint(x: frame.midX + x * scale, y: frame.midY - y * scale)
    }

    /// One fully constrained profile: rectangle and centered circle share a
    /// sketch, so the saved history is exactly Sketch, Extrude.
    func authorPlate() async throws -> [String: Any] {
        _ = try await documents.newPart()
        try await documents.activate("onecad.tool.sketch")
        try await documents.activate("onecad.plane.xy")
        try await documents.activate(label: "Properties")
        try await waitForText("Sketch · Fully constrained")
        try await tool("rectangle")
        try await tap(sketchPoint(0, 0))
        try await numeric([("onecad.dimension.input", "60"), ("onecad.param.height", "40")])
        try await waitForText("Entities: 8")
        try await waitForText("Constraints: 7")
        try await tool("point")
        try await numeric([("onecad.dimension.input", "30"), ("onecad.param.y", "20")])
        try await waitForText("Horizontal distance (mm): 30")
        try await waitForText("Vertical distance (mm): 20")
        try await tool("circle")
        try await tap(sketchPoint(30, 20))
        try await numeric([("onecad.dimension.input", "6")])
        try await waitForText("Coincident")
        try await waitForText("Diameter (mm): 6")
        try await waitForText("Sketch · Fully constrained")
        try await documents.activate("onecad.tool.finishSketch")
        try await waitForText("2 closed region(s) returned")
        try await tap(sketchPoint(10, 10))
        try await tool("extrude")
        try await replaceField("onecad.dimension.input", with: "4")
        try await confirmOperation()
        return try await documents.waitForSummary("plate regenerated", timeout: 90) {
            DocumentDriver.features($0) == ["Sketch", "Extrude"] && !DocumentDriver.busy($0)
                && ($0["bodies"] as? [Any])?.count == 1
        }
    }

    func solveFirstSketch() async throws -> [String: Any] {
        try await documents.activate(label: "History")
        try await documents.activate("onecad.history.0")
        try await documents.activate(label: "Edit feature")
        try await test.waitFor("sketch editor opened", timeout: 30) {
            documents.exists("onecad.tool.finishSketch")
        }
        try await documents.activate(label: "Properties")
        try await test.waitFor("fully constrained sketch editor", timeout: 30) {
            documents.node { $0.label.hasSuffix(" · Fully constrained") } != nil
        }
        try await documents.activate("onecad.tool.finishSketch")
        return try await documents.waitForSummary("solved sketch retained") {
            !DocumentDriver.busy($0) && ($0["sketches"] as? [[String: Any]])?.first?["dof"] as? Int == 0
        }
    }

    func modifyVerticalEdge(_ operation: String) async throws -> [String: Any] {
        try await documents.activate(label: "Home view")
        let home = try await documents.waitForSummary("isometric fitted camera") {
            guard let offset = DocumentDriver.view($0)["cameraOffset"] as? [Double] else { return false }
            return offset.count == 3 && offset[0] > 0 && offset[1] < 0 && offset[2] > 0
                && simd_length(SIMD3(offset[0], offset[1], offset[2])) < 200
        }
        let body = try XCTUnwrap((home["bodies"] as? [[String: Any]])?.first)
        let min = try Self.vector(body, "min")
        let max = try Self.vector(body, "max")
        let point = SIMD3(max.x, min.y, (min.z + max.z) / 2)
        try await tap(project(point, summary: home))
        try await documents.activate(label: "Properties")
        try await waitForText("Edge")
        try await tool(operation.lowercased())
        try await replaceField("onecad.dimension.input", with: "1")
        try await confirmOperation()
        return try await documents.waitForSummary("\(operation) regenerated", timeout: 90) {
            DocumentDriver.features($0) == ["Sketch", "Extrude", operation] && !DocumentDriver.busy($0)
        }
    }

    private func project(_ point: SIMD3<Double>, summary: [String: Any]) throws -> CGPoint {
        let view = DocumentDriver.view(summary)
        let offset = try Self.vector(view, "cameraOffset")
        let target = try Self.vector(view, "cameraTarget")
        let back = simd_normalize(offset)
        let right = simd_normalize(SIMD3(-offset.y, offset.x, 0))
        let up = simd_cross(back, right)
        let relative = point - target
        let depth = simd_length(offset) - simd_dot(relative, back)
        let frame = try viewport()
        let scale = Double(frame.height) / (2 * depth * tan(38 * .pi / 180))
        return CGPoint(x: frame.midX + simd_dot(relative, right) * scale,
                       y: frame.midY - simd_dot(relative, up) * scale)
    }

    private func confirmOperation() async throws {
        try await test.waitFor("Create enabled", timeout: 60) {
            documents.node {
                $0.identifier == "onecad.op.confirm" && $0.flags & ONECAD_IOS_A11Y_DISABLED == 0
            } != nil
        }
        // Return applies the focused numeric card without racing the software
        // keyboard's layout animation and a moving button's old bounds.
        try await key(0x28)
        try await test.waitFor("operation card closed", timeout: 30) {
            !documents.exists("onecad.op.confirm")
        }
    }

    private static func vector(_ object: [String: Any], _ key: String) throws -> SIMD3<Double> {
        let values = try XCTUnwrap(object[key] as? [Double], "missing \(key)")
        guard values.count == 3 else { throw SemanticHooks.Failure(description: "\(key): \(values)") }
        return SIMD3(values[0], values[1], values[2])
    }
}
#endif
