import simd
import UIKit
import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Confirmed synthetic Pencil picks and public editor controls, never hardware evidence.
@MainActor
extension ModelDriver {
    func offsetTopFace(cancel: Bool) async throws -> [String: Any] {
        let view = try await fittedHomeView()
        let point = try project(SIMD3(-10, 10, 4), summary: view, orthographic: false)
        documents.diagnostic("before top face pick\npoint \(point), view \(DocumentDriver.view(view))")
        try await documents.activate(label: "Properties")
        if cancel || documents.node({ $0.label == "Face" }) == nil { try await pencilTap(point) }
        try await waitForText("Face")
        try await tool("offsetFace")
        try await replaceField("onecad.dimension.input", with: "4")
        try await documents.test.waitFor("Offset face preview ready", timeout: 60) {
            documents.node {
                $0.identifier == "onecad.op.confirm" && $0.flags & ONECAD_IOS_A11Y_DISABLED == 0
            } != nil
        }
        if cancel { try await documents.activate("onecad.op.cancel") }
        else { try await confirmOperation() }
        return try await operationSummary(cancel ? nil : "OffsetFace")
    }

    func transformBody() async throws -> [String: Any] {
        let explorer = try XCTUnwrap(documents.node { $0.identifier == "button.explorer" })
        if explorer.flags & ONECAD_IOS_A11Y_SELECTED == 0 { try await documents.activate("button.explorer") }
        try await documents.activate("other.body-1")
        try await tool("moveRotate")
        for (identifier, value) in [
            ("onecad.dimension.input", "10"), ("onecad.param.translationY", "5"),
            ("onecad.param.translationZ", "0"), ("onecad.param.rotationAngle", "90"),
        ] {
            documents.diagnostic("before Move / Rotate field \(identifier) = \(value)")
            do { try await replaceField(identifier, with: value) }
            catch {
                documents.diagnostic("Move / Rotate field failed: \(identifier) = \(value): \(error)")
                throw error
            }
            documents.diagnostic("after Move / Rotate field \(identifier) = \(value)")
        }
        try await confirmOperation()
        return try await operationSummary("TransformBody")
    }

    func undoModelOperation() async throws -> [String: Any] {
        try await key(0x1D, modifiers: ONECAD_IOS_MOD_COMMAND)
        return try await operationSummary(nil)
    }

    private func operationSummary(_ operation: String?) async throws -> [String: Any] {
        let expected = ["Sketch", "Extrude"] + (operation.map { [$0] } ?? [])
        return try await documents.waitForSummary("model operation \(operation ?? "baseline")", timeout: 90) {
            DocumentDriver.features($0) == expected && !DocumentDriver.busy($0)
                && ($0["bodies"] as? [Any])?.count == 1
                && !documents.exists("onecad.op.confirm")
                && ($0["features"] as? [[String: Any]])?.allSatisfy { $0["status"] as? String == "ok" } == true
        }
    }

    func fittedHomeView() async throws -> [String: Any] {
        try await documents.test.waitFor("native keyboard ended before fit", timeout: 15) {
            Self.canvas()?.subviews.compactMap { $0 as? KeyInputProxy }.allSatisfy { !$0.isFirstResponder } == true
        }
        let explorer = try XCTUnwrap(documents.node { $0.identifier == "button.explorer" })
        if explorer.flags & ONECAD_IOS_A11Y_SELECTED != 0 { try await documents.activate("button.explorer") }
        try await documents.activate(label: "Home view")
        try await documents.activate(label: "Fit model in view")
        var previous: [Double] = []
        var stable = 0
        return try await documents.waitForSummary("home perspective fit settled") {
            let view = DocumentDriver.view($0)
            guard let offset = view["cameraOffset"] as? [Double], let target = view["cameraTarget"] as? [Double],
                  let frame = documents.node({ $0.identifier == "onecad.viewport" })?.frame,
                  !DocumentDriver.busy($0) else { return false }
            let current = offset + target + [frame.minX, frame.minY, frame.width, frame.height].map(Double.init)
            stable = current == previous ? stable + 1 : 0
            previous = current
            return stable >= 5
        }
    }
}
#endif
