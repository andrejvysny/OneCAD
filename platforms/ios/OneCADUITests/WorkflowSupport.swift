import XCTest
import simd

/// One UI control as the suite addresses it: the stable automation id where
/// the editor defines one (`crates/onecad-ui/src/automation.rs` on the
/// stable-id lane: `onecad.tool.<id>`, `onecad.dimension.input`,
/// `onecad.param.<name>`, `onecad.op.confirm`, ...), then today's derived
/// `<role>.<label>` id, then the visible label. The first that exists wins.
/// Controls without a stable id list only the derived id and label.
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

    static let newPart = UIRef("button.new-part", label: "New part")
    static let discardRecovery = UIRef("button.discard-recovery", label: "Discard recovery")
    static let viewport = UIRef("onecad.viewport", labelPrefix: "Model viewport")
    static let sketch = tool("sketch", auto: "sketch", label: "Sketch")
    static let moreTools = tool("more", auto: "more-tools", label: "More tools")
    static let rectangle = tool("rectangle", auto: "rectangle", label: "Rectangle")
    static let circle = tool("circle", auto: "circle", label: "Circle")
    static let extrude = tool("extrude", auto: "extrude", label: "Extrude")
    static let fillet = tool("fillet", auto: "fillet", label: "Fillet")
    static let chamfer = tool("chamfer", auto: "chamfer", label: "Chamfer")
    static let offsetFace = tool("offsetFace", auto: "offset-face", label: "Offset face")
    static let moveRotate = tool("moveRotate", auto: "move-rotate", label: "Move / Rotate")
    static let numericInput = UIRef("button.numeric-input-tab", label: "Numeric input · Tab")
    static let finish = UIRef("onecad.tool.finishSketch", "button.finish", label: "Finish")
    static let discard = UIRef("onecad.tool.discardSketch", "button.discard", label: "Discard")
    static let planeChooser = UIRef("static_text.choose-xy-xz-yz-or-a-planar-face",
                                    label: "Choose XY, XZ, YZ or a planar face")
    // A creation chip's or operation card's first field is the dimension
    // input; the others are `onecad.param.<field>`.
    static let width = UIRef("onecad.dimension.input", "text_field.width-mm", label: "Width mm")
    static let height = UIRef("onecad.param.height", "text_field.height-mm", label: "Height mm")
    static let diameter = UIRef("onecad.dimension.input", "text_field.ø-mm", "text_field.mm", label: "Ø mm")
    static let x = UIRef("onecad.dimension.input", "text_field.x", label: "X")
    static let y = UIRef("onecad.param.y", "text_field.y", label: "Y")
    static let extrudeDistance = UIRef("onecad.dimension.input", "onecad.param.extrudeDistance",
                                       "text_field.extrude-distance-mm", label: "Extrude distance (mm)")
    static let create = UIRef("onecad.op.confirm", "button.create", label: "Create ↵")
    static let cancelOperation = UIRef("onecad.op.cancel", "button.cancel", label: "Cancel")
    static let operationValue = UIRef("onecad.dimension.input")
    static let translationY = UIRef("onecad.param.translationY", label: "Translation Y (mm)")
    static let translationZ = UIRef("onecad.param.translationZ", label: "Translation Z (mm)")
    static let rotationAngle = UIRef("onecad.param.rotationAngle", label: "Rotation angle (deg)")
    static let point = tool("point", auto: "point", label: "Point")
    static let cutMode = UIRef("button.cut", label: "Cut")
    static let flip = UIRef("button.flip", label: "Flip")
    static let bodyRow = UIRef("other.body-1", label: "Body 1")
    static let editFeature = UIRef("button.edit-feature", label: "Edit feature")
    static let undo = UIRef("button.undo", label: "Undo")
    static let redo = UIRef("button.redo", label: "Redo")
    static let fileMenu = UIRef(OneCADID.fileMenu)
    static let save = UIRef("onecad.menu.file.save", label: "Save")
    static let historyTab = UIRef("checkbox.history", label: "History")
    static let propertiesTab = UIRef("checkbox.properties", label: "Properties")
    static let supportFace = UIRef(labelPrefix: "Support face: ")
    static let fitAll = UIRef("button.fit-all", label: "Fit all")
    static let topView = UIRef("button.top-view", label: "TOP view")
    static let homeView = UIRef("button.home-view", label: "Home view")
    static let rightView = UIRef("button.right-view", label: "RIGHT view")
}

