import XCTest

/// Accessibility identifiers shared with the app and the Rust snapshot.
/// Catalogue and ownership: docs/ios/TESTING.md.
enum OneCADID {
    static let canvas = "onecad.canvas"
    static let startupError = "onecad.error.startup"
    static let relaunchRequired = "onecad.error.relaunchRequired"
    static let fileMenu = "onecad.menu.file"
    static let saveName = "onecad.save.name"
}

extension XCUIApplication {
    /// Any element type: Rust-exported nodes surface as buttons, static text
    /// or plain elements depending on their role.
    func element(id: String) -> XCUIElement {
        descendants(matching: .any).matching(identifier: id).firstMatch
    }
}

extension XCTestCase {
    /// Readiness by predicate, never by sleeping. Fails with `message` on
    /// timeout and returns whether the condition was met.
    @MainActor
    @discardableResult
    func waitUntil(_ element: XCUIElement, _ condition: ElementCondition,
                   timeout: TimeInterval = 20, _ message: @autoclosure () -> String = "",
                   file: StaticString = #filePath, line: UInt = #line) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: condition.predicate, object: element)
        let result = XCTWaiter().wait(for: [expectation], timeout: timeout)
        let met = result == .completed
        if !met {
            let detail = message()
            XCTFail("\(element) not \(condition) after \(timeout)s\(detail.isEmpty ? "" : ": " + detail)",
                    file: file, line: line)
        }
        return met
    }

    /// Launches the app and waits until the canvas host view is on screen.
    @MainActor
    func launchOneCAD(file: StaticString = #filePath, line: UInt = #line) -> XCUIApplication {
        let app = XCUIApplication()
        app.launch()
        waitUntil(app.element(id: OneCADID.canvas), .exists, "canvas never appeared",
                  file: file, line: line)
        return app
    }
}

enum ElementCondition: CustomStringConvertible {
    case exists
    case hittable
    case gone
    case enabled
    case value(String)

    var predicate: NSPredicate {
        switch self {
        case .exists: NSPredicate(format: "exists == true")
        case .hittable: NSPredicate(format: "exists == true AND hittable == true")
        case .gone: NSPredicate(format: "exists == false")
        case .enabled: NSPredicate(format: "exists == true AND hittable == true AND enabled == true")
        case let .value(text): NSPredicate(format: "exists == true AND value == %@", text)
        }
    }

    var description: String {
        switch self {
        case .exists: "existing"
        case .hittable: "hittable"
        case .gone: "gone"
        case .enabled: "enabled"
        case let .value(text): "showing '\(text)'"
        }
    }
}
