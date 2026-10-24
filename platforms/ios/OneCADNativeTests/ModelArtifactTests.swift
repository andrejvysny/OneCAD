import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Saves UI-authored simulator models for the independent Mac geometry oracle.
/// Measurements come from the linked in-process kernel, not cached geometry.
@MainActor
final class ModelArtifactTests: XCTestCase {
    private var driver: DocumentDriver?
    private let plateVolume = 9600 - 36 * Double.pi

    override func setUp() async throws { continueAfterFailure = false }

    override func tearDown() async throws { try? await driver?.closeEveryTab() }

    private func start() async throws -> ModelDriver {
        let documents = try await DocumentDriver.attach(self)
        driver = documents
        try await documents.closeEveryTab()
        let runID = try XCTUnwrap(documents.hooks.runID, "ONECAD_RUN_ID (TEST_RUNNER_ONECAD_RUN_ID) not set")
        XCTAssertFalse(runID.isEmpty)
        return ModelDriver(documents: documents)
    }

    func testPlateArtifact() async throws {
        let model = try await start()
        let summary: [String: Any]
        do { summary = try await model.authorPlate() }
        catch {
            model.documents.diagnostic("plate authoring failed: \(error)")
            throw error
        }
        try assertModel(summary, operations: ["Sketch", "Extrude"], decrease: nil)
        try await save("plate", model: model)
        try await exerciseOffsetAndTransform(model, baseline: summary)
    }

    func testFilletedPlateArtifact() async throws {
        let model = try await start()
        try assertModel(try await model.authorPlate(), operations: ["Sketch", "Extrude"], decrease: nil)
        let summary = try await model.modifyVerticalEdge("Fillet")
        try assertModel(summary, operations: ["Sketch", "Extrude", "Fillet"], decrease: 4 - .pi)
        try await save("plate-fillet", model: model)
    }

    func testChamferedPlateArtifact() async throws {
        let model = try await start()
        try assertModel(try await model.authorPlate(), operations: ["Sketch", "Extrude"], decrease: nil)
        let summary = try await model.modifyVerticalEdge("Chamfer")
        try assertModel(summary, operations: ["Sketch", "Extrude", "Chamfer"], decrease: 2)
        try await save("plate-chamfer", model: model)
    }

    private func exerciseOffsetAndTransform(_ model: ModelDriver, baseline: [String: Any]) async throws {
        let original = DocumentDriver.document(baseline)
        let cancelled = try await model.offsetTopFace(cancel: true)
        try assertModel(cancelled, operations: ["Sketch", "Extrude"], decrease: nil)
        XCTAssertEqual(DocumentDriver.document(cancelled)["revision"] as? Int, original["revision"] as? Int)
        XCTAssertEqual(DocumentDriver.document(cancelled)["undoDepth"] as? Int, original["undoDepth"] as? Int)
        let offset = try await model.offsetTopFace(cancel: false)
        try assertModel(offset, operations: ["Sketch", "Extrude", "OffsetFace"], decrease: nil,
                        height: 8, volumeMultiplier: 2)
        try assertBounds(offset, min: [-40, 0, 0], max: [0, 60, 8])
        XCTAssertEqual(DocumentDriver.document(offset)["undoDepth"] as? Int,
                       (original["undoDepth"] as? Int).map { $0 + 1 })
        try assertModel(try await model.undoModelOperation(), operations: ["Sketch", "Extrude"], decrease: nil)
        let transformed = try await model.transformBody()
        try assertModel(transformed, operations: ["Sketch", "Extrude", "TransformBody"], decrease: nil)
        try assertBounds(transformed, min: [-40, 15, 0], max: [20, 55, 4])
        XCTAssertEqual(DocumentDriver.document(transformed)["undoDepth"] as? Int,
                       (original["undoDepth"] as? Int).map { $0 + 1 })
        let restored = try await model.undoModelOperation()
        try assertModel(restored, operations: ["Sketch", "Extrude"], decrease: nil)
        try assertBounds(restored, min: [-40, 0, 0], max: [0, 60, 4])
        XCTAssertEqual(DocumentDriver.document(restored)["undoDepth"] as? Int, original["undoDepth"] as? Int)
    }

    private func assertBounds(_ summary: [String: Any], min expectedMin: [Double], max expectedMax: [Double]) throws {
        let body = try XCTUnwrap((summary["bodies"] as? [[String: Any]])?.first)
        let min = try XCTUnwrap(body["min"] as? [Double])
        let max = try XCTUnwrap(body["max"] as? [Double])
        XCTAssertEqual(min.count, 3)
        XCTAssertEqual(max.count, 3)
        guard min.count == 3, max.count == 3 else { return }
        for axis in 0..<3 {
            XCTAssertEqual(min[axis], expectedMin[axis], accuracy: 1e-5)
            XCTAssertEqual(max[axis], expectedMax[axis], accuracy: 1e-5)
        }
    }

