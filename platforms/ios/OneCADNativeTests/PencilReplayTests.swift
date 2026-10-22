import QuartzCore
import UIKit
import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Synthetic simulator evidence only. Every replay traverses the production
/// normalizer and router; cancellation uses the public input ABI.
@MainActor
final class PencilReplayTests: XCTestCase {
    func testSyntheticPencilReplayPreservesInputAndDocumentContracts() async throws {
        let documents = try await DocumentDriver.attach(self)
        try await documents.closeEveryTab()
        _ = try await documents.newPart()
        try await documents.activate("onecad.tool.sketch")
        try await documents.activate("onecad.plane.xy")
        let model = ModelDriver(documents: documents)
        try await model.tool("line")
        let viewport = try XCTUnwrap(documents.node { $0.identifier == "onecad.viewport" }).frame
        let trace = try PencilReplayTrace(documents: documents, origin: CGPoint(x: viewport.midX - 120, y: viewport.midY + 100))
        defer { _ = onecad_ios_input_cancel_all(documents.hooks.app, ONECAD_IOS_CANCEL_SYSTEM_GESTURE) }

        try await trace.chronology()
        try await trace.replacePredictions()
        try await trace.refineEstimation()
        try await trace.cancelAndRejectLateEnd()
        try trace.seal()
        try await documents.activate("onecad.tool.discardSketch")
        try await documents.closeEveryTab()
    }
}

@MainActor
private final class PencilReplayTrace {
    private let documents: DocumentDriver
    private let origin: CGPoint
    private let contact = Int.random(in: 10_000_000...1_000_000_000)
    private let timestamp = CACurrentMediaTime()
    private let runID: String
    private var scenarios: [[String: Any]] = []
    private var baseline: [String: Any] = [:]
    private var initialInput: [String: Any] = [:]

    private var hooks: SemanticHooks { documents.hooks }
    private var test: XCTestCase { documents.test }
    private var actualPosition: [Double] { [Double(origin.x) + 20, Double(origin.y) + 20] }

    init(documents: DocumentDriver, origin: CGPoint) throws {
        self.documents = documents
        self.origin = origin
        runID = try XCTUnwrap(documents.hooks.runID, "Pencil replay requires the gate's run id")
    }

    func chronology() async throws {
        begin("chronology-coalesced")
        try cancel()
        baseline = try await document()
        initialInput = try state()
        XCTAssertEqual(initialInput["liveContacts"] as? Int, 0)
        var actual = sample("moved", time: 0.02, delta: 20, index: 102)
        actual["force"] = 0.25
        actual["max_force"] = 1.0
        actual["estimated"] = ["force"]
        actual["expects_update"] = ["force"]
        var duplicate = actual
        duplicate["provenance"] = "coalesced"
        try replay([
            actual,
            sample("moved", provenance: "coalesced", time: 0.01, delta: 10, index: 101),
            sample("began", time: 0, delta: 0, index: 100),
            duplicate,
            sample("moved", provenance: "predicted", time: 0.03, delta: 30),
        ])
        let observed = try state()
        XCTAssertEqual(observed["liveContacts"] as? Int, 1)
        XCTAssertEqual(observed["pencilPosition"] as? [Double], actualPosition)
        XCTAssertEqual(observed["prediction"] as? [Double], position(30))
        XCTAssertEqual(pose(observed)["force"] as? Double, 0.25)
        XCTAssertEqual(pose(observed)["estimated"] as? Int, 1)
        XCTAssertEqual(pose(observed)["expectsUpdate"] as? Int, 1)
        assertDelta("duplicateCoalesced", from: initialInput, to: observed, equals: 1)
        for name in ["invalid", "stale", "unknownContact", "oldGeneration", "refusedContacts"] {
            assertDelta(name, from: initialInput, to: observed, equals: 0)
        }
        try await assertUnchangedDocument()
    }

    func replacePredictions() async throws {
        begin("prediction-replacement")
        let before = try state()
        try replay([
            sample("moved", provenance: "predicted", time: 0.04, delta: 40),
            sample("moved", provenance: "predicted", time: 0.05, delta: 50),
        ])
        let first = try state()
        XCTAssertEqual(first["prediction"] as? [Double], position(50))
        try replay([sample("moved", provenance: "predicted", time: 0.06, delta: 60)])
        let replaced = try state()
        XCTAssertEqual(replaced["prediction"] as? [Double], position(60))
        XCTAssertEqual(replaced["pencilPosition"] as? [Double], actualPosition)
        XCTAssertEqual(replaced["liveContacts"] as? Int, 1)
        XCTAssertEqual(try canonical(replaced["pose"]), try canonical(before["pose"]))
        XCTAssertEqual(try canonical(replaced["stats"]), try canonical(before["stats"]))
        try await assertUnchangedDocument()
    }

    func refineEstimation() async throws {
        begin("estimation-refinement")
        let before = try state()
        var update = sample("moved", provenance: "estimation_update", time: 0.07, delta: 200, index: 102)
        update["force"] = 0.75
        update["max_force"] = 1.0
        update["estimated"] = [String]()
        update["expects_update"] = [String]()
        try replay([update])
        let refined = try state()
        XCTAssertEqual(pose(refined)["force"] as? Double, 0.75)
        XCTAssertEqual(pose(refined)["estimated"] as? Int, 0)
        XCTAssertEqual(pose(refined)["expectsUpdate"] as? Int, 0)
        XCTAssertEqual(refined["pencilPosition"] as? [Double], actualPosition)
        XCTAssertEqual(refined["prediction"] as? [Double], before["prediction"] as? [Double])
        XCTAssertEqual(refined["liveContacts"] as? Int, 1)
        assertDelta("estimationMatched", from: before, to: refined, equals: 1)
        assertDelta("estimationUnmatched", from: before, to: refined, equals: 0)
        try await assertUnchangedDocument()
    }

