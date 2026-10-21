import XCTest

/// Public XCUITest of the real app (evidence class 4, docs/ios/ARCHITECTURE.md).
@MainActor
final class LaunchTests: XCTestCase {
    override func setUp() async throws {
        continueAfterFailure = false
    }

    func testLaunchShowsCanvasAndFileMenu() {
        let app = launchOneCAD()
        XCTAssertFalse(app.element(id: OneCADID.startupError).exists, "app reported a startup failure")
        XCTAssertFalse(app.element(id: OneCADID.relaunchRequired).exists, "app is in relaunch-required state")
        // Exported by Rust through onecad_ios_a11y_snapshot (egui File menu).
        waitUntil(app.element(id: OneCADID.fileMenu), .hittable, "File menu not exported by the a11y snapshot")
    }
}
