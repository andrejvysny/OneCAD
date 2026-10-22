import XCTest

/// One UI control as the suite addresses it: the stable automation id the
/// editor is moving to (`onecad.tool.<id>`, `onecad.param.<name>`, ...), then
/// today's derived `<role>.<label>` id, then the visible label. The first that
/// exists wins, so the suite survives the switch to stable ids unchanged.
struct UIRef: CustomStringConvertible {
    let ids: [String]
    let label: String?
    /// For controls whose label carries live state (the viewport's hint).
    let labelPrefix: String?

    init(_ ids: String..., label: String? = nil, labelPrefix: String? = nil) {
        self.ids = ids
        self.label = label
        self.labelPrefix = labelPrefix
    }

    var description: String {
        (ids + [label.map { "label '\($0)'" }, labelPrefix.map { "label '\($0)…'" }].compactMap { $0 })
            .joined(separator: " | ")
    }

    /// Matches any of the names; used only to wait for presence.
    var anyName: NSPredicate {
        var parts = ids.map { NSPredicate(format: "identifier == %@", $0) }
        if let label { parts.append(NSPredicate(format: "label == %@", label)) }
        if let labelPrefix { parts.append(NSPredicate(format: "label BEGINSWITH %@", labelPrefix)) }
        return NSCompoundPredicate(orPredicateWithSubpredicates: parts)
    }

    /// Candidate queries in priority order: ids first, the label last, so a
    /// label shared by several controls (toolbar "Sketch" and the explorer's
    /// "Sketch" row) never shadows an id match.
    @MainActor
    func candidates(in app: XCUIApplication) -> [XCUIElement] {
        let all = app.descendants(matching: .any)
        var queries = ids.map { all.matching(identifier: $0) }
        if let label { queries.append(all.matching(NSPredicate(format: "label == %@", label))) }
        if let labelPrefix { queries.append(all.matching(NSPredicate(format: "label BEGINSWITH %@", labelPrefix))) }
        return queries.map { $0.firstMatch }
    }
}

/// Catalogue of the controls the required workflow touches
/// (docs/ios/TESTING.md, "Required workflow").
enum UI {
    static func tool(_ id: String, auto: String, label: String) -> UIRef {
        UIRef("onecad.tool.\(id)", "button.\(auto)", "menu_item.\(auto)", label: label)
    }

    static let newPart = UIRef("onecad.start.newPart", "button.new-part", label: "New part")
    static let discardRecovery = UIRef("onecad.recovery.discard", "button.discard-recovery", label: "Discard recovery")
    static let viewport = UIRef("onecad.viewport", labelPrefix: "Model viewport")
    static let sketch = tool("sketch", auto: "sketch", label: "Sketch")
    static let moreTools = UIRef("onecad.tool.more", "button.more-tools", label: "More tools")
    static let rectangle = tool("rectangle", auto: "rectangle", label: "Rectangle")
    static let circle = tool("circle", auto: "circle", label: "Circle")
    static let extrude = tool("extrude", auto: "extrude", label: "Extrude")
    static let numericInput = UIRef("onecad.sketch.numeric", "button.numeric-input-tab", label: "Numeric input · Tab")
    static let finish = UIRef("onecad.sketch.finish", "button.finish", label: "Finish")
    static let planeChooser = UIRef("onecad.plane.chooser", "static_text.choose-xy-xz-yz-or-a-planar-face",
                                    label: "Choose XY, XZ, YZ or a planar face")
    static let width = UIRef("onecad.param.width", "text_field.width-mm", label: "Width mm")
    static let height = UIRef("onecad.param.height", "text_field.height-mm", label: "Height mm")
    static let diameter = UIRef("onecad.param.diameter", "text_field.ø-mm", "text_field.mm", label: "Ø mm")
    static let extrudeDistance = UIRef("onecad.param.distance", "text_field.extrude-distance-mm",
                                       label: "Extrude distance (mm)")
    static let create = UIRef("onecad.op.confirm", "button.create", label: "Create ↵")
    static let point = tool("point", auto: "point", label: "Point")
    static let x = UIRef("onecad.param.x", "text_field.x", label: "X")
    static let y = UIRef("onecad.param.y", "text_field.y", label: "Y")
    static let cutMode = UIRef("onecad.param.operation.cut", "button.cut", label: "Cut")
    static let flip = UIRef("onecad.param.flip", "button.flip", label: "Flip")
    static let bodyRow = UIRef("onecad.explorer.body.0", "other.body-1", label: "Body 1")
    static let editFeature = UIRef("onecad.history.edit", "button.edit-feature", label: "Edit feature")
    static let discard = UIRef("onecad.sketch.discard", "button.discard", label: "Discard")
    static let undo = UIRef("onecad.titlebar.undo", "button.undo", label: "Undo")
    static let redo = UIRef("onecad.titlebar.redo", "button.redo", label: "Redo")
    static let fileMenu = UIRef(OneCADID.fileMenu)
    static let save = UIRef("onecad.menu.file.save", label: "Save")
    static let historyTab = UIRef("onecad.inspector.history", "checkbox.history", label: "History")
    static let propertiesTab = UIRef("onecad.inspector.properties", "checkbox.properties", label: "Properties")
}

