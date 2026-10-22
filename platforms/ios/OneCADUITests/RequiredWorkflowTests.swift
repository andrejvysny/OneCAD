import XCTest

/// Required part workflow through public XCUITest only (evidence class 4,
/// docs/ios/ARCHITECTURE.md): taps and coordinate taps on the canvas, the
/// system software keyboard for egui fields, `typeText` for UIKit fields.
/// No test hooks, no synthesized HID or touch events.
///
/// Canvas coordinates assume the iPad Pro 13-inch simulator in portrait with
/// the default camera of a new document (docs/ios/TESTING.md); the test fails
/// fast on any other window size.
@MainActor
final class RequiredWorkflowTests: XCTestCase {
    /// Sketch-plane millimetres to screen points under the default sketch
    /// camera, measured on iPad Pro 13-inch (M5) portrait: 60 mm = 77 pt.
    static let pointsPerMillimetre: CGFloat = 1.285
    /// Window of that measurement; the scale, the XY quad offset and the
    /// software keyboard's tab key all depend on it.
    static let measuredWindow = CGSize(width: 1032, height: 1376)

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
        requireMeasuredDevice()

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
            // The circle's disk (the outer region excludes it); its sketch
            // reports the face it was placed on.
            sketchPoint(30, 20).tap()
            ui.wait(UI.supportFace, .exists)
        }
        step("07-cut-through") {
            // The disk selected in step 06 is the profile.
            ui.tap(UI.extrude)
            let operation = ui.control(stableID: "onecad.param.operation", value: "New Body")
            waitUntil(operation, .hittable, "extrude Operation selector")
            operation.tap()
            ui.tap(UI.cutMode)
            waitUntil(ui.control(stableID: "onecad.param.operation", value: "Cut"), .value("Cut"),
                      "extrude Operation shows Cut")
            ui.tap(UI.bodyRow)
            setExtrudeDistance("5")
            ui.tap(UI.flip)
            ui.waitText("Reverse")
            ui.wait(UI.create, .enabled).tap()
            ui.expectHistory(["Sketch", "Extrude", "Sketch", "Extrude"])
            XCTAssertFalse(ui.text("Body 2").exists, "Cut adds no body")
            ui.tap(UI.propertiesTab)
            ui.tap(UI.bodyRow)
            ui.waitText("Health: Healthy")
            // The hole's rim is an edge of Body 1 on the top face: a Cut (an
            // Add would merge into the top face, a new body is ruled out
            // above). Fit keeps the face pick outside the 22 pt finger edge
            // window; the visible hole's rim is 3 mm from its centre.
            expectPlatePicks([(10, 10, .face), (33, 20, .edge)], "hole rim at (33, 20)")
        }
        step("08-edit-width-70") {
            editPlateWidth(from: "60", to: "70")
            ui.expectHistory(["Sketch", "Extrude", "Sketch", "Extrude"])
            ui.tap(UI.propertiesTab)
            ui.tap(UI.bodyRow)
            ui.waitText("Health: Healthy")
            // The regenerated plate reaches x = 70. Fitted picks clear the
            // 22 pt finger edge window and stay inside the visible canvas.
            expectPlatePicks([(-3, 20, .nothing), (62, 20, .face)], width: 70, "widened plate at (62, 20)")
            expectPlatePicks([(10, 10, .face), (33, 20, .edge)], width: 70, "hole rim kept at (33, 20)")
        }
        step("09-undo-redo") {
            // Undo restores the 60 mm width, a second undo removes the cut.
            ui.tap(UI.undo)
            expectPlateWidth("60")
            // At the fitted scale, x = 62 lies more than 22 pt outside the
            // restored 60 mm plate. Y = 28 avoids the floating tool pill.
            expectPlatePicks([(10, 10, .face), (62, 28, .nothing)], "plate back to 60 mm at (62, 28)")
            ui.tap(UI.undo)
            ui.expectHistory(["Sketch", "Extrude", "Sketch"])
            ui.tap(UI.redo)
            ui.expectHistory(["Sketch", "Extrude", "Sketch", "Extrude"])
            expectPlateWidth("60")
            expectThroughHole()
        }
        let modeling = WorkflowModelingChecks(ui: ui, keyboard: keyboard, test: self)
        step("10-fillet-1mm-preview-commit-undo") {
            modeling.edgeTreatment(UI.fillet, feature: "Fillet")
        }
        step("11-separate-chamfer-1mm-preview-commit-undo") {
            modeling.edgeTreatment(UI.chamfer, feature: "Chamfer")
        }
        step("12-offset-face-preview-cancel-commit-undo") {
            modeling.offsetFace()
        }
        step("13-move-rotate-preview-commit-undo") {
            modeling.moveRotate()
            expectThroughHole()
        }
        step("14-save") {
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
        step("15-relaunch-and-reopen") {
            app.terminate()
            app.launch()
            waitUntil(app.element(id: OneCADID.canvas), .exists, "canvas after relaunch")
            ui.waitStartPage()
            ui.tap(UIRef(label: name))
            ui.wait(UI.viewport, .exists)
            ui.expectHistory(["Sketch", "Extrude", "Sketch", "Extrude"])
            expectFeatureValue(row: 1, "Extrude · 4.0 mm")
            expectFeatureValue(row: 3, "Extrude · 5.0 mm")
            ui.waitText("Body 1")
            XCTAssertFalse(ui.text("Body 2").exists, "reopened cut adds no body")
            ui.tap(UI.propertiesTab)
            ui.tap(UI.bodyRow)
            ui.waitText("Health: Healthy")
            expectThroughHole()
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

    /// The gate's full run id (`TEST_RUNNER_ONECAD_RUN_ID`), so the saved
    /// document carries it; otherwise a short unique suffix.
    private func documentSuffix() -> String {
        let keep: (Character) -> Bool = { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") }
        if let run = ProcessInfo.processInfo.environment["ONECAD_RUN_ID"], !run.isEmpty {
            return String(run.filter(keep))
        }
        return String(UUID().uuidString.filter(keep).suffix(8))
    }

    /// Canvas millimetres, the XY quad offset and the keyboard's tab key were
    /// measured on one window size; elsewhere every coordinate tap would miss.
    private func requireMeasuredDevice() {
        let size = app.windows.firstMatch.frame.size
        if size != Self.measuredWindow {
            XCTFail("RequiredWorkflowTests needs the iPad Pro 13-inch simulator in portrait "
                + "(window \(Self.measuredWindow)); this window is \(size). Re-measure "
                + "pointsPerMillimetre and the XY quad offset first (docs/ios/TESTING.md).")
        }
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

    /// What a tap in the model view selects, read from the Properties title.
    enum Pick: String {
        case face = "Face"
        case edge = "Edge"
        case nothing = "Nothing selected"
    }

    /// Taps `taps` in order; each must select its pick (a face or edge of
    /// Body 1, or nothing). Consecutive picks differ, so every title change
    /// shows that tap was handled. A plain tap replaces the selection.
    /// Regeneration and camera animation have no completion signal in the
    /// UI, so a tap may meet the previous geometry or view: the sequence is
    /// retried from `prepare` and its first tap, each wait bounded, at most
    /// `attempts` times.
    private func expectPicks(_ taps: [(XCUICoordinate, Pick)], _ what: String, attempts: Int = 4,
                             prepare: () -> Void = {}, file: StaticString = #filePath, line: UInt = #line) {
        ui.tap(UI.propertiesTab, file: file, line: line)
        for attempt in 1...attempts {
            prepare()
            let met = taps.allSatisfy { point, pick in
                point.tap()
                guard ui.text(pick.rawValue).waitForExistence(timeout: 5) else { return false }
                return pick == .nothing || ui.text("on Body 1").exists
            }
            if met { return }
            if attempt == attempts {
                XCTFail("\(what): expected \(taps.map(\.1.rawValue))", file: file, line: line)
            }
        }
    }

    /// Coordinates remain sketch millimetres after editing or fitting; Home
    /// restores the camera basis before Top and Fit frame the current bounds.
    private func expectPlatePicks(_ picks: [(Double, Double, Pick)], width: Double = 60, _ what: String) {
        ui.tap(UI.homeView)
        ui.tap(UI.topView)
        ui.tap(UI.fitAll)
        let view = WorkflowFittedPlateView(ui: ui, direction: .top, width: width, height: 4)
        expectPicks(picks.map { (view.point($0.0, $0.1, 4), $0.2) }, what)
    }

    /// Top view, then Fit all, frames the plate's bounding box at the
    /// viewport centre, which is the hole's centre (30, 20) of the 60 x 40
    /// plate, with the hole about 90 pt across: wider than twice the 22 pt
    /// finger edge window. Nothing under the centre, looking straight down, means no
    /// floor: the hole goes through. A reopened document keeps its own
    /// (isometric) camera, hence the explicit top view.
    private func expectThroughHole(file: StaticString = #filePath, line: UInt = #line) {
        let viewport = ui.find(UI.viewport).frame
        let centre = CGPoint(x: viewport.midX, y: viewport.midY)
        expectPicks([(ui.point(centre.x - 100, centre.y), .face), (ui.point(centre.x, centre.y), .nothing)],
                    "through hole at the fitted plate's centre",
                    prepare: {
                        ui.tap(UI.homeView, file: file, line: line)
                        ui.tap(UI.topView, file: file, line: line)
                        ui.tap(UI.fitAll, file: file, line: line)
                    }, file: file, line: line)
    }

    /// Selects History row `row` (0-based) and reads its "<type> · <value>" line.
    private func expectFeatureValue(row: Int, _ text: String, file: StaticString = #filePath, line: UInt = #line) {
        ui.tap(UI.historyTab, file: file, line: line)
        let rows = ui.historyRowElements()
        guard rows.indices.contains(row) else {
            XCTFail("history row \(row) missing: \(rows.map(\.label))", file: file, line: line)
            return
        }
        rows[row].tap()
        ui.waitText(text, file: file, line: line)
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
