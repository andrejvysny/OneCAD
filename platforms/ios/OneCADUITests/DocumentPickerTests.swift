import XCTest

/// The system document pickers through the public UI (evidence class 4).
/// Providers beyond the simulator's local storage are physical-only
/// (ONECAD-469).
@MainActor
final class DocumentPickerTests: XCTestCase {
    override func setUp() async throws {
        continueAfterFailure = false
    }

    /// File › Import, then Cancel in the picker: no tab is added, the history
    /// is unchanged and Activity reports the cancellation.
    func testCancelledImportPickerLeavesTheDocumentUnchanged() {
        let app = launchOneCAD()
        discardRecoveryOffer(app)
        newPartWithSketch(app)
        tapFileMenu(app, OneCADID.fileImport)
        let cancel = app.navigationBars.buttons["Cancel"]
        waitUntil(cancel, .hittable, "import picker not shown")
        cancel.tap()
        waitUntil(cancel, .gone, "import picker still shown")
        waitUntil(app.element(id: OneCADID.tab(1)), .gone, "a tab is left from the cancelled import")
        XCTAssertEqual(app.element(id: OneCADID.historyRow(0)).label, "Sketch")
        XCTAssertFalse(app.element(id: OneCADID.historyRow(1)).exists)
        let activity = app.element(label: "Activity · Cancelled")
        waitUntil(activity, .hittable, "the cancellation is not reported")
        activity.tap()
        waitUntil(app.element(label: "File action cancelled"), .exists, "no cancellation message")
        app.element(id: OneCADID.canvas).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6)).tap()
        closeTabDiscarding(app)
    }

    /// A saved document is sent to Files through the export picker (asCopy);
    /// the document keeps its own sandbox file.
    func testSendACopyOfASavedDocumentOpensTheExportPicker() {
        let app = launchOneCAD()
        discardRecoveryOffer(app)
        newPartWithSketch(app)
        let name = "copy-\(UUID().uuidString.prefix(8).lowercased())"
        saveThroughPrompt(app, as: name)
        tapFileMenu(app, OneCADID.fileSendCopy)
        let cancel = app.navigationBars.buttons["Cancel"]
        waitUntil(cancel, .hittable, "export picker not shown")
        cancel.tap()
        waitUntil(cancel, .gone, "export picker still shown")
        XCTAssertEqual(app.element(id: OneCADID.tab(0)).label, name, "the document changed its file")
        closeTabDiscarding(app)
    }
}
