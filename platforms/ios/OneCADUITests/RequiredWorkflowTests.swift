import XCTest

/// Required part workflow through public XCUITest only (evidence class 4,
/// docs/ios/ARCHITECTURE.md): taps and coordinate taps on the canvas, the
/// system software keyboard for egui fields, `typeText` for UIKit fields.
/// No test hooks, no synthesized HID or touch events.
///
/// Canvas coordinates assume the iPad Pro 13-inch simulator in portrait with
/// the default camera of a new document (docs/ios/TESTING.md).
@MainActor
final class RequiredWorkflowTests: XCTestCase {
    /// Sketch-plane millimetres to screen points under the default sketch
    /// camera, measured on iPad Pro 13-inch (M5) portrait: 60 mm = 77 pt.
    static let pointsPerMillimetre: CGFloat = 1.285

    private var app: XCUIApplication!
    private var ui: OneCADScreen!
    private var keyboard: SoftwareKeyboard!
    private var origin = CGPoint.zero

    override func setUp() async throws {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
    }

    /// Failures carry the screen and the accessibility tree they saw.
    override func record(_ issue: XCTIssue) {
        var issue = issue
        if let app, app.state == .runningForeground {
            issue.add(XCTAttachment(screenshot: app.screenshot()))
            issue.add(XCTAttachment(string: app.debugDescription))
        }
        super.record(issue)
    }

