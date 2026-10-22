import XCTest

/// Document flows through the public UI only (docs/ios/TESTING.md). Labels
/// are matched where the Rust snapshot has no automation id (recovery dialog,
/// recents cards, system pickers and alerts).
extension XCTestCase {
    /// A recovery offer left by an earlier process is discarded, so a test
    /// starts from Home without someone else's unsaved work.
    @MainActor
    func discardRecoveryOffer(_ app: XCUIApplication) {
        let discard = app.element(label: "Discard recovery")
        if discard.waitForExistence(timeout: 3) {
            discard.tap()
            waitUntil(discard, .gone, "recovery offer still shown")
        }
    }

    /// File › New Part, then an empty sketch on XY: a document whose History
    /// shows one "Sketch" row.
    @MainActor
    func newPartWithSketch(_ app: XCUIApplication) {
        tapFileMenu(app, OneCADID.fileNew)
        waitUntil(app.element(id: OneCADID.tool("sketch")), .hittable, "no new part")
        app.element(id: OneCADID.tool("sketch")).tap()
        waitUntil(app.element(id: OneCADID.planeXY), .hittable, "no plane chooser")
        app.element(id: OneCADID.planeXY).tap()
        waitUntil(app.element(id: OneCADID.finishSketch), .hittable, "sketch not entered")
        app.element(id: OneCADID.finishSketch).tap()
        showHistory(app)
        waitUntil(app.element(id: OneCADID.historyRow(0)), .exists, "no Sketch history row")
    }

    @MainActor
    func showHistory(_ app: XCUIApplication) {
        let history = app.element(id: "checkbox.history")
        waitUntil(history, .hittable, "no History tab in the inspector")
        history.tap()
    }

    @MainActor
    func tapFileMenu(_ app: XCUIApplication, _ item: String) {
        waitUntil(app.element(id: OneCADID.fileMenu), .hittable, "File menu")
        app.element(id: OneCADID.fileMenu).tap()
        let row = app.element(id: item)
        waitUntil(row, .hittable, "\(item) not offered")
        row.tap()
    }

    /// File › Save through the native name prompt (SAVE_DOCUMENT); the tab
    /// then carries the name.
    @MainActor
    func saveThroughPrompt(_ app: XCUIApplication, as name: String) {
        tapFileMenu(app, OneCADID.fileSave)
        let field = app.textFields[OneCADID.saveName]
        waitUntil(field, .hittable, "no save name prompt")
        field.tap()
        field.buttons["Clear text"].tap()
        field.typeText(name)
        app.alerts.buttons["Save"].tap()
        waitUntil(app.element(id: OneCADID.tab(0)), .exists, "no document tab")
        let named = NSPredicate(format: "label == %@", name)
        let titled = XCTNSPredicateExpectation(predicate: named, object: app.element(id: OneCADID.tab(0)))
        XCTAssertEqual(XCTWaiter().wait(for: [titled], timeout: 30), .completed, "tab never showed \(name)")
    }

    /// Closes the active tab, leaving `remaining` tabs; unsaved work is
    /// discarded in the tab's prompt.
    @MainActor
    func closeTabDiscarding(_ app: XCUIApplication, remaining: Int = 0) {
        tapFileMenu(app, OneCADID.fileCloseTab)
        let discard = app.element(label: "Don't save")
        if discard.waitForExistence(timeout: 3) { discard.tap() }
        waitUntil(app.element(id: OneCADID.tab(remaining)), .gone, "tab still open")
    }
}

extension XCUIApplication {
    func element(label: String) -> XCUIElement {
        descendants(matching: .any).matching(NSPredicate(format: "label == %@", label)).firstMatch
    }
}

extension OneCADID {
    static let fileNew = "onecad.menu.file.new"
    static let fileSave = "onecad.menu.file.save"
    static let fileSaveAll = "onecad.menu.file.saveAll"
    static let fileCloseTab = "onecad.menu.file.closeTab"
    static let fileImport = "onecad.menu.file.import"
    static let fileSendCopy = "onecad.menu.file.sendCopy"
    static let saveName = "onecad.save.name"
    static let planeXY = "onecad.plane.xy"
    static let finishSketch = "onecad.tool.finishSketch"

    static func tool(_ name: String) -> String { "onecad.tool.\(name)" }
    static func tab(_ index: Int) -> String { "onecad.tab.\(index)" }
    static func historyRow(_ index: Int) -> String { "onecad.history.\(index)" }
}
