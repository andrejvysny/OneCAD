import XCTest

/// Public finger controls, keyboard, Files, history and persistence (evidence class 4).
/// The gate seeds its current Mac-authored fixture in the Files fixture source.
/// This suite opens it through Files; no app test hooks or Pencil synthesis.
/// Pencil authoring has separate synthetic-native and physical evidence.
@MainActor
final class RequiredWorkflowTests: XCTestCase {
    private var app: XCUIApplication!
    private var ui: OneCADScreen!
    private var keyboard: SoftwareKeyboard!

    override func setUp() async throws {
        continueAfterFailure = false
    }

    /// Failures carry the screen and the accessibility tree they saw.
    override func record(_ issue: XCTIssue) {
        var issue = issue
        if Thread.isMainThread, let (png, tree) = MainActor.assumeIsolated({ () -> (Data, String)? in
            let application = XCUIApplication()
            guard application.state == .runningForeground else { return nil }
            return (XCUIScreen.main.screenshot().pngRepresentation, application.debugDescription)
        }) {
            issue.add(XCTAttachment(data: png, uniformTypeIdentifier: "public.png"))
            issue.add(XCTAttachment(string: tree))
        }
        super.record(issue)
    }

    func testPencilOnlyCanvasWithNumericControlsHistoryUndoSaveReopen() {
        launchWorkflow()
        step("01-fingers-do-not-author-canvas") { expectFingerAuthoringRefused() }
        step("02-numeric-point-controls-undo-redo") { exerciseNumericPoint() }
        ui.tap(UI.discard)
        closeTabDiscarding(app)
        step("03-public-open-mac-authored-plate") { openSeededPlate() }
        step("04-history-numeric-width-undo-redo") {
            editPlateWidth(from: "60", to: "70")
            expectHealthyPlate()
            ui.tap(UI.undo)
            expectPlateWidth("60")
            ui.tap(UI.redo)
            expectPlateWidth("70")
            ui.tap(UI.undo)
            expectPlateWidth("60")
            expectHealthyPlate()
        }
        let name = "Plate-" + documentSuffix()
        step("05-save-as") { saveAs(name) }
        step("06-relaunch-and-reopen") {
            app.terminate()
            app.launch()
            waitUntil(app.element(id: OneCADID.canvas), .exists, "canvas after relaunch")
            ui.waitStartPage()
            ui.tap(UIRef(label: name))
            ui.wait(UI.viewport, .exists)
            expectHealthyPlate()
            expectPlateWidth("60")
        }
        closeTabDiscarding(app)
    }

    func testOpenedPlateSaveAsAndStepExportThroughNativePrompts() {
        launchWorkflow()
        openSeededPlate()
        let name = "PromptPlate-" + documentSuffix()
        saveAs(name)
        exerciseNamedPrompts(name)
        app.terminate()
        app.launch()
        waitUntil(app.element(id: OneCADID.canvas), .exists, "canvas after relaunch")
        let recent = app.element(label: name)
        waitUntil(recent, .hittable, "named Save As document in recents")
        recent.tap()
        ui.wait(UI.viewport, .exists)
        expectHealthyPlate()
        closeTabDiscarding(app)
    }

    private func launchWorkflow() {
        app = launchOneCAD()
        ui = OneCADScreen(app: app, test: self)
        keyboard = SoftwareKeyboard(app: app, test: self)
        discardRecoveryOffer(app)
    }

    private func expectFingerAuthoringRefused() {
        tapFileMenu(app, OneCADID.fileNew)
        ui.tap(UI.sketch)
        ui.tap(UIRef(OneCADID.planeXY))
        ui.wait(UI.finish)
        ui.tap(UI.propertiesTab)
        ui.setExplorer(open: false)
        ui.tap(UI.moreTools)
        ui.tap(UI.rectangle)
        let view = WorkflowSketchView(ui: ui)
        view.point(-80, 0).tap()
        view.point(80, 40).tap()
        view.point(-80, 0).press(forDuration: 0.05, thenDragTo: view.point(80, 40))
        ui.tap(UI.point)
        ui.waitText("Entities: 0")
        ui.waitText("Constraints: 0")
        ui.waitText("Sketch · Fully constrained")
    }

    private func exerciseNumericPoint() {
        ui.tap(UI.point)
        enterNumeric([(UI.x, "30"), (UI.y, "20")])
        expectNumericPoint()
        ui.tap(UI.undo)
        ui.waitText("Entities: 0")
        ui.waitText("Constraints: 0")
        ui.tap(UI.redo)
        expectNumericPoint()
    }

