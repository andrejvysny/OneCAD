import QuartzCore
import simd
import UIKit
import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Confirmed synthetic Pencil samples traverse the production normalizer/router.
/// This qualifies simulator delivery, never physical Pencil hardware.
@MainActor
final class PencilModelingTests: XCTestCase {
    private var documents: DocumentDriver?
    private var ownedDocument: String?
    private var pencil: ModelingPencil?

    override func setUp() async throws { continueAfterFailure = false }

    override func tearDown() async throws {
        guard let documents else { return }
        _ = onecad_ios_input_cancel_all(documents.hooks.app, ONECAD_IOS_CANCEL_SYSTEM_GESTURE)
        try? pencil?.seal()
        guard let ownedDocument, let summary = await documents.summary(),
              DocumentDriver.document(summary)["documentId"] as? String == ownedDocument else { return }
        if documents.exists("onecad.op.cancel") {
            try await documents.activate("onecad.op.cancel")
        }
        if documents.exists("onecad.tool.discardSketch") {
            try await documents.activate("onecad.tool.discardSketch")
        }
        try await documents.file("onecad.menu.file.closeTab")
        if documents.node({ $0.label == "Don't save" }) != nil {
            try await documents.activate(label: "Don't save")
        }
    }

    func testSyntheticPencilStrokeRectangleExtrudeAndHistoryInLandscape() async throws {
        let documents = try await DocumentDriver.attach(self)
        self.documents = documents
        if documents.node({ $0.label == "Restore" }) != nil {
            try await documents.activate(label: "Restore")
        }
        let created = try await documents.newPart()
        ownedDocument = try XCTUnwrap(DocumentDriver.document(created)["documentId"] as? String)
        let pencil = try ModelingPencil(documents: documents)
        self.pencil = pencil
        _ = try await pencil.stableViewport()
        let model = ModelDriver(documents: documents)
        try await documents.activate("onecad.tool.sketch")
        try await documents.activate("onecad.plane.xy")
        try await strokeThenUndo(model, pencil: pencil)
        try await rectangle(model, pencil: pencil)
        let accepted = try await extrude(model, pencil: pencil)
        try await history(accepted, model: model, pencil: pencil)
        try pencil.seal()
    }

    private func strokeThenUndo(_ model: ModelDriver, pencil: ModelingPencil) async throws {
        try await model.tool("penStroke")
        var points: [CGPoint] = []
        for index in 0...20 {
            points.append(try await model.sketchPoint(-20 + Double(index) * 2, 5))
        }
        try await pencil.drag(points)
        let accepted = try await sketchCount(3, model: model)
        pencil.record("confirmed-stroke-accepted", summary: accepted)
        try await model.key(0x1D, modifiers: ONECAD_IOS_MOD_COMMAND)
        let undone = try await sketchCount(0, model: model)
        pencil.record("stroke-undo-empty-sketch", summary: undone)
    }

    private func rectangle(_ model: ModelDriver, pencil: ModelingPencil) async throws {
        try await model.tool("rectangle")
        // Exact coordinate QA excludes the default angular snap of the rectangle diagonal.
        let snap = try XCTUnwrap(model.documents.node {
            $0.label == "Snap" && $0.role == ONECAD_IOS_ROLE_CHECKBOX.rawValue
        })
        if snap.flags & ONECAD_IOS_A11Y_SELECTED != 0 {
            try await model.documents.activate("sketch Snap checkbox") {
                $0.id == snap.id
            }
        }
        try await pencil.tap(model.sketchPoint(0, 0))
        try await pencil.tap(model.sketchPoint(60, 40))
        let accepted = try await sketchCount(8, model: model)
        pencil.record("confirmed-rectangle-accepted", summary: accepted)
        try await model.documents.activate("onecad.tool.finishSketch")
        try await model.waitForText("1 closed region(s) returned")
        let finished = try await model.documents.waitForSummary("Pencil profile accepted") {
            DocumentDriver.features($0) == ["Sketch"] && !DocumentDriver.busy($0)
        }
        XCTAssertEqual((finished["sketches"] as? [[String: Any]])?.first?["entities"] as? Int, 8)
        pencil.record("closed-profile", summary: finished)
    }