@MainActor
struct OneCADScreen {
    let app: XCUIApplication
    let test: XCTestCase

    /// The first candidate of `ref` that exists now, else its first candidate.
    func find(_ ref: UIRef) -> XCUIElement {
        let candidates = ref.candidates(in: app)
        if let existing = candidates.first(where: { $0.exists }) { return existing }
        // The focused virtual field is replaced by the native editable proxy.
        let focused = app.element(id: OneCADID.keyInputProxy)
        if ref.ids.contains("onecad.dimension.input"), focused.exists,
           ref.label == nil || focused.label == ref.label {
            return focused
        }
        return candidates[0]
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

    /// Inspector History rows: derived `other.` ids today, `onecad.history.<n>`
    /// on the stable-id lane. Excludes toolbar buttons with the same label.
    static let historyRow = NSPredicate(format: "identifier BEGINSWITH 'other.' OR identifier BEGINSWITH 'onecad.history.'")

    /// The `index`-th (0-based) row-like element labelled `label`, for counting rows.
    func nth(_ label: String, _ index: Int) -> XCUIElement {
        let named = NSPredicate(format: "label == %@", label)
        return app.descendants(matching: .any)
            .matching(NSCompoundPredicate(andPredicateWithSubpredicates: [named, Self.historyRow]))
            .element(boundBy: index)
    }

    /// Feature rows of the inspector's History tab, top to bottom (feature
    /// order): row elements below the inspector's tab control, within its
    /// column. The explorer is no substitute: it lists sketches by id.
    func historyRowElements() -> [XCUIElement] {
        let tab = find(UI.propertiesTab).frame
        return app.descendants(matching: .any)
            .matching(Self.historyRow)
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
        let counts = Dictionary(grouping: expected, by: { $0 }).mapValues(\.count)
        for (label, count) in counts {
            test.waitUntil(nth(label, count - 1), .exists, "history row \(label) #\(count)",
                           file: file, line: line)
            test.waitUntil(nth(label, count), .gone, "no history row \(label) #\(count + 1)",
                           file: file, line: line)
        }
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
        let focused = app.element(id: OneCADID.keyInputProxy)
        if focused.exists, focused.value as? String == value { return focused }
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
        // Resolve the focused native proxy as well as the virtual controls.
        let present = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            MainActor.assumeIsolated { self.find(ref).exists }
        }, object: nil)
        XCTAssertEqual(XCTWaiter().wait(for: [present], timeout: timeout), .completed,
                       "\(ref) did not appear", file: file, line: line)
        let element = find(ref)
        test.waitUntil(element, condition, timeout: timeout, "\(ref)", file: file, line: line)
        return element
    }

    func tap(_ ref: UIRef, file: StaticString = #filePath, line: UInt = #line) {
        wait(ref, .hittable, file: file, line: line).tap()
    }

    /// A focused virtual field has already been replaced by the native proxy.
    /// It receives keyboard input without a second tap on its transparent view.
    func focus(_ ref: UIRef, file: StaticString = #filePath, line: UInt = #line) {
        let present = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            MainActor.assumeIsolated { self.editableField(ref).exists }
        }, object: nil)
        XCTAssertEqual(XCTWaiter().wait(for: [present], timeout: 30), .completed,
                       "editable field \(ref) did not appear", file: file, line: line)
        let field = editableField(ref)
        if field.identifier != OneCADID.keyInputProxy {
            test.waitUntil(field, .hittable, "field before focus", file: file, line: line)
            field.tap()
        }
        let focused = app.element(id: OneCADID.keyInputProxy)
        test.waitUntil(focused, .exists, "native text input after focus", file: file, line: line)
        if let label = ref.label {
            let matched = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", label), object: focused)
            XCTAssertEqual(XCTWaiter().wait(for: [matched], timeout: 10), .completed,
                           "native text input focuses \(label)", file: file, line: line)
        }
    }

    private func editableField(_ ref: UIRef) -> XCUIElement {
        let focused = app.element(id: OneCADID.keyInputProxy)
        if focused.exists, ref.label == nil || focused.label == ref.label { return focused }
        let all = app.descendants(matching: .any)
        let fields = ref.ids.map { all.matching(identifier: $0).firstMatch }
        // Labels such as X also belong to StaticText. Only known editor field
        // identifiers can receive the tap that starts a native input session.
        return fields.first(where: { $0.exists }) ?? fields[0]
    }

    /// Selected state is the public panel state; tooltip text is not its label.
    func setExplorer(open: Bool) {
        let toggles = app.buttons.matching(NSPredicate(format: "label == 'Explorer'"))
        if let toggle = toggles.allElementsBoundByIndex.first, toggle.isSelected != open {
            toggle.tap()
        }
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            MainActor.assumeIsolated {
                let current = toggles.allElementsBoundByIndex
                return current.count == 1 && current[0].isSelected == open
            }
        }, object: nil)
        XCTAssertEqual(XCTWaiter().wait(for: [settled], timeout: 10), .completed,
                       "Explorer did not settle in the requested state")
    }

    func selectBody() {
        setExplorer(open: true)
        tap(UI.bodyRow)
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
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        test.add(shot)
    }
}

