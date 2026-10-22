import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Three documents on one UIScene (IP17, ONECAD-464) against the real
/// in-process kernel: history, selection, camera and dirty state stay per
/// document; work started in one tab lands only there; a tab switch cancels
/// live touch contacts. Tabs are switched from the strip (`onecad.tab.<n>`).
@MainActor
final class MultiDocumentTests: XCTestCase {
    private var driver: DocumentDriver?
    private var artifacts: [URL] = []

    override func tearDown() async throws {
        try? await driver?.closeEveryTab()
        for url in artifacts {
            try? FileManager.default.removeItem(at: url)
            try? FileManager.default.removeItem(at: URL(fileURLWithPath: url.path + ".provenance.json"))
        }
    }

    private func start() async throws -> DocumentDriver {
        let docs = try await DocumentDriver.attach(self)
        driver = docs
        try await docs.closeEveryTab()
        return docs
    }

    private typealias Doc = DocumentDriver

    func testThreeDocumentsKeepIndependentHistorySelectionCameraAndDirtyState() async throws {
        let docs = try await start()
        _ = try await docs.openMacPlate(as: "isolation-plate")
        _ = try await docs.newPart()
        let third = try await docs.newPart()
        XCTAssertEqual(Doc.tabCount(third), 3)
        var cameras: [Int: [Double]] = [:]
        for index in [1, 2] { cameras[index] = Self.camera(try await docs.showTab(index)) }
        let plateCamera = Self.camera(try await docs.showTab(0))
        // Edit only the plate: a new sketch, then undo it (history moves and
        // the document stays dirty), a camera turn and a selection.
        let depth = Doc.document(try await docs.addSketch())["undoDepth"] as? Int ?? 0
        XCTAssertGreaterThan(depth, 0, "plate undo depth")
        try await docs.file("onecad.menu.edit.undo")
        let undone = try await docs.waitForSummary("plate undo") {
            Doc.document($0)["undoDepth"] as? Int == depth - 1 && !Doc.busy($0)
        }
        let history = Self.history(undone)
        try await docs.activate(label: "Orbit left 90°")
        try await docs.activate("checkbox.history")
        try await docs.activate("onecad.history.0")
        let plate = try await docs.waitForSummary("plate selection and camera") {
            Doc.view($0)["selected"] as? Int == 1 && Self.camera($0) != plateCamera
        }
        XCTAssertEqual(Doc.dirty(plate), true)
        for index in [1, 2] {
            let other = try await docs.showTab(index)
            XCTAssertEqual(Doc.features(other), [], "tab \(index) history")
            XCTAssertEqual(Doc.document(other)["undoDepth"] as? Int, 0, "tab \(index) undo")
            XCTAssertEqual(Doc.dirty(other), false, "tab \(index) dirty")
            XCTAssertEqual(Doc.view(other)["selected"] as? Int, 0, "tab \(index) selection")
            XCTAssertEqual(Self.camera(other), cameras[index], "tab \(index) camera")
        }
        let back = try await docs.showTab(0)
        XCTAssertEqual(Self.history(back), history, "the plate kept its history and redo")
        XCTAssertEqual(Doc.dirty(back), true)
        XCTAssertEqual(Doc.view(back)["selected"] as? Int, 1, "the plate kept its selection")
        XCTAssertEqual(Self.camera(back), Self.camera(plate), "the plate kept its camera")
    }

    /// Features with undo and redo depth.
    private static func history(_ summary: [String: Any]) -> [String] {
        let document = Doc.document(summary)
        return Doc.features(summary) + ["undo \(document["undoDepth"] ?? "-")", "redo \(document["redoDepth"] ?? "-")"]
    }

    private static func camera(_ summary: [String: Any]) -> [Double] {
        (Doc.view(summary)["cameraOffset"] as? [Double] ?? []) + (Doc.view(summary)["cameraTarget"] as? [Double] ?? [])
    }