    private func expectNumericPoint() {
        // Numeric coordinates add a fixed construction origin and the point.
        ui.waitText("Entities: 2")
        ui.waitText("Constraints: 3")
        ui.waitText("Sketch · Fully constrained")
        ui.waitText("Fixed")
        ui.waitText("Horizontal distance (mm): 30")
        ui.waitText("Vertical distance (mm): 20")
    }

    private func openSeededPlate() {
        guard let filename = ProcessInfo.processInfo.environment["ONECAD_PUBLIC_FIXTURE_NAME"], !filename.isEmpty else {
            XCTFail("TEST_RUNNER_ONECAD_PUBLIC_FIXTURE_NAME must name the current gate's seeded Mac fixture")
            return
        }
        tapFileMenu(app, "onecad.menu.file.open")
        waitUntil(app.navigationBars.buttons["Cancel"], .hittable, "public Open picker")
        let browse = app.buttons["Browse"]
        if browse.exists && browse.isHittable { browse.tap() }
        let local = app.element(label: "On My iPad")
        waitUntil(local, .hittable, "local Files location")
        local.tap()
        let folder = app.cells.matching(NSPredicate(format: "label == %@ OR label BEGINSWITH %@",
                                                   "OneCAD Fixtures", "OneCAD Fixtures,")).firstMatch
        waitUntil(folder, .hittable, "OneCAD Fixtures Documents folder")
        folder.tap()
        let stem = (filename as NSString).deletingPathExtension
        let file = app.cells.matching(NSPredicate(
            format: "label == %@ OR label BEGINSWITH %@ OR label == %@ OR label BEGINSWITH %@",
            stem, stem + ",", filename, filename + ",")).firstMatch
        waitUntil(file, .hittable, "seeded Mac-authored fixture \(filename)")
        file.tap()
        waitUntil(app.navigationBars.buttons["Cancel"], .gone, "Open picker completed")
        ui.wait(UI.viewport, .exists)
        expectHealthyPlate()
    }

    private func expectHealthyPlate() {
        ui.expectHistory(["Sketch", "Plate"])
        ui.tap(UIRef("onecad.history.1", label: "Plate"))
        ui.waitText("Extrude · 4.0 mm")
        ui.tap(UI.propertiesTab)
        ui.selectBody()
        ui.waitText("Health: Healthy")
        XCTAssertFalse(ui.text("Body 2").exists, "the fixture retains exactly one body")
    }

