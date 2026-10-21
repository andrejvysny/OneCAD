import XCTest

@MainActor
final class OrientationTests: XCTestCase {
    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        XCUIDevice.shared.orientation = .portrait
    }

    /// Rotates through every iPad orientation; after each one the canvas and
    /// the Rust-exported File menu must be reachable again.
    func testRotationKeepsAppResponsive() {
        let app = launchOneCAD()
        let orientations: [UIDeviceOrientation] = [.landscapeLeft, .portraitUpsideDown, .landscapeRight, .portrait]
        for orientation in orientations {
            XCUIDevice.shared.orientation = orientation
            waitUntil(app.element(id: OneCADID.canvas), .hittable, "canvas after rotating to \(orientation.rawValue)")
            waitUntil(app.element(id: OneCADID.fileMenu), .hittable, "File menu after rotating to \(orientation.rawValue)")
            XCTAssertEqual(app.state, .runningForeground)
        }
    }
}