    /// A Save and a STEP import started in one tab finish there: the strip
    /// refuses to switch while the owning document is busy, and neither
    /// result reaches another tab.
    func testPendingSaveAndImportLandOnlyInTheirOwnDocuments() async throws {
        let docs = try await start()
        _ = try await docs.openMacPlate(as: "pending-plate")
        let step = try await stagedStep(docs)
        _ = try await docs.newPart()
        _ = try await docs.showTab(0)
        let ticket = try docs.hooks.call(["cmd": "save_artifact", "name": "pending-save"])
        try await docs.activate("onecad.tab.1")
        let saved = try await docs.hooks.run(["cmd": "result", "ticket": ticket["ticket"] as Any], in: self)
        artifacts.append(URL(fileURLWithPath: try XCTUnwrap(saved["path"] as? String)))
        // Whether the strip refused the switch (save still running) or took it
        // after the save finished, the result belongs to the plate alone.
        let plate = try await docs.showTab(0)
        XCTAssertEqual(Doc.dirty(plate), false)
        XCTAssertEqual(Doc.title(plate), "pending-save")
        let other = try await docs.showTab(1)
        XCTAssertNil(Doc.document(other)["path"] as? String, "the save reached another document")
        docs.openPath(step)
        try await docs.activate("onecad.tab.0")
        let imported = try await docs.waitForSummary("import finished") {
            Doc.tabCount($0) == 3 && !Doc.busy($0) && !Doc.features($0).isEmpty
        }
        XCTAssertEqual(Doc.view(imported)["tabIndex"] as? Int, 2, "the import kept its own tab")
        XCTAssertEqual((imported["bodies"] as? [Any])?.count, 1)
        let first = try await docs.showTab(0)
        XCTAssertEqual(Doc.features(first), ["Sketch", "Extrude"])
        let second = try await docs.showTab(1)
        XCTAssertEqual(Doc.features(second), [])
    }

    /// A Pencil stroke still down when a tab is chosen is cancelled with
    /// DocumentSwitch: its prediction is dropped and its later lift commits
    /// nothing in the document now shown.
    func testATabSwitchCancelsALiveTouchContact() async throws {
        let docs = try await start()
        _ = try await docs.openMacPlate(as: "switch-plate")
        let fresh = try await docs.newPart()
        let canvas = try XCTUnwrap(docs.node { $0.identifier == "onecad.viewport" }?.frame)
        let (x, y) = (canvas.midX, canvas.midY + 120)
        _ = try docs.hooks.call(["cmd": "replay", "samples": [
            ["contact": 41, "phase": "began", "t": 1.0, "x": x, "y": y],
            ["contact": 41, "phase": "moved", "t": 1.02, "x": x + 30, "y": y],
            ["contact": 41, "phase": "moved", "provenance": "predicted", "t": 1.04, "x": x + 45, "y": y],
        ]])
        XCTAssertNotNil(try prediction(docs, 41), "the stroke is live")
        try await docs.activate("onecad.tab.0")
        try await waitFor("contact cancelled by the switch", timeout: 15) {
            try self.prediction(docs, 41) == nil
        }
        _ = try docs.hooks.call(["cmd": "replay", "samples": [
            ["contact": 41, "phase": "ended", "t": 1.1, "x": x + 60, "y": y],
        ]])
        let plate = try await docs.waitForSummary("plate shown") { Doc.view($0)["tabIndex"] as? Int == 0 }
        XCTAssertEqual(Doc.features(plate), ["Sketch", "Extrude"])
        XCTAssertEqual(Doc.dirty(plate), false)
        let left = try await docs.showTab(1)
        XCTAssertEqual(Doc.features(left), Doc.features(fresh))
        XCTAssertEqual(Doc.dirty(left), false)
    }

    private func prediction(_ docs: DocumentDriver, _ contact: Int) throws -> [Double]? {
        try docs.hooks.call(["cmd": "prediction", "contact": contact])["prediction"] as? [Double]
    }

    /// A STEP of the plate, written by the kernel and moved out of the run
    /// directory into Documents/Inbox, as Files would hand it over.
    private func stagedStep(_ docs: DocumentDriver) async throws -> URL {
        let exported = try await docs.hooks.exportStepArtifact("pending-import", in: self)
        let source = URL(fileURLWithPath: try XCTUnwrap(exported["path"] as? String))
        try? FileManager.default.removeItem(at: URL(fileURLWithPath: source.path + ".provenance.json"))
        let target = try DocumentInbox.inboxDirectory().appending(path: "pending-\(UUID().uuidString.prefix(8)).step")
        try FileManager.default.moveItem(at: source, to: target)
        artifacts.append(target)
        return target
    }
}
#else
/// Makes a non-Acceptance run visibly skip instead of passing with 0 tests.
final class MultiDocumentTests: XCTestCase {
    func testThreeDocumentsKeepIndependentHistorySelectionCameraAndDirtyState() throws {
        throw XCTSkip("semantic test hooks exist only in the Acceptance configuration")
    }

    func testPendingSaveAndImportLandOnlyInTheirOwnDocuments() throws {
        throw XCTSkip("semantic test hooks exist only in the Acceptance configuration")
    }

    func testATabSwitchCancelsALiveTouchContact() throws {
        throw XCTSkip("semantic test hooks exist only in the Acceptance configuration")
    }
}
#endif
