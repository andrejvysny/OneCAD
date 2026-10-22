import XCTest

/// File › Save All across document tabs through the public UI (evidence
/// class 4): every dirty untitled document is shown and asked for a name in
/// turn, and a cancelled prompt is reported with what was left unsaved.
@MainActor
final class SaveAllTests: XCTestCase {
    override func setUp() async throws {
        continueAfterFailure = false
    }

    func testSaveAllAsksForEachUntitledDocumentAndReportsACancel() {
        let app = launchOneCAD()
        discardRecoveryOffer(app)
        newPartWithSketch(app)
        newPartWithSketch(app)
        waitUntil(app.element(id: OneCADID.tab(1)), .exists, "second document has no tab")
        tapFileMenu(app, OneCADID.fileSaveAll)
        let name = "saveall-\(UUID().uuidString.prefix(8).lowercased())"
        let field = app.textFields[OneCADID.saveName]
        waitUntil(field, .hittable, "Save All asked for no name")
        field.tap()
        field.buttons["Clear text"].tap()
        field.typeText(name)
        app.alerts.buttons["Save"].tap()
        // The second untitled document is shown and asked next.
        waitUntil(app.alerts.buttons["Cancel"], .hittable, "Save All skipped the second document")
        app.alerts.buttons["Cancel"].tap()
        let saved = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", name),
                                              object: app.element(id: OneCADID.tab(0)))
        XCTAssertEqual(XCTWaiter().wait(for: [saved], timeout: 30), .completed, "first document not saved")
        XCTAssertEqual(app.element(id: OneCADID.tab(1)).label, "Untitled")
        let activity = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH 'Activity'")).firstMatch
        waitUntil(activity, .hittable, "no Activity control")
        activity.tap()
        let report = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH 'Save All left unsaved: Untitled (cancelled)'")).firstMatch
        waitUntil(report, .exists, "the cancel is not reported")
        app.element(id: OneCADID.canvas).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6)).tap()
        closeTabDiscarding(app, remaining: 1)
        closeTabDiscarding(app)
    }
}
