import XCTest

/// Surface lifecycle through public UI (evidence class 4, simulator only): the CAD
/// viewport is drawn again after rotation and after a background/foreground cycle,
/// which releases and recreates the renderer's targets (docs/ios/HOST.md, Lifecycle).
@MainActor
final class SurfaceLifecycleTests: XCTestCase {
    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        XCUIDevice.shared.orientation = .portrait
    }

    func testViewportReturnsAfterRotationAndBackgrounding() {
        let app = launchOneCAD()
        let fileMenu = app.element(id: OneCADID.fileMenu)
        waitUntil(fileMenu, .hittable, "File menu")
        fileMenu.tap()
        let newDocument = app.element(id: OneCADID.fileNew)
        waitUntil(newDocument, .hittable, "File › New")
        newDocument.tap()
        let viewport = app.element(id: OneCADID.viewport)
        waitUntil(viewport, .hittable, "viewport of the new document")
        attachScreenshot(app, "1-document")

        XCUIDevice.shared.orientation = .landscapeLeft
        waitUntil(viewport, .hittable, "viewport after rotating")
        attachScreenshot(app, "2-rotated")

        XCUIDevice.shared.press(.home)
        XCTAssertTrue(waitForBackground(app, timeout: 20), "app did not leave the foreground")
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20), "app did not return")
        waitUntil(viewport, .hittable, "viewport after returning to the foreground")
        waitUntil(fileMenu, .hittable, "File menu after returning to the foreground")
        XCTAssertFalse(app.element(id: OneCADID.relaunchRequired).exists)
        attachScreenshot(app, "3-foreground")
    }

    /// The simulator may keep the app in `.runningBackground` without suspending it.
    private func waitForBackground(_ app: XCUIApplication, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if app.state == .runningBackground || app.state == .runningBackgroundSuspended {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        } while Date() < deadline
        return false
    }

    private func attachScreenshot(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