    func cancelAndRejectLateEnd() async throws {
        begin("cancellation-late-ended")
        let live = try state()
        XCTAssertEqual(live["liveContacts"] as? Int, 1)
        XCTAssertNotNil(live["prediction"] as? [Double])
        try cancel()
        let cancelled = try state()
        XCTAssertEqual(cancelled["liveContacts"] as? Int, 0)
        XCTAssertTrue(cancelled["prediction"] is NSNull)
        try replay([sample("ended", time: 0.08, delta: 120)])
        let late = try state()
        XCTAssertEqual(late["liveContacts"] as? Int, 0)
        XCTAssertTrue(late["prediction"] is NSNull)
        XCTAssertEqual(late["pencilPosition"] as? [Double], actualPosition)
        XCTAssertEqual(pose(late)["force"] as? Double, 0.75)
        assertDelta("oldGeneration", from: cancelled, to: late, equals: 1)
        try await assertUnchangedDocument()
    }

    func seal() throws {
        let directory = try XCTUnwrap(FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first)
            .appendingPathComponent("acceptance").appendingPathComponent(runID)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let record: [String: Any] = [
            "schemaVersion": 1, "runId": runID, "synthetic": true,
            "kernelIdentity": try hooks.kernelIdentity(), "scenarios": scenarios,
        ]
        let bytes = try JSONSerialization.data(withJSONObject: record, options: [.prettyPrinted, .sortedKeys])
        try bytes.write(to: directory.appendingPathComponent("pencil-replay.trace"), options: .atomic)
        _ = try hooks.call(["cmd": "seal_artifact", "name": "pencil-replay.trace"])
    }

    private func begin(_ name: String) {
        scenarios.append(["name": name, "observations": [[String: Any]]()])
    }

    private func record(_ request: [String: Any], _ response: [String: Any]) {
        var observations = scenarios[scenarios.count - 1]["observations"] as? [[String: Any]] ?? []
        observations.append(["request": request, "response": response])
        scenarios[scenarios.count - 1]["observations"] = observations
    }

    @discardableResult
    private func call(_ request: [String: Any]) throws -> [String: Any] {
        let response = try hooks.call(request)
        record(request, response)
        return response
    }

    private func state() throws -> [String: Any] {
        let response = try call(["cmd": "input_state", "contact": contact, "estimationIndex": 102])
        XCTAssertEqual(response["synthetic"] as? Bool, true)
        return response
    }

    private func replay(_ samples: [[String: Any]]) throws {
        let canvas = try XCTUnwrap(Self.canvas(), "no live Metal canvas")
        let response = try call(["cmd": "replay", "generation": canvas.viewGeneration, "samples": samples])
        XCTAssertEqual(response["synthetic"] as? Bool, true)
        XCTAssertEqual(response["accepted"] as? Int, samples.count)
        XCTAssertEqual(response["overflowed"] as? Bool, false)
    }

    private func cancel() throws {
        let reason = ONECAD_IOS_CANCEL_SYSTEM_GESTURE
        let status = onecad_ios_input_cancel_all(hooks.app, reason)
        record(["abi": "onecad_ios_input_cancel_all", "reason": reason], ["status": status.rawValue])
        XCTAssertEqual(status, ONECAD_IOS_OK)
        guard status == ONECAD_IOS_OK else { throw SemanticHooks.Failure(description: "input_cancel_all failed") }
    }

    private func document() async throws -> [String: Any] {
        try await test.nextRunLoopTick()
        let request: [String: Any] = ["cmd": "document_summary"]
        let response = try await hooks.run(request, in: test)
        record(request, response)
        return try XCTUnwrap(response["summary"] as? [String: Any])
    }

    private func assertUnchangedDocument() async throws {
        let observed = try await document()
        for key in ["document", "features", "sketches", "bodies"] {
            XCTAssertEqual(try canonical(observed[key]), try canonical(baseline[key]), "Pencil replay changed \(key)")
        }
    }

    private func assertDelta(_ name: String, from before: [String: Any], to after: [String: Any], equals expected: Int) {
        let previous = (before["stats"] as? [String: Int])?[name]
        let current = (after["stats"] as? [String: Int])?[name]
        XCTAssertNotNil(previous, "missing baseline counter \(name)")
        XCTAssertNotNil(current, "missing observed counter \(name)")
        if let previous, let current { XCTAssertEqual(current - previous, expected, "\(name) delta") }
    }

    private func pose(_ state: [String: Any]) -> [String: Any] { state["pose"] as? [String: Any] ?? [:] }

    private func canonical(_ value: Any?) throws -> Data {
        try JSONSerialization.data(withJSONObject: try XCTUnwrap(value), options: [.sortedKeys, .fragmentsAllowed])
    }

    private func position(_ delta: Double) -> [Double] { [Double(origin.x) + delta, Double(origin.y) + delta] }

    private func sample(_ phase: String, provenance: String = "actual", time: Double,
                        delta: Double, index: Int? = nil) -> [String: Any] {
        var sample: [String: Any] = [
            "contact": contact, "kind": "pencil", "phase": phase, "provenance": provenance,
            "t": timestamp + time, "x": position(delta)[0], "y": position(delta)[1],
        ]
        if let index { sample["estimation_index"] = index }
        return sample
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