@MainActor
struct OneCADScreen {
    let app: XCUIApplication
    let test: XCTestCase

    /// The first candidate of `ref` that exists now, else its first candidate.
    func find(_ ref: UIRef) -> XCUIElement {
        let candidates = ref.candidates(in: app)
        return candidates.first(where: { $0.exists }) ?? candidates[0]
    }

    func text(_ label: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", label)).firstMatch
    }

    /// Waits for a visible text (inspector rows, dimension lists, statuses).
    @discardableResult
    func waitText(_ label: String, _ condition: ElementCondition = .exists, timeout: TimeInterval = 30,
                  file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let element = text(label)
        test.waitUntil(element, condition, timeout: timeout, "text '\(label)'", file: file, line: line)
        return element
    }

    /// A control known by its current value only (egui combo boxes are
    /// unnamed today: the Operation selector shows "New Body").
    func control(stableID: String, value: String) -> XCUIElement {
        let stable = app.descendants(matching: .any).matching(identifier: stableID).firstMatch
        if stable.exists { return stable }
        return app.descendants(matching: .any).matching(NSPredicate(format: "value == %@", value)).firstMatch
    }

    /// The `index`-th (0-based) element labelled `label`, for counting rows.
    func nth(_ label: String, _ index: Int) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", label)).element(boundBy: index)
    }

    /// Feature rows of the inspector's History tab, top to bottom (feature
    /// order): row elements below the inspector's tab control, within its
    /// column. The explorer is no substitute: it lists sketches by id.
    func historyRowElements() -> [XCUIElement] {
        let tab = find(UI.propertiesTab).frame
        return app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'other.' OR identifier BEGINSWITH 'onecad.history.'"))
            .allElementsBoundByIndex
            .filter { $0.frame.minX >= tab.minX - 24 && $0.frame.minY > tab.maxY && !$0.label.isEmpty }
            .sorted { $0.frame.minY < $1.frame.minY }
    }

    func historyRows() -> [String] {
        historyRowElements().map(\.label)
    }

    /// Shows the History tab and waits until it lists exactly `expected`.
    func expectHistory(_ expected: [String], file: StaticString = #filePath, line: UInt = #line) {
        tap(UI.historyTab, file: file, line: line)
        let extrudes = expected.filter { $0 == "Extrude" }.count
        if extrudes > 0 {
            test.waitUntil(nth("Extrude", extrudes - 1), .exists, "history row Extrude #\(extrudes)",
                           file: file, line: line)
        }
        test.waitUntil(nth("Extrude", extrudes), .gone, "no history row Extrude #\(extrudes + 1)",
                       file: file, line: line)
        XCTAssertEqual(historyRows(), expected, "history rows", file: file, line: line)
    }

    /// The Edit button on the same constraint-list row as `rowText`.
    func editButton(onRow rowText: XCUIElement) -> XCUIElement {
        let row = rowText.frame
        let buttons = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label == 'Edit'")).allElementsBoundByIndex
        return buttons.first { abs($0.frame.midY - row.midY) < row.height && $0.frame.minX > row.minX }
            ?? app.descendants(matching: .any).matching(identifier: "onecad.dimension.edit").firstMatch
    }

    /// The text field currently showing `value` (the dimension editor's
    /// field is unnamed today; the stable id wins once it exists).
    func field(stableID: String, value: String) -> XCUIElement {
        let stable = app.descendants(matching: .any).matching(identifier: stableID).firstMatch
        if stable.exists { return stable }
        return app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'text_field.' AND value == %@", value)).firstMatch
    }

    @discardableResult
    func wait(_ ref: UIRef, _ condition: ElementCondition = .hittable, timeout: TimeInterval = 30,
              file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        if case .gone = condition {
            test.waitUntil(app.descendants(matching: .any).matching(ref.anyName).firstMatch, .gone,
                           timeout: timeout, "\(ref)", file: file, line: line)
            return find(ref)
        }
        // Wait until any of the names is present, then hold the
        // highest-priority one to the condition.
        test.waitUntil(app.descendants(matching: .any).matching(ref.anyName).firstMatch, .exists,
                       timeout: timeout, "\(ref)", file: file, line: line)
        let element = find(ref)
        test.waitUntil(element, condition, timeout: timeout, "\(ref)", file: file, line: line)
        return element
    }

    func tap(_ ref: UIRef, file: StaticString = #filePath, line: UInt = #line) {
        wait(ref, .hittable, file: file, line: line).tap()
    }

    /// The start page, after discarding autosave recovery copies a previous
    /// (failed or killed) run left behind; the workflow must start from a
    /// new document, never from recovered state.
    func waitStartPage(file: StaticString = #filePath, line: UInt = #line) {
        let either = NSCompoundPredicate(orPredicateWithSubpredicates: [UI.newPart.anyName, UI.discardRecovery.anyName])
        for _ in 0..<8 {
            test.waitUntil(app.descendants(matching: .any).matching(either).firstMatch, .exists,
                           "start page or recovery prompt", file: file, line: line)
            let discard = find(UI.discardRecovery)
            guard discard.exists else { break }
            discard.tap()
            test.waitUntil(discard, .gone, "recovery prompt dismissed", file: file, line: line)
        }
        wait(UI.newPart, file: file, line: line)
    }

    /// A point on the screen in points (the canvas fills the window).
    func point(_ x: CGFloat, _ y: CGFloat) -> XCUICoordinate {
        app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: x, dy: y))
    }

    func screenshot(_ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        test.add(shot)
    }
}