    private func extrude(_ model: ModelDriver, pencil: ModelingPencil) async throws -> [String: Any] {
        let documents = model.documents
        _ = try await documents.waitForSummary("restored isometric camera") {
            guard let offset = DocumentDriver.view($0)["cameraOffset"] as? [Double], offset.count == 3 else { return false }
            return offset[0] > 0 && offset[1] < 0 && offset[2] > 0 && !DocumentDriver.busy($0)
        }
        _ = try await pencil.stableViewport()
        let currentView = await documents.summary()
        let view = try XCTUnwrap(currentView)
        pencil.record("restored-camera-before-profile-pick", summary: view)
        // The canonical XY sketch basis maps (u, v) to world (-v, u, 0).
        try await pencil.tap(pencil.project(SIMD3(-20, 30, 0), summary: view))
        try await model.tool("extrude")
        try await waitForCreate(documents)
        let frame = try await pencil.stableViewport()
        let before = try await documents.waitForSummary("Extrude draft ready") { !DocumentDriver.busy($0) }
        let field = try XCTUnwrap(documents.node { $0.identifier == "onecad.dimension.input" })
        let distance = try XCTUnwrap(Double(field.value), "distance must expose its actual numeric draft: \(field.value)")
        XCTAssertTrue(distance.isFinite)
        // At the restored camera scale the shared handle snaps to 10 mm increments.
        let target = distance + 20
        let start = try pencil.project(SIMD3(-20, 30, distance), summary: before, viewport: frame)
        let end = try pencil.project(SIMD3(-20, 30, target), summary: before, viewport: frame)
        XCTAssertGreaterThan(hypot(end.x - start.x, end.y - start.y), 8, "handle drag must exceed its threshold")
        try await pencil.drag((0...6).map { index in
            let fraction = CGFloat(index) / 6
            return CGPoint(x: start.x + (end.x - start.x) * fraction, y: start.y + (end.y - start.y) * fraction)
        })
        try await waitFor("Pencil handle changed draft", timeout: 30) {
            guard let value = documents.node({ $0.identifier == "onecad.dimension.input" })?.value,
                  let number = Double(value) else { return false }
            return abs(number - distance) > 1
        }
        let draft = try XCTUnwrap(documents.node { $0.identifier == "onecad.dimension.input" })
        let actualDistance = try XCTUnwrap(Double(draft.value))
        XCTAssertEqual(actualDistance, target, accuracy: 0.51, "shared handle snapping")
        let uncommitted = try await documents.waitForSummary("handle draft only") { !DocumentDriver.busy($0) }
        XCTAssertEqual(DocumentDriver.features(uncommitted), ["Sketch"])
        XCTAssertEqual(DocumentDriver.document(uncommitted)["undoDepth"] as? Int,
                       DocumentDriver.document(before)["undoDepth"] as? Int)
        pencil.record("pencil-handle-draft-only", summary: uncommitted)
        try await waitForCreate(documents)
        let confirm = try XCTUnwrap(documents.node { $0.identifier == "onecad.op.confirm" }).frame
        try await pencil.tap(CGPoint(x: confirm.midX, y: confirm.midY))
        let accepted = try await documents.waitForSummary("Pencil extrusion committed", timeout: 90) {
            DocumentDriver.features($0) == ["Sketch", "Extrude"] && !DocumentDriver.busy($0)
                && ($0["bodies"] as? [Any])?.count == 1
        }
        try assertSolid(accepted, height: actualDistance)
        XCTAssertEqual(DocumentDriver.document(accepted)["undoDepth"] as? Int,
                       (DocumentDriver.document(before)["undoDepth"] as? Int).map { $0 + 1 })
        pencil.record("committed-live-solid", summary: accepted)
        return accepted
    }