/// The system software keyboard, driven key by key through public XCUITest
/// taps. The focused Rust draft is exposed through the native text proxy;
/// every key reaches UIKit's text system exactly as a user's tap does.
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
        if name.count == 1, "0123456789".contains(name), !key(name).exists {
            // Some fields expose the locale's alphabet keyboard; its visible
            // numeric hints are flick gestures, not separate tappable keys.
            let numbers = key("more")
            test.waitUntil(numbers, .hittable, "keyboard numeric layout", file: file, line: line)
            numbers.tap()
            dismissKeyboardTutorial(file: file, line: line)
        }
        let element = key(name)
        test.waitUntil(element, .hittable, timeout: 10, "keyboard key '\(name)'", file: file, line: line)
        element.tap()
    }

    private func dismissKeyboardTutorial(file: StaticString, line: UInt) {
        let message = keyboard.staticTexts["To access the alternate letters on the keyboard,\npull down on the key and release."]
        guard message.waitForExistence(timeout: 1) else { return }
        let okay = keyboard.buttons["OK"]
        test.waitUntil(okay, .hittable, "keyboard alternate-letter tutorial", file: file, line: line)
        okay.tap()
        test.waitUntil(message, .gone, "keyboard tutorial dismissed", file: file, line: line)
    }

    func type(_ text: String, file: StaticString = #filePath, line: UInt = #line) {
        for character in text { press(String(character), file: file, line: line) }
    }

    /// Read the acknowledged native draft after focus changes, rather than
    /// retaining a virtual field that disappears from the accessibility tree.
    @discardableResult
    func replaceFocused(with text: String, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        waitShown(file: file, line: line)
        let focused = app.element(id: OneCADID.keyInputProxy)
        test.waitUntil(focused, .exists, "focused native text input", file: file, line: line)
        let current = focused.value as? String ?? ""
        for _ in current { press("delete", file: file, line: line) }
        type(text, file: file, line: line)
        return focused
    }

    func done(file: StaticString = #filePath, line: UInt = #line) {
        press("Done", file: file, line: line)
        test.waitUntil(keyboard, .gone, "keyboard dismissed before viewport picks", file: file, line: line)
    }
}

/// Projection for a new document before Fit or navigation changes its scale.
/// Camera::default (onecad-scene/src/camera.rs) uses offset (400,-400,300)
/// and a 76° vertical FOV. sketch_entry::align_camera preserves that distance,
/// centres the support origin and aligns screen right/up with sketch X/Y.
@MainActor
struct WorkflowSketchView {
    static let halfFOVTangent = tan(38 * Double.pi / 180)
    private static let cameraOffset = SIMD3<Double>(400, -400, 300)
    private static let cameraDistance = simd_length(cameraOffset)

    let ui: OneCADScreen

    func point(_ x: CGFloat, _ y: CGFloat) -> XCUICoordinate {
        // Fetch after the keyboard/layout transition, rather than retaining a
        // rectangle from the empty document before its chrome was laid out.
        let frame = ui.wait(UI.viewport, .exists).frame
        XCTAssertGreaterThan(frame.height, 0)
        let scale = frame.height / CGFloat(2 * Self.cameraDistance * Self.halfFOVTangent)
        return ui.point(frame.midX + x * scale, frame.midY - y * scale)
    }

}