    func testPlateWithHoleEditUndoSaveReopen() {
        app = launchOneCAD()
        ui = OneCADScreen(app: app, test: self)
        keyboard = SoftwareKeyboard(app: app, test: self)
        let name = "Plate-" + documentSuffix()

        step("01-new-document") {
            ui.waitStartPage()
            ui.tap(UI.newPart)
            let viewport = ui.wait(UI.viewport, .exists)
            ui.wait(UI.sketch)
            // A fresh document's camera centres the world origin in the viewport.
            origin = CGPoint(x: viewport.frame.midX, y: viewport.frame.midY)
            XCTAssertFalse(ui.text("Body 1").exists, "a new document has no body")
        }
        step("02-sketch-on-xy") {
            ui.tap(UI.sketch)
            ui.wait(UI.planeChooser, .exists)
            // The XY plane quad covers the area just below the origin in the default view.
            ui.point(origin.x, origin.y + 41).tap()
            ui.wait(UI.finish)
            // The inspector tab is a persisted preference; show Properties.
            ui.tap(UI.propertiesTab)
            ui.waitText("Sketch · Fully constrained")
            ui.waitText("Entities: 0")
        }
        step("03-rectangle-60x40") {
            ui.tap(UI.moreTools)
            ui.tap(UI.rectangle)
            sketchPoint(0, 0).tap()
            enterNumeric([(UI.width, "60"), (UI.height, "40")])
            ui.waitText("Distance (mm): 60")
            ui.waitText("Distance (mm): 40")
            ui.waitText("Fixed")
            ui.waitText("Sketch · Fully constrained")
            ui.waitText("Entities: 8")
            ui.waitText("Constraints: 7")
        }
        step("04-finish-sketch") {
            ui.tap(UI.finish)
            ui.wait(UI.sketch)
            ui.waitText("1 closed region(s) returned")
        }
        step("05-extrude-4mm") {
            sketchPoint(30, 20).tap()
            ui.waitText("Solve: 0 degrees of freedom")
            ui.tap(UI.extrude)
            setExtrudeDistance("4")
            // Enabled only once the preview for this distance is ready.
            ui.wait(UI.create, .enabled).tap()
            ui.waitText("Body 1")
            ui.expectHistory(["Sketch", "Extrude"])
        }
        step("06-hole-sketch-on-top-face") {
            ui.tap(UI.sketch)
            ui.wait(UI.planeChooser, .exists)
            // Top view: the plate's top face is the only face under its centre.
            sketchPoint(30, 20).tap()
            ui.wait(UI.finish)
            ui.tap(UI.propertiesTab)
            ui.tap(UI.point)
            ui.tap(UI.numericInput)
            enterNumeric([(UI.x, "30"), (UI.y, "20")], open: false)
            ui.waitText("Horizontal distance (mm): 30")
            ui.waitText("Vertical distance (mm): 20")
            ui.tap(UI.moreTools)
            ui.tap(UI.circle)
            // Snaps to the point just placed: the centre becomes coincident.
            sketchPoint(30, 20).tap()
            enterNumeric([(UI.diameter, "6")])
            ui.waitText("Coincident")
            ui.waitText("Diameter (mm): 6")
            ui.waitText("Sketch · Fully constrained")
            ui.tap(UI.finish)
            ui.wait(UI.sketch)
            ui.waitText("2 closed region(s) returned")
        }
        step("07-cut-through") {
            // The circle's disk: the outer region excludes it.
            sketchPoint(30, 20).tap()
            ui.tap(UI.extrude)
            let operation = ui.control(stableID: "onecad.param.operation", value: "New Body")
            waitUntil(operation, .hittable, "extrude Operation selector")
            operation.tap()
            ui.tap(UI.cutMode)
            ui.tap(UI.bodyRow)
            setExtrudeDistance("5")
            ui.tap(UI.flip)
            ui.waitText("Reverse")
            ui.wait(UI.create, .enabled).tap()
            ui.expectHistory(["Sketch", "Extrude", "Sketch", "Extrude"])
            ui.tap(UI.propertiesTab)
            ui.tap(UI.bodyRow)
            ui.waitText("Health: Healthy")
        }
        step("08-edit-width-70") {
            editPlateWidth(from: "60", to: "70")
            ui.expectHistory(["Sketch", "Extrude", "Sketch", "Extrude"])
            ui.tap(UI.propertiesTab)
            ui.tap(UI.bodyRow)
            ui.waitText("Health: Healthy")
        }
        step("09-undo-redo") {
            // Undo restores the 60 mm width, a second undo removes the cut.
            ui.tap(UI.undo)
            expectPlateWidth("60")
            ui.tap(UI.undo)
            ui.expectHistory(["Sketch", "Extrude", "Sketch"])
            ui.tap(UI.redo)
            ui.expectHistory(["Sketch", "Extrude", "Sketch", "Extrude"])
            expectPlateWidth("60")
        }
        step("10-save") {
            ui.tap(UI.fileMenu)
            ui.tap(UI.save)
            let field = app.textFields[OneCADID.saveName]
            waitUntil(field, .hittable, "save name prompt")
            field.tap()
            let current = field.value as? String ?? ""
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count) + name)
            app.alerts.buttons["Save"].tap()
            waitUntil(field, .gone, "save name prompt dismissed")
            ui.wait(UIRef("button.\(name.lowercased())", label: name), .exists)
        }
        step("11-relaunch-and-reopen") {
            app.terminate()
            app.launch()
            waitUntil(app.element(id: OneCADID.canvas), .exists, "canvas after relaunch")
            ui.waitStartPage()
            ui.tap(UIRef(label: name))
            ui.wait(UI.viewport, .exists)
            ui.expectHistory(["Sketch", "Extrude", "Sketch", "Extrude"])
            ui.waitText("Body 1")
            ui.tap(UI.propertiesTab)
            ui.tap(UI.bodyRow)
            ui.waitText("Health: Healthy")
            expectPlateWidth("60")
        }
    }

    // MARK: Steps

    /// One workflow step: its actions and assertions, then a kept screenshot.
    private func step(_ name: String, _ body: () -> Void) {
        XCTContext.runActivity(named: name) { _ in
            body()
            ui.screenshot(name)
        }
    }

    private func documentSuffix() -> String {
        let run = ProcessInfo.processInfo.environment["ONECAD_RUN_ID"] ?? UUID().uuidString
        return String(run.filter { $0.isLetter || $0.isNumber }.suffix(8))
    }

    /// Screen point of sketch coordinates (mm) under the default camera.
    private func sketchPoint(_ x: CGFloat, _ y: CGFloat) -> XCUICoordinate {
        let scale = Self.pointsPerMillimetre
        return ui.point(origin.x + x * scale, origin.y - y * scale)
    }

    /// Opens numeric creation (unless already open) and fills `fields` in
    /// order with Tab between them, committing with Return.
    private func enterNumeric(_ fields: [(UIRef, String)], open: Bool = true) {
        if open { ui.tap(UI.numericInput) }
        keyboard.waitShown()
        for (index, (ref, text)) in fields.enumerated() {
            if index > 0 { keyboard.tab() }
            let field = ui.wait(ref, .exists)
            keyboard.replace(field.value as? String ?? "", with: text)
            waitUntil(field, .value(text), "\(ref) shows \(text)")
        }
        keyboard.done()
    }

    private func setExtrudeDistance(_ text: String) {
        let field = ui.wait(UI.extrudeDistance)
        field.tap()
        keyboard.waitShown()
        keyboard.replace(field.value as? String ?? "", with: text)
        waitUntil(field, .value(text), "extrude distance shows \(text)")
    }

    /// Opens the plate sketch from its History row (feature 1).
    private func editFirstSketch() {
        ui.tap(UI.historyTab)
        waitUntil(ui.nth("Extrude", 0), .exists, "history rows listed")
        let rows = ui.historyRowElements()
        XCTAssertEqual(rows.first?.label, "Sketch", "first history row")
        rows.first?.tap()
        ui.tap(UI.editFeature)
        ui.wait(UI.finish)
        ui.tap(UI.propertiesTab)
    }

    /// Re-enters the first sketch and drives its width dimension through the
    /// constraint list's Edit field.
    private func editPlateWidth(from old: String, to new: String) {
        editFirstSketch()
        let row = ui.waitText("Distance (mm): \(old)")
        let edit = ui.editButton(onRow: row)
        waitUntil(edit, .hittable, "Edit beside the \(old) mm width")
        edit.tap()
        keyboard.waitShown()
        let field = ui.field(stableID: "onecad.dimension.input", value: old)
        waitUntil(field, .exists, "dimension field showing \(old)")
        keyboard.replace(old, with: new)
        keyboard.done()
        ui.waitText("Distance (mm): \(new)")
        ui.waitText("Sketch · Fully constrained")
        ui.tap(UI.finish)
        ui.wait(UI.sketch)
    }

    /// Reads the width back in the sketch's constraint list, then leaves
    /// without changes.
    private func expectPlateWidth(_ width: String) {
        editFirstSketch()
        ui.waitText("Distance (mm): \(width)")
        ui.waitText("Distance (mm): 40")
        ui.tap(UI.discard)
        ui.wait(UI.sketch)
    }
}