    private func history(_ accepted: [String: Any], model: ModelDriver, pencil: ModelingPencil) async throws {
        let depth = try XCTUnwrap(DocumentDriver.document(accepted)["undoDepth"] as? Int)
        try await model.key(0x1D, modifiers: ONECAD_IOS_MOD_COMMAND)
        let undone = try await model.documents.waitForSummary("Pencil extrusion undo") {
            DocumentDriver.features($0) == ["Sketch"] && !DocumentDriver.busy($0)
                && DocumentDriver.document($0)["undoDepth"] as? Int == depth - 1
        }
        XCTAssertEqual((undone["bodies"] as? [Any])?.count, 0)
        pencil.record("extrusion-undo", summary: undone)
        try await model.key(0x1D, modifiers: ONECAD_IOS_MOD_COMMAND | ONECAD_IOS_MOD_SHIFT)
        let redone = try await model.documents.waitForSummary("Pencil extrusion redo", timeout: 90) {
            DocumentDriver.features($0) == ["Sketch", "Extrude"] && !DocumentDriver.busy($0)
                && DocumentDriver.document($0)["undoDepth"] as? Int == depth
        }
        let body = try XCTUnwrap((accepted["bodies"] as? [[String: Any]])?.first)
        let min = try XCTUnwrap(body["min"] as? [Double])
        let max = try XCTUnwrap(body["max"] as? [Double])
        try assertSolid(redone, height: max[2] - min[2])
        let restored = try XCTUnwrap((redone["bodies"] as? [[String: Any]])?.first)
        XCTAssertEqual(try XCTUnwrap(restored["volume"] as? Double), try XCTUnwrap(body["volume"] as? Double), accuracy: 1e-5)
        pencil.record("extrusion-redo", summary: redone)
    }

    private func sketchCount(_ count: Int, model: ModelDriver) async throws -> [String: Any] {
        try await model.documents.waitForSummary("accepted sketch entities \(count)") {
            ($0["sketches"] as? [[String: Any]])?.first?["entities"] as? Int == count && !DocumentDriver.busy($0)
        }
    }

    private func waitForCreate(_ documents: DocumentDriver) async throws {
        try await waitFor("Create enabled", timeout: 60) {
            documents.node { $0.identifier == "onecad.op.confirm" && $0.flags & ONECAD_IOS_A11Y_DISABLED == 0 } != nil
        }
    }

    private func assertSolid(_ summary: [String: Any], height: Double) throws {
        XCTAssertEqual(DocumentDriver.document(summary)["geometrySource"] as? String, "live")
        let bodies = try XCTUnwrap(summary["bodies"] as? [[String: Any]])
        XCTAssertEqual(bodies.count, 1)
        let body = try XCTUnwrap(bodies.first)
        XCTAssertEqual(body["healthy"] as? Bool, true)
        XCTAssertEqual(body["solids"] as? Int, 1)
        let min = try XCTUnwrap(body["min"] as? [Double])
        let max = try XCTUnwrap(body["max"] as? [Double])
        XCTAssertEqual(min.count, 3)
        XCTAssertEqual(max.count, 3)
        guard min.count == 3, max.count == 3 else { return }
        for (axis, expected) in [40.0, 60.0, abs(height)].enumerated() {
            XCTAssertEqual(max[axis] - min[axis], expected, accuracy: 0.01)
        }
        XCTAssertEqual(try XCTUnwrap(body["volume"] as? Double), 60 * 40 * abs(height), accuracy: 0.1)
        XCTAssertTrue((summary["features"] as? [[String: Any]] ?? []).allSatisfy { $0["status"] as? String == "ok" })
    }

}

@MainActor
private final class ModelingPencil {
    private let documents: DocumentDriver
    private let runID: String
    private var observations: [[String: Any]] = []
    private var contact = Int.random(in: 10_000_000...1_000_000_000)
    private var clock = CACurrentMediaTime()

    init(documents: DocumentDriver) throws {
        self.documents = documents
        runID = try XCTUnwrap(documents.hooks.runID, "synthetic Pencil QA requires ONECAD_RUN_ID")
    }

    func tap(_ point: CGPoint) async throws { try await drag([point, point]) }

