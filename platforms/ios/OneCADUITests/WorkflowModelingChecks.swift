import XCTest
import simd

/// Public UI checks start from, and restore, the required 60 × 40 × 4 mm
/// plate with a through hole. Kernel measurements belong to native tests.
@MainActor
struct WorkflowModelingChecks {
    let ui: OneCADScreen
    let keyboard: SoftwareKeyboard
    let test: XCTestCase

    private let baseline = ["Sketch", "Extrude", "Sketch", "Extrude"]

    func edgeTreatment(_ tool: UIRef, feature: String) {
        ui.expectHistory(baseline)
        selectOuterVerticalEdge()
        start(tool)
        setValue(UI.operationValue, "1")
        ui.wait(UI.create, .enabled)
        ui.screenshot("\(feature.lowercased())-1mm-preview")
        commit(feature, detail: "\(feature) · 1.0 mm")
        ui.screenshot("\(feature.lowercased())-1mm-committed")
        undo(feature)
        // Chamfer starts on the original sharp edge, never on a fillet.
        selectOuterVerticalEdge()
    }

    func offsetFace() {
        expectThickness(4, probe: .nothing)
        beginOffset()
        ui.wait(UI.create, .enabled)
        ui.screenshot("offset-face-4mm-preview-before-cancel")
        ui.tap(UI.cancelOperation)
        ui.wait(UI.create, .gone)
        ui.expectHistory(baseline)
        expectThickness(4, probe: .nothing)

        beginOffset()
        ui.wait(UI.create, .enabled)
        ui.screenshot("offset-face-4mm-preview")
        commit("Offset face", detail: "OffsetFace · 4.0 mm")
        expectThickness(8, probe: .face)
        ui.screenshot("offset-face-8mm-thickness")
        undo("Offset face")
        expectThickness(4, probe: .nothing)
    }

    func moveRotate() {
        let view = fittedView(.top)
        let left = view.point(5, 20, 4)
        let holeCentre = view.point(30, 20, 4)
        expectPicks([(left, .face), (holeCentre, .nothing)], "baseline before Move / Rotate")
        ui.tap(UI.bodyRow)
        ui.waitText("Health: Healthy")
        start(UI.moveRotate)
        setValue(UI.operationValue, "10")
        setValue(UI.translationY, "5")
        setValue(UI.translationZ, "0")
        setValue(UI.rotationAngle, "90")
        ui.wait(UI.create, .enabled)
        ui.screenshot("move-10-5-0-rotate-90-preview")
        commit("Move / Rotate", detail: "TransformBody · Δ(10.0, 5.0, 0.0)")
        // Sketch XY maps to world (-Y,X). Rotation about (-20,30,2),
        // then translation, gives world bounds [-40,20] × [15,55].
        // The hole moves from (-20,30) to (-10,35), exposing solid material
        // under its old centre while the low-Y probe leaves the plate.
        expectPicks([(left, .nothing), (holeCentre, .face)], "translated and rotated plate")
        ui.screenshot("move-10-5-0-rotate-90-committed")
        undo("Move / Rotate")
        expectPicks([(left, .face), (holeCentre, .nothing)], "Move / Rotate undo restored plate")
    }

    private func beginOffset() {
        let view = fittedView(.home)
        expectPicks([(view.point(30, 10, 4), .face)], "top face for Offset face")
        start(UI.offsetFace)
        // A 4 mm change leaves room beyond the 22 pt finger edge window to
        // distinguish the new side face from the original top edge.
        setValue(UI.operationValue, "4")
    }

    private func selectOuterVerticalEdge() {
        for attempt in 1...4 {
            let view = fittedView(.home)
            ui.tap(UI.propertiesTab)
            view.point(30, 10, 4).tap()
            guard ui.text("Face").waitForExistence(timeout: 5) else { continue }
            view.point(0, 0, 2).tap()
            if ui.text("Edge").waitForExistence(timeout: 5), ui.text("on Body 1").exists { return }
            if attempt == 4 { XCTFail("the visible 4 mm outer edge at sketch (0,0,2) was not selected") }
        }
        XCTFail("Home view did not expose the plate's top face and vertical outer edge")
    }

    private func expectThickness(_ height: Double, probe: RequiredWorkflowTests.Pick) {
        let view = fittedView(.right, height: height)
        expectPicks([(view.point(30, 0, 2), .face), (view.point(30, 0, 6), probe)],
                    "plate thickness \(height) mm at z = 6 mm")
    }