/// The system software keyboard, driven key by key through public XCUITest
/// taps (the editor's fields live in Rust, so `typeText` has no UIKit
/// element with keyboard focus to target). Every key reaches the app through
/// UIKit's text system exactly as a user's tap does.
@MainActor
struct SoftwareKeyboard {
    let app: XCUIApplication
    let test: XCTestCase

    private var keyboard: XCUIElement { app.keyboards.firstMatch }

    private func key(_ name: String) -> XCUIElement {
        keyboard.descendants(matching: .any)
            .matching(NSPredicate(format: "label == %@ OR identifier == %@", name, name)).firstMatch
    }

    func waitShown(file: StaticString = #filePath, line: UInt = #line) {
        test.waitUntil(keyboard, .hittable, timeout: 15,
                       "software keyboard (disable Connect Hardware Keyboard on the simulator)",
                       file: file, line: line)
    }

    func press(_ name: String, file: StaticString = #filePath, line: UInt = #line) {
        let element = key(name)
        test.waitUntil(element, .hittable, timeout: 10, "keyboard key '\(name)'", file: file, line: line)
        element.tap()
    }

    func type(_ text: String, file: StaticString = #filePath, line: UInt = #line) {
        for character in text { press(String(character), file: file, line: line) }
    }

    /// Replaces the focused field's text: one delete per character of `current`.
    func replace(_ current: String, with text: String, file: StaticString = #filePath, line: UInt = #line) {
        for _ in current { press("delete", file: file, line: line) }
        type(text, file: file, line: line)
    }

    func tab(file: StaticString = #filePath, line: UInt = #line) { press("tab", file: file, line: line) }
    func done(file: StaticString = #filePath, line: UInt = #line) { press("Done", file: file, line: line) }
}