    private func saveAs(_ name: String) {
        tapFileMenu(app, "onecad.menu.file.saveAs")
        let field = app.textFields[OneCADID.saveName]
        waitUntil(field, .hittable, "Save As name prompt")
        replacePromptName(field, with: name)
        app.alerts.buttons["Save"].tap()
        waitUntil(field, .gone, "Save As prompt dismissed")
        let tab = app.element(id: OneCADID.tab(0))
        let renamed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == 1 AND label == %@", name),
                                                object: tab)
        XCTAssertEqual(XCTWaiter().wait(for: [renamed], timeout: 30), .completed,
                       "saved tab must have exact name '\(name)', actual '\(tab.label)'")
    }

    private func exerciseNamedPrompts(_ name: String) {
        let field = app.textFields[OneCADID.saveName]
        for decision in ["Cancel", "Replace"] {
            tapFileMenu(app, "onecad.menu.file.saveAs")
            waitUntil(field, .hittable, "named Save As prompt")
            XCTAssertEqual(field.value as? String, name, "named document supplies its own default")
            replacePromptName(field, with: name + ".OneCaD.onecad")
            app.alerts.buttons["Save"].tap()
            let replace = app.alerts.buttons["Replace"]
            waitUntil(replace, .hittable, "one suffix resolves to the existing document")
            app.alerts.buttons[decision].tap()
            waitUntil(replace, .gone, "overwrite decision completed")
            XCTAssertEqual(app.element(id: OneCADID.tab(0)).label, name)
        }
        for cancel in [true, false] {
            tapFileMenu(app, "onecad.menu.file.export")
            waitUntil(field, .hittable, "native STEP name prompt")
            XCTAssertEqual(field.value as? String, name, "export uses the named plate default")
            if cancel {
                app.alerts.buttons["Cancel"].tap()
                waitUntil(field, .gone, "cancelled STEP prompt")
                XCTAssertEqual(app.element(id: OneCADID.tab(0)).label, name)
                continue
            }
            replacePromptName(field, with: name + ".STEP.step")
            app.alerts.buttons["Save"].tap()
            let cancelPicker = app.navigationBars.buttons["Cancel"]
            waitUntil(cancelPicker, .hittable, timeout: 60, "valid STEP reaches system export picker")
            ui.screenshot("native-prompt-step-picker")
            cancelPicker.tap()
            waitUntil(cancelPicker, .gone, "system export picker dismissed")
            let activity = app.element(label: "Activity · Completed")
            waitUntil(activity, .hittable, "STEP export completed")
            activity.tap()
            openExportDetails()
            let exported = app.descendants(matching: .any).matching(NSPredicate(
                format: "label BEGINSWITH %@ AND label ENDSWITH %@", "Exported ", "/" + name + ".step"))
                .firstMatch
            waitUntil(exported, .exists, "STEP export reports one normalized suffix")
            app.element(id: OneCADID.canvas).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6)).tap()
        }
    }

    private func openExportDetails() {
        let header = ui.waitText("Export · Completed")
        let summaries = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Exported "))
            .allElementsBoundByIndex.filter { $0.frame.minY >= header.frame.maxY }
        guard let summary = summaries.min(by: { $0.frame.minY < $1.frame.minY }) else {
            return XCTFail("completed Export entry must expose its summary")
        }
        let nextHeader = app.staticTexts.allElementsBoundByIndex.filter {
            $0.label.contains(" · ") && $0.frame.minY > summary.frame.maxY
        }.map(\.frame.minY).min() ?? app.frame.maxY
        let details = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Details"))
            .allElementsBoundByIndex.filter {
                $0.frame.minY >= summary.frame.maxY && $0.frame.maxY <= nextHeader
            }
        XCTAssertEqual(details.count, 1, "exactly one Details disclosure belongs to the latest Export entry")
        guard let detail = details.first, details.count == 1 else { return }
        waitUntil(detail, .hittable, "completed Export Details disclosure")
        detail.tap()
    }

    private func replacePromptName(_ field: XCUIElement, with name: String) {
        field.tap()
        if !(field.value as? String ?? "").isEmpty {
            let clear = field.buttons["Clear text"]
            waitUntil(clear, .hittable, "native prompt Clear text button")
            clear.tap()
        }
        field.typeText(name)
        waitUntil(field, .value(name), "complete native prompt name before Save")
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

    /// Each field is tapped explicitly because smaller iPad keyboards lack Tab.
    private func enterNumeric(_ fields: [(UIRef, String)], open: Bool = true) {
        if open { ui.tap(UI.numericInput) }
        keyboard.waitShown()
        for (ref, text) in fields {
            ui.focus(ref)
            let focused = keyboard.replaceFocused(with: text)
            waitUntil(focused, .value(text), "\(ref) shows \(text)")
        }
        keyboard.done()
    }

    /// Opens the plate sketch from its History row (feature 1).
    private func editFirstSketch() {
        ui.expectHistory(["Sketch", "Plate"])
        let rows = ui.historyRowElements()
        XCTAssertEqual(rows.first?.label, "Sketch", "first history row")
        ui.tap(UIRef("onecad.history.0", label: "Sketch"))
        ui.tap(UI.editFeature)
        ui.wait(UI.finish)
        ui.tap(UI.propertiesTab)
    }

    /// Re-enters the first sketch and drives its width dimension through the
    /// constraint list's Edit field.
    private func editPlateWidth(from old: String, to new: String) {
        editFirstSketch()
        let row = constraintRow("Horizontal distance (mm): \(old)")
        let edit = ui.editButton(onRow: row)
        waitUntil(edit, .hittable, "Edit beside the \(old) mm width")
        edit.tap()
        keyboard.waitShown()
        let field = ui.field(stableID: "onecad.dimension.input", value: old)
        waitUntil(field, .exists, "dimension field showing \(old)")
        keyboard.replaceFocused(with: new)
        keyboard.done()
        _ = constraintRow("Horizontal distance (mm): \(new)")
        ui.waitText("iPad acceptance plate · Fully constrained")
        ui.tap(UI.finish)
        ui.wait(UI.sketch)
    }

    /// Reads the width back in the sketch's constraint list, then leaves
    /// without changes.
    private func expectPlateWidth(_ width: String) {
        editFirstSketch()
        _ = constraintRow("Horizontal distance (mm): \(width)")
        _ = constraintRow("Vertical distance (mm): 40")
        _ = constraintRow("Diameter (mm): 6")
        ui.waitText("Entities: 14")
        ui.waitText("Constraints: 13")
        ui.waitText("iPad acceptance plate · Fully constrained")
        ui.tap(UI.discard)
        ui.wait(UI.sketch)
    }

    private func constraintRow(_ label: String) -> XCUIElement {
        let row = ui.text(label)
        let heading = ui.waitText("Sketch constraints")
        let show = ui.wait(UIRef("button.show-dimension-labels", label: "Show dimension labels"), .exists)
        let viewport = ui.wait(UI.viewport, .exists).frame
        // The native canvas exports individual controls rather than a UIKit
        // scroll view. Use its observed header; 220 pt is the production cap
        // in constraint_authoring.rs, further bounded by the actual viewport.
        let top = show.frame.maxY + 4
        let panel = ui.waitText("Constraints: 13").frame
        let fallback = CGRect(x: heading.frame.minX, y: top, width: panel.maxX - heading.frame.minX,
                              height: min(220, viewport.maxY - top - 8))
        let exported = app.scrollViews.allElementsBoundByIndex.first {
            $0.frame.contains(CGPoint(x: fallback.midX, y: fallback.minY + 8)) && $0.frame.height <= 221
        }
        let frame = exported?.frame.intersection(fallback) ?? fallback
        XCTAssertGreaterThan(frame.height, 40, "constraint scroll area must be usable")
        for attempt in 0...8 {
            if row.exists, row.isHittable, row.frame.minY >= frame.minY, row.frame.maxY <= frame.maxY {
                let edit = ui.editButton(onRow: row)
                if edit.exists, edit.isHittable, edit.frame.minY >= frame.minY, edit.frame.maxY <= frame.maxY {
                    return row
                }
            }
            guard attempt < 8 else { break }
            // Existing clipped nodes locate the direction. Otherwise search
            // toward the end, then reverse, since scroll state can persist.
            let upward = row.exists ? row.frame.midY > frame.midY : attempt < 4
            if !scrollConstraintPanel(row, frame: frame, upward: upward, capture: attempt == 0) { break }
        }
        XCTFail("constraint '\(label)' not visible and hittable after 8 public panel scrolls")
        return row
    }

    private func scrollConstraintPanel(_ row: XCUIElement, frame: CGRect, upward: Bool, capture: Bool) -> Bool {
        let from = upward ? frame.maxY - 12 : frame.minY + 12
        let to = upward ? frame.minY + 12 : frame.maxY - 12
        let previous = row.exists ? row.frame : nil
        let controls = app.buttons.matching(NSPredicate(format: "label IN %@", ["Edit", "Remove"]))
            .allElementsBoundByIndex.map(\.frame).filter {
                $0.minY >= frame.minY && $0.minX >= frame.minX && $0.maxX <= frame.maxX
            }
        guard let contentRight = controls.map(\.maxX).max() else {
            XCTFail("constraint scroll content must expose its Edit/Remove bounds")
            return false
        }
        // Vertical ScrollArea shrinks horizontally to its content. Panel
        // padding can lie outside it; a button can capture an inside drag.
        let candidates = [min(contentRight - 12, frame.maxX - 12), frame.minX + 12,
                          (frame.minX + contentRight) / 2]
        guard let x = candidates.first(where: { x in
            x >= frame.minX + 8 && x <= frame.maxX - 8
                && !controls.contains { $0.insetBy(dx: -2, dy: -2).contains(CGPoint(x: x, y: from)) }
        }) else {
            XCTFail("constraint scroll needs a start point clear of buttons")
            return false
        }
        ui.point(x, from).press(forDuration: 0.05, thenDragTo: ui.point(x, to))
        guard let previous else { return true }
        let moved = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            !row.exists || row.frame != previous
        }, object: nil)
        let result = XCTWaiter().wait(for: [moved], timeout: 3)
        XCTAssertEqual(result, .completed, "constraint panel must visibly scroll, was \(previous)")
        if result != .completed { ui.screenshot("constraint-panel-did-not-scroll") }
        else if capture { ui.screenshot("constraint-panel-scrolled") }
        return result == .completed
    }
}
