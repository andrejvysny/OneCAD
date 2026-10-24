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
            try sendKey(usage, pressed: pressed, modifiers: modifiers)
        }
        // Complete the physical chord: held modifiers must not leak into the
        // next Pencil gesture after the main key has been released.
        var held = modifiers
        let releases: [(UInt32, UInt32)] = [
            (ONECAD_IOS_MOD_SHIFT, 0xE1), (ONECAD_IOS_MOD_CONTROL, 0xE0),
            (ONECAD_IOS_MOD_OPTION, 0xE2), (ONECAD_IOS_MOD_COMMAND, 0xE3),
        ]
        for (mask, modifierUsage) in releases where held & mask != 0 {
            held &= ~mask
            try sendKey(modifierUsage, pressed: 0, modifiers: held)
        }
        try await test.nextRunLoopTick()
    }

    private func sendKey(_ usage: UInt32, pressed: UInt32, modifiers: UInt32) throws {
        var event = onecad_ios_key_event()
        event.struct_size = UInt32(MemoryLayout<onecad_ios_key_event>.size)
        event.hid_usage = usage
        event.modifiers = modifiers
        event.pressed = pressed
        try checked(onecad_ios_key_submit(app, &event), "key_submit")
    }

    func replaceField(_ identifier: String, with text: String) async throws {
        try await test.waitFor("\(identifier) visible", timeout: 15) { documents.exists(identifier) }
        let frame = try await visibleCardField(identifier)
        try await fingerTap(CGPoint(x: frame.midX, y: frame.midY))
        try await replaceFocusedField(identifier, with: text)
    }

    private func visibleCardField(_ identifier: String) async throws -> CGRect {
        for attempt in 0...8 {
            let geometry = try await stableCardField(identifier)
            if geometry.body.contains(geometry.field) { return geometry.field }
            guard attempt < 8 else { break }
            let upward = geometry.field.midY > geometry.body.midY
            let x = geometry.header.minX + 20
            let lower = geometry.body.maxY - 8
            let upper = geometry.body.minY + 8
            // Short drags avoid throwing the field past the opposite clip edge.
            let distance = min(lower - upper, max(8, min(
                abs(geometry.field.midY - geometry.body.midY) * 0.5, geometry.body.height / 3)))
            documents.diagnostic("scroll card field \(identifier), attempt \(attempt + 1), body \(geometry.body), field \(geometry.field)")
            try await fingerDrag(from: CGPoint(x: x, y: upward ? lower : upper),
                                 to: CGPoint(x: x, y: upward ? lower - distance : upper + distance))
        }
        documents.diagnostic("card field remained clipped: \(identifier)")
        throw SemanticHooks.Failure(description: "\(identifier) outside visible card body after 8 finger scrolls")
    }

    private struct CardFieldGeometry: Equatable {
        let header: CGRect
        let body: CGRect
        let field: CGRect
        let generation: UInt64
    }

    private func stableCardField(_ identifier: String) async throws -> CardFieldGeometry {
        var previous: CardFieldGeometry?
        var stable = 0
        try await test.waitFor("\(identifier) card geometry settled", timeout: 15) {
            let current = try cardFieldGeometry(identifier)
            stable = previous == current ? stable + 1 : 0
            previous = current
            return stable >= 3
        }
        return try XCTUnwrap(previous)
    }

    private func cardFieldGeometry(_ identifier: String) throws -> CardFieldGeometry {
        let canvas = try XCTUnwrap(Self.canvas())
        let nodes = documents.nodes()
        let field = try XCTUnwrap(nodes.first { $0.identifier == identifier }).frame
        let cancel = try XCTUnwrap(nodes.first { $0.identifier == "onecad.op.cancel" }).frame
        let confirm = try XCTUnwrap(nodes.first { $0.identifier == "onecad.op.confirm" }).frame
        let footer = cancel.union(confirm)
        let headers = nodes.filter {
            $0.role == ONECAD_IOS_ROLE_STATIC_TEXT.rawValue && $0.frame.height >= 32
                && $0.frame.width >= footer.width && $0.frame.maxY <= footer.minY
                && $0.frame.minX <= footer.minX && $0.frame.maxX >= footer.maxX
        }
        guard headers.count == 1, let header = headers.first?.frame else {
            documents.diagnostic("ambiguous card header for \(identifier): \(headers.map(\.label))")
            throw SemanticHooks.Failure(description: "expected exactly one active card header")
        }
        let body = CGRect(x: header.minX + 8, y: header.maxY + 4,
                          width: header.width - 16, height: footer.minY - header.maxY - 8)
            .intersection(try viewport()).intersection(canvas.bounds.inset(by: canvas.safeAreaInsets))
        guard !body.isNull, body.height > 32 else {
            throw SemanticHooks.Failure(description: "active card has no usable visible body: \(body)")
        }
        return CardFieldGeometry(header: header, body: body, field: field, generation: canvas.viewGeneration)
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

    /// Programmatic Pencil samples qualify confirmed delivery, never hardware.
    /// ACTUAL distinguishes confirmed samples from predictions, not synthesis.
    func pencilTap(_ point: CGPoint) async throws {
        try await contactTap(point, kind: ONECAD_IOS_CONTACT_PENCIL.rawValue)
    }

    func fingerTap(_ point: CGPoint) async throws {
        try await contactTap(point, kind: ONECAD_IOS_CONTACT_FINGER.rawValue)
    }

    private func fingerDrag(from start: CGPoint, to end: CGPoint) async throws {
        let canvas = try XCTUnwrap(Self.canvas(), "no live Metal canvas")
        let contact = UInt64.random(in: 1_000_000...UInt64.max)
        for index in 0...12 {
            let progress = CGFloat(index) / 12
            var sample = onecad_ios_touch_sample()
            sample.contact_id = contact
            sample.kind = ONECAD_IOS_CONTACT_FINGER.rawValue
            sample.provenance = ONECAD_IOS_SAMPLE_ACTUAL.rawValue
            sample.present = ONECAD_IOS_HAS_LOCATION
            sample.phase = (index == 0 ? ONECAD_IOS_PHASE_BEGAN
                : index == 12 ? ONECAD_IOS_PHASE_ENDED : ONECAD_IOS_PHASE_MOVED).rawValue
            sample.x_pt = Double(start.x + (end.x - start.x) * progress)
            sample.y_pt = Double(start.y + (end.y - start.y) * progress)
            sample.timestamp_s = CACurrentMediaTime()
            let status = withUnsafePointer(to: &sample) { pointer in
                var batch = onecad_ios_input_batch()
                batch.struct_size = UInt32(MemoryLayout<onecad_ios_input_batch>.size)
                batch.count = 1
                batch.samples = pointer
                batch.view_generation = canvas.viewGeneration
                return onecad_ios_input_submit(app, &batch)
            }
            try checked(status, "finger drag input_submit")
            try await test.nextRunLoopTick()
        }
    }

    private func contactTap(_ point: CGPoint, kind: UInt32) async throws {
        let canvas = try XCTUnwrap(Self.canvas(), "no live Metal canvas")
        var sample = onecad_ios_touch_sample()
        sample.contact_id = UInt64.random(in: 1_000_000...UInt64.max)
        sample.kind = kind
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

    static func canvas() -> MetalCanvasView? {
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
        try await pencilTap(sketchPoint(0, 0))
        try await numeric([("onecad.dimension.input", "60"), ("onecad.param.height", "40")])
        try await waitForText("Entities: 8")
        try await waitForText("Constraints: 7")
        try await tool("point")
        try await numeric([("onecad.dimension.input", "30"), ("onecad.param.y", "20")])
        try await waitForText("Horizontal distance (mm): 30")
        try await waitForText("Vertical distance (mm): 20")
        try await tool("circle")
        try await pencilTap(sketchPoint(30, 20))
        try await numeric([("onecad.dimension.input", "6")])
        try await waitForText("Coincident")
        try await waitForText("Diameter (mm): 6")
        try await waitForText("Sketch · Fully constrained")
        try await documents.activate("onecad.tool.finishSketch")
        try await waitForText("2 closed region(s) returned")
        try await pencilTap(sketchPoint(10, 10))
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
        let front = try await verticalEdgeView()
        let body = try XCTUnwrap((front["bodies"] as? [[String: Any]])?.first)
        let min = try Self.vector(body, "min")
        let max = try Self.vector(body, "max")
        let point = SIMD3(max.x, min.y, (min.z + max.z) / 2)
        let projected = try project(point, summary: front, orthographic: true)
        let lower = try project(SIMD3(max.x, min.y, min.z), summary: front, orthographic: true)
        let upper = try project(SIMD3(max.x, min.y, max.z), summary: front, orthographic: true)
        XCTAssertGreaterThan(abs(lower.y - upper.y), 44, "vertical edge remains visibly separated from its endpoints")
        documents.diagnostic("before vertical edge pick\nworld \(point), tap \(projected), viewport \(try viewport()), orthographic view \(DocumentDriver.view(front))")
        try await pencilTap(projected)
        try await documents.activate(label: "Properties")
        try await waitForText("Edge")
        documents.diagnostic("after vertical edge pick\nview \(DocumentDriver.view(await documents.summary() ?? [:]))")
        try await tool(operation.lowercased())
        try await replaceField("onecad.dimension.input", with: "1")
        try await confirmOperation()
        return try await documents.waitForSummary("\(operation) regenerated", timeout: 90) {
            DocumentDriver.features($0) == ["Sketch", "Extrude", operation] && !DocumentDriver.busy($0)
        }
    }

    private func verticalEdgeView() async throws -> [String: Any] {
        try await test.waitFor("native text input ended before navigation", timeout: 15) {
            Self.canvas()?.subviews.compactMap { $0 as? KeyInputProxy }.allSatisfy { !$0.isFirstResponder } == true
        }
        let explorer = try XCTUnwrap(documents.node { $0.identifier == "button.explorer" })
        if explorer.flags & ONECAD_IOS_A11Y_SELECTED != 0 {
            try await documents.activate("button.explorer")
        }
        try await documents.activate(label: "Home view")
        try await documents.activate(label: "FRONT view")
        // Home restores Perspective; the public toggle establishes Orthographic.
        try await documents.activate(label: "Toggle perspective / orthographic projection")
        try await documents.activate(label: "Fit model in view")
        var previous: [Double] = []
        var stable = 0
        return try await documents.waitForSummary("front orthographic fit and viewport settled") {
            let view = DocumentDriver.view($0)
            guard let offset = view["cameraOffset"] as? [Double], offset.count == 3,
                  abs(offset[0]) < 1e-8, offset[1] < 0, abs(offset[2]) < 1e-8,
                  let target = view["cameraTarget"] as? [Double], let frame = try? viewport(),
                  !DocumentDriver.busy($0) else { return false }
            let current = offset + target + [frame.minX, frame.minY, frame.width, frame.height].map(Double.init)
            stable = current == previous ? stable + 1 : 0
            previous = current
            return stable >= 5
        }
    }

    func project(_ point: SIMD3<Double>, summary: [String: Any], orthographic: Bool) throws -> CGPoint {
        let view = DocumentDriver.view(summary)
        let offset = try Self.vector(view, "cameraOffset")
        let target = try Self.vector(view, "cameraTarget")
        let back = simd_normalize(offset)
        let right = simd_normalize(SIMD3(-offset.y, offset.x, 0))
        let up = simd_cross(back, right)
        let relative = point - target
        let depth = orthographic ? simd_length(offset) : simd_length(offset) - simd_dot(relative, back)
        let frame = try viewport()
        let scale = Double(frame.height) / (2 * depth * tan(38 * .pi / 180))
        return CGPoint(x: frame.midX + simd_dot(relative, right) * scale,
                       y: frame.midY - simd_dot(relative, up) * scale)
    }

    func confirmOperation() async throws {
        try await test.waitFor("Create enabled", timeout: 60) {
            documents.node {
                $0.identifier == "onecad.op.confirm" && $0.flags & ONECAD_IOS_A11Y_DISABLED == 0
            } != nil
        }
        // Return applies the focused numeric card without racing the software
        // keyboard's layout animation and a moving button's old bounds.
        try await key(0x28)
        do {
            try await test.waitFor("operation card closed", timeout: 30) {
                !documents.exists("onecad.op.confirm")
            }
        } catch {
            documents.diagnostic("Return confirmation failed: \(error)")
            throw error
        }
    }

    private static func vector(_ object: [String: Any], _ key: String) throws -> SIMD3<Double> {
        let values = try XCTUnwrap(object[key] as? [Double], "missing \(key)")
        guard values.count == 3 else { throw SemanticHooks.Failure(description: "\(key): \(values)") }
        return SIMD3(values[0], values[1], values[2])
    }
}
#endif