    func drag(_ points: [CGPoint]) async throws {
        XCTAssertGreaterThanOrEqual(points.count, 2)
        contact += 1
        for (index, point) in points.enumerated() {
            let phase = index == 0 ? "began" : (index == points.count - 1 ? "ended" : "moved")
            clock = max(clock + 0.01, CACurrentMediaTime())
            let canvas = try XCTUnwrap(Self.canvas(), "no live Metal canvas")
            XCTAssertGreaterThan(canvas.bounds.width, canvas.bounds.height, "landscape only")
            let request: [String: Any] = ["cmd": "replay", "generation": canvas.viewGeneration, "samples": [[
                "contact": contact, "kind": "pencil", "phase": phase, "provenance": "actual",
                "t": clock, "x": Double(point.x), "y": Double(point.y),
            ]]]
            let reply = try documents.hooks.call(request)
            observations.append(["request": request, "response": reply])
            XCTAssertEqual(reply["synthetic"] as? Bool, true)
            XCTAssertEqual(reply["accepted"] as? Int, 1)
            XCTAssertEqual(reply["overflowed"] as? Bool, false)
            try await documents.test.nextRunLoopTick()
        }
    }

    func stableViewport() async throws -> CGRect {
        var previous: CGRect?
        var stable = 0
        try await documents.test.waitFor("landscape viewport stable across frames", timeout: 15) {
            guard let canvas = Self.canvas(), canvas.bounds.width > canvas.bounds.height,
                  let frame = self.documents.node({ $0.identifier == "onecad.viewport" })?.frame else { return false }
            stable = previous == frame ? stable + 1 : 0
            previous = frame
            return stable >= 5
        }
        return try XCTUnwrap(previous)
    }

    /// The fresh tab's default perspective and zero roll are restored on sketch exit.
    func project(_ point: SIMD3<Double>, summary: [String: Any], viewport: CGRect? = nil) throws -> CGPoint {
        let view = DocumentDriver.view(summary)
        let offset = try vector(view, "cameraOffset")
        let target = try vector(view, "cameraTarget")
        let back = simd_normalize(offset)
        let right = simd_normalize(SIMD3(-offset.y, offset.x, 0))
        let up = simd_cross(back, right)
        let relative = point - target
        let depth = simd_length(offset) - simd_dot(relative, back)
        XCTAssertGreaterThan(depth, 0)
        let frame = try viewport ?? XCTUnwrap(documents.node { $0.identifier == "onecad.viewport" }).frame
        let scale = Double(frame.height) / (2 * depth * tan(38 * .pi / 180))
        let result = CGPoint(x: frame.midX + simd_dot(relative, right) * scale,
                             y: frame.midY - simd_dot(relative, up) * scale)
        XCTAssertTrue(frame.insetBy(dx: 16, dy: 16).contains(result), "projected Pencil target outside usable viewport: \(result)")
        return result
    }

    private func vector(_ value: [String: Any], _ key: String) throws -> SIMD3<Double> {
        let array = try XCTUnwrap(value[key] as? [Double], "missing \(key)")
        guard array.count == 3 else { throw SemanticHooks.Failure(description: "invalid \(key): \(array)") }
        return SIMD3(array[0], array[1], array[2])
    }

    func record(_ name: String, summary: [String: Any]) {
        observations.append(["name": name, "summary": summary])
    }

    func seal() throws {
        let directory = try FileManager.default.url(for: .documentDirectory, in: .userDomainMask,
                                                    appropriateFor: nil, create: true)
            .appendingPathComponent("acceptance").appendingPathComponent(runID)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let trace: [String: Any] = ["schemaVersion": 1, "runId": runID, "synthetic": true,
                                   "inputCapabilities": ["pressure": "absent", "hover": "absent", "hardwareCapabilitiesFabricated": false],
                                   "kernelIdentity": try documents.hooks.kernelIdentity(), "observations": observations]
        try JSONSerialization.data(withJSONObject: trace, options: [.prettyPrinted, .sortedKeys])
            .write(to: directory.appendingPathComponent("pencil-modeling.trace"), options: .atomic)
        _ = try documents.hooks.call(["cmd": "seal_artifact", "name": "pencil-modeling.trace"])
    }

    private static func canvas() -> MetalCanvasView? {
        func find(_ view: UIView) -> MetalCanvasView? {
            if let canvas = view as? MetalCanvasView { return canvas }
            return view.subviews.lazy.compactMap(find).first
        }
        return UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).lazy.compactMap { find($0) }.first
    }
}
#endif
