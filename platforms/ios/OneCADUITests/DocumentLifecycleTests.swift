import XCTest

/// Documents across processes through the public UI (evidence class 4). The
/// app is ended with `XCUIApplication.terminate()`: process termination in
/// the simulator, never jetsam evidence (docs/ios/TESTING.md).
@MainActor
final class DocumentLifecycleTests: XCTestCase {
    override func setUp() async throws {
        continueAfterFailure = false
    }

    /// Create content, save it through the name prompt, terminate, relaunch
    /// and reopen it from Home's recents in the fresh process.
    func testSavedDocumentReopensFromRecentsInAFreshProcess() {
        let app = launchOneCAD()
        discardRecoveryOffer(app)
        newPartWithSketch(app)
        let name = "reopen-\(UUID().uuidString.prefix(8).lowercased())"
        saveThroughPrompt(app, as: name)
        app.terminate()
        app.launch()
        waitUntil(app.element(id: OneCADID.canvas), .exists, "canvas after relaunch")
        let recent = app.element(label: name)
        waitUntil(recent, .hittable, "\(name) not in recents after relaunch")
        recent.tap()
        waitUntil(app.element(id: OneCADID.tab(0)), .exists, "reopened document has no tab")
        XCTAssertEqual(app.element(id: OneCADID.tab(0)).label, name)
        showHistory(app)
        let row = app.element(id: OneCADID.historyRow(0))
        waitUntil(row, .exists, "no history after reopening")
        XCTAssertEqual(row.label, "Sketch")
        XCTAssertFalse(app.element(id: OneCADID.historyRow(1)).exists, "history grew on reopen")
        closeTabDiscarding(app)
    }

    /// Unsaved work survives a process termination: backgrounding flushes the
    /// autosave, and the next launch offers it; Restore brings the history
    /// back as an unsaved document.
    func testUnsavedWorkIsOfferedAfterProcessTermination() {
        let app = launchOneCAD()
        discardRecoveryOffer(app)
        newPartWithSketch(app)
        // Backgrounding requests the autosave flush (DID_ENTER_BACKGROUND);
        // the app is brought back before it is terminated, so the flush has
        // had the round trip to finish, and the relaunch proves it did.
        XCUIDevice.shared.press(.home)
        waitFor(app, .runningBackground, .runningBackgroundSuspended)
        app.activate()
        waitFor(app, .runningForeground)
        waitUntil(app.element(id: OneCADID.fileMenu), .hittable, "app not back in the foreground")
        app.terminate()
        app.launch()
        let restore = app.element(label: "Restore")
        waitUntil(restore, .hittable, "no recovery offer after termination")
        restore.tap()
        showHistory(app)
        let row = app.element(id: OneCADID.historyRow(0))
        waitUntil(row, .exists, "recovered document has no history")
        XCTAssertEqual(row.label, "Sketch")
        closeTabDiscarding(app)
    }

    private func waitFor(_ app: XCUIApplication, _ states: XCUIApplication.State...) {
        let raw = states.map { NSNumber(value: $0.rawValue) }
        let reached = XCTNSPredicateExpectation(predicate: NSPredicate(format: "state IN %@", raw), object: app)
        XCTAssertEqual(XCTWaiter().wait(for: [reached], timeout: 30), .completed, "app never reached \(states)")
    }
}