    private func start(_ tool: UIRef) {
        if !ui.find(tool).exists { ui.tap(UI.moreTools) }
        ui.tap(tool)
        ui.wait(UI.operationValue)
        ui.wait(UI.cancelOperation)
    }

    private func setValue(_ ref: UIRef, _ value: String) {
        let field = ui.wait(ref)
        field.tap()
        keyboard.waitShown()
        keyboard.replace(field.value as? String ?? "", with: value)
        test.waitUntil(field, .value(value), "\(ref) shows \(value)")
    }

    private func commit(_ feature: String, detail: String) {
        ui.wait(UI.create, .enabled).tap()
        ui.wait(UI.create, .gone)
        ui.expectHistory(baseline + [feature])
        ui.historyRowElements().last?.tap()
        ui.waitText(detail)
        ui.tap(UI.propertiesTab)
        ui.tap(UI.bodyRow)
        ui.waitText("Health: Healthy")
        XCTAssertFalse(ui.text("Body 2").exists, "\(feature) must preserve one body")
    }

    private func undo(_ feature: String) {
        ui.tap(UI.undo)
        ui.tap(UI.historyTab)
        test.waitUntil(ui.nth(feature, 0), .gone, "Undo removed \(feature)")
        ui.expectHistory(baseline)
        ui.tap(UI.propertiesTab)
        ui.tap(UI.bodyRow)
        ui.waitText("Health: Healthy")
    }

    private func expectPicks(_ picks: [(XCUICoordinate, RequiredWorkflowTests.Pick)], _ what: String) {
        ui.tap(UI.propertiesTab)
        for _ in 0..<4 {
            let matched = picks.allSatisfy { point, pick in
                point.tap()
                guard ui.text(pick.rawValue).waitForExistence(timeout: 5) else { return false }
                return pick == .nothing || ui.text("on Body 1").exists
            }
            if matched { return }
        }
        XCTFail("\(what): expected \(picks.map(\.1.rawValue))")
    }

    private func fittedView(_ direction: WorkflowFittedPlateView.Direction, height: Double = 4) -> WorkflowFittedPlateView {
        ui.tap(UI.homeView)
        switch direction {
        case .home: break
        case .top: ui.tap(UI.topView)
        case .right: ui.tap(UI.rightView)
        }
        ui.tap(UI.fitAll)
        return WorkflowFittedPlateView(ui: ui, direction: direction, height: height)
    }
}

/// Screen projection for public Home/Top/Right + Fit controls and known plate
/// bounds. Mirrors camera.rs/fit.rs constants; no scene or test-hook reads.
@MainActor
struct WorkflowFittedPlateView {
    enum Direction { case home, top, right }

    let ui: OneCADScreen
    let viewport: CGRect
    let center: SIMD3<Double>
    let right: SIMD3<Double>
    let up: SIMD3<Double>
    let back: SIMD3<Double>
    let distance: Double
    let tangent: Double

    init(ui: OneCADScreen, direction: Direction, width: Double = 60, height: Double) {
        self.ui = ui
        viewport = ui.find(UI.viewport).frame
        center = SIMD3(-20, width / 2, height / 2)
        tangent = tan(76 * .pi / 360)
        switch direction {
        case .home: back = simd_normalize(SIMD3(400, -400, 300))
        case .top: back = SIMD3(0, 0, 1)
        case .right: back = SIMD3(1, 0, 0)
        }
        right = direction == .top ? SIMD3(1, 0, 0) : simd_normalize(SIMD3(-back.y, back.x, 0))
        up = simd_cross(back, right)
        let horizontalTangent = tangent * viewport.width / viewport.height
        var required = 0.5
        for x in [-40.0, 0] {
            for y in [0.0, width] {
                for z in [0.0, height] {
                    let relative = SIMD3(x, y, z) - center
                    let depth = simd_dot(relative, back)
                    required = max(required, abs(simd_dot(relative, right)) / horizontalTangent + depth,
                                   abs(simd_dot(relative, up)) / tangent + depth, depth + 0.1)
                }
            }
        }
        distance = required * 1.15
    }

    func point(_ x: Double, _ y: Double, _ z: Double) -> XCUICoordinate {
        let relative = SIMD3(-y, x, z) - center
        let depth = distance - simd_dot(relative, back)
        let scale = viewport.height / (2 * tangent * depth)
        return ui.point(viewport.midX + simd_dot(relative, right) * scale,
                        viewport.midY - simd_dot(relative, up) * scale)
    }
}