    private func assertModel(_ summary: [String: Any], operations: [String], decrease: Double?,
                             height: Double = 4, volumeMultiplier: Double = 1) throws {
        XCTAssertEqual(DocumentDriver.features(summary), operations)
        XCTAssertEqual(DocumentDriver.document(summary)["geometrySource"] as? String, "live")
        let features = try XCTUnwrap(summary["features"] as? [[String: Any]])
        XCTAssertTrue(features.allSatisfy { $0["status"] as? String == "ok" }, "\(features)")
        let bodies = try XCTUnwrap(summary["bodies"] as? [[String: Any]])
        XCTAssertEqual(bodies.count, 1, "\(bodies)")
        let body = try XCTUnwrap(bodies.first)
        XCTAssertEqual(body["healthy"] as? Bool, true)
        XCTAssertEqual(body["solids"] as? Int, 1)
        let volume = try XCTUnwrap(body["volume"] as? Double)
        XCTAssertTrue(volume.isFinite)
        if let decrease {
            XCTAssertLessThan(volume, plateVolume)
            XCTAssertEqual(plateVolume - volume, decrease, accuracy: decrease * 1e-4)
        } else {
            XCTAssertEqual(volume, plateVolume * volumeMultiplier, accuracy: plateVolume * volumeMultiplier * 1e-6)
        }
        try assertExtents(body, height: height)
        let sketches = try XCTUnwrap(summary["sketches"] as? [[String: Any]])
        XCTAssertEqual(sketches.count, 1)
        let sketch = try XCTUnwrap(sketches.first)
        XCTAssertEqual(sketch["status"] as? String, "FullyConstrained")
        XCTAssertEqual(sketch["dof"] as? Int, 0)
        XCTAssertEqual(sketch["entities"] as? Int, 12)
        XCTAssertEqual(sketch["constraints"] as? Int, 12)
    }

    private func assertExtents(_ body: [String: Any], height: Double) throws {
        let min = try XCTUnwrap(body["min"] as? [Double])
        let max = try XCTUnwrap(body["max"] as? [Double])
        XCTAssertEqual(min.count, 3)
        XCTAssertEqual(max.count, 3)
        guard min.count == 3, max.count == 3 else { return }
        let xy = [max[0] - min[0], max[1] - min[1]].sorted()
        for (axis, expected) in [(0, 40.0), (1, 60.0)] {
            let rounding = (0..<2).map { Double(Float(min[$0]).ulp + Float(max[$0]).ulp) / 2 }.max() ?? 0
            XCTAssertEqual(xy[axis], expected, accuracy: 1e-7 + 1e-9 * expected + rounding)
        }
        let rounding = Double(Float(min[2]).ulp + Float(max[2]).ulp) / 2
        XCTAssertEqual(max[2] - min[2], height, accuracy: 1e-7 + height * 1e-9 + rounding)
    }

    private func save(_ name: String, model: ModelDriver) async throws {
        let hooks = model.documents.hooks
        let runID = try XCTUnwrap(hooks.runID)
        let saved = try await hooks.saveArtifact(name, in: self)
        XCTAssertEqual(saved["saved"] as? Bool, true, "\(saved)")
        XCTAssertEqual(saved["clean"] as? Bool, true, "\(saved)")
        XCTAssertEqual(saved["runId"] as? String, runID)
        let file = URL(fileURLWithPath: try XCTUnwrap(saved["path"] as? String))
        XCTAssertEqual(file.lastPathComponent, "\(name).onecad")
        XCTAssertEqual(file.deletingLastPathComponent().lastPathComponent, runID)
        let provenance = try readJSON(URL(fileURLWithPath: file.path + ".provenance.json"))
        XCTAssertEqual(provenance["kind"] as? String, "document")
        XCTAssertEqual(provenance["runId"] as? String, runID)
        XCTAssertEqual(provenance["artifact"] as? String, file.lastPathComponent)
        XCTAssertEqual(provenance["sha256"] as? String, try DocumentDriver.sha256(file))
        try assertKernel(provenance)
        let written = try await hooks.writeSemantic(name, in: self)
        let semanticFile = URL(fileURLWithPath: try XCTUnwrap(written["path"] as? String))
        XCTAssertEqual(semanticFile.deletingLastPathComponent(), file.deletingLastPathComponent())
        XCTAssertEqual(semanticFile.lastPathComponent, "\(name).json")
        let semantic = try readJSON(semanticFile)
        XCTAssertEqual(semantic["kind"] as? String, "semantic")
        XCTAssertEqual(semantic["runId"] as? String, runID)
        XCTAssertEqual(semantic["name"] as? String, name)
        try assertKernel(semantic)
        let summary = try XCTUnwrap(semantic["summary"] as? [String: Any])
        XCTAssertEqual(DocumentDriver.document(summary)["path"] as? String, file.path)
        XCTAssertEqual(DocumentDriver.dirty(summary), false)
    }

    private func assertKernel(_ record: [String: Any]) throws {
        let identity = try XCTUnwrap(record["kernelIdentity"] as? [String: Any])
        XCTAssertEqual(identity["backend"] as? String, "in-process")
        XCTAssertTrue(identity["relaunchRequired"] is NSNull, "\(identity)")
        let embedded = try XCTUnwrap(identity["embedded"] as? [String: Any])
        let linked = try XCTUnwrap(identity["linked"] as? [String: Any])
        XCTAssertTrue(NSDictionary(dictionary: embedded).isEqual(to: linked), "linked kernel differs from embedded identity")
        XCTAssertEqual(embedded["platform"] as? String, "iphonesimulator")
        XCTAssertEqual(embedded["testHooks"] as? Bool, true)
    }

    private func readJSON(_ url: URL) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
    }
}
#else
final class ModelArtifactTests: XCTestCase {
    func testPlateArtifact() throws { throw XCTSkip("semantic test hooks exist only in the Acceptance configuration") }
    func testFilletedPlateArtifact() throws { throw XCTSkip("semantic test hooks exist only in the Acceptance configuration") }
    func testChamferedPlateArtifact() throws { throw XCTSkip("semantic test hooks exist only in the Acceptance configuration") }
}
#endif
