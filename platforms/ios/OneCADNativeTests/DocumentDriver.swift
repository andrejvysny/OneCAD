import CryptoKit
import UIKit
import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Drives the hosted app's documents through public paths only: Rust-exported
/// controls are activated by automation id (VoiceOver activation, as a tap),
/// files arrive through `onecad_ios_open_path` ("Open in"). Semantic hooks
/// only observe (docs/ios/TESTING.md).
@MainActor
struct DocumentDriver {
    let hooks: SemanticHooks
    let test: XCTestCase

    static func attach(_ test: XCTestCase) async throws -> DocumentDriver {
        DocumentDriver(hooks: try await SemanticHooks.attach(test), test: test)
    }

    private var bridge: RustBridge? { AppDelegate.shared?.bridge }

    func node(_ match: (AccessibilityNode) -> Bool) -> AccessibilityNode? {
        nodes().first(where: match)
    }

    func nodes() -> [AccessibilityNode] { bridge?.accessibilitySnapshot()?.nodes ?? [] }

    func exists(_ identifier: String) -> Bool {
        node { $0.identifier == identifier } != nil
    }

    /// Activates the first enabled node `match` accepts once it is exported.
    func activate(_ what: String, timeout: TimeInterval = 15,
                  where match: @escaping (AccessibilityNode) -> Bool) async throws {
        let enabled = { (node: AccessibilityNode) in
            match(node) && node.flags & UInt32(ONECAD_IOS_A11Y_DISABLED) == 0
        }
        do {
            try await test.waitFor("\(what) exported and enabled", timeout: timeout) { node(enabled) != nil }
        } catch {
            diagnostic("activation timeout: \(what)")
            throw error
        }
        let target = try XCTUnwrap(node(enabled))
        XCTAssertTrue(bridge?.activateAccessibility(target.id) ?? false, "activating \(what)")
        // Activation queues an egui event. Let the host consume it before a
        // following action can target a control from the previous snapshot.
        try await test.nextRunLoopTick()
    }

    func diagnostic(_ context: String) {
        let nodes = bridge?.accessibilitySnapshot()?.nodes ?? []
        let tree = nodes.map {
            "\($0.identifier) | \($0.label) | \($0.value) | role \($0.role) flags \($0.flags) | \($0.frame)"
        }.joined(separator: "\n")
        let geometry = surfaceDiagnostic()
        let attachment = XCTAttachment(string: context + "\n" + geometry + "\n" + tree)
        attachment.name = context.components(separatedBy: "\n").first ?? "native diagnostic"
        attachment.lifetime = .keepAlways
        test.add(attachment)
        print("NATIVE DIAGNOSTIC: \(context)\n\(geometry)\n\(tree)")
        if let window = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene })
            .flatMap(\.windows).first(where: \.isKeyWindow) {
            let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                _ = window.drawHierarchy(in: window.bounds, afterScreenUpdates: false)
            }
            let screenshot = XCTAttachment(image: image)
            screenshot.name = "native diagnostic screen"
            screenshot.lifetime = .keepAlways
            test.add(screenshot)
        }
    }

    private func surfaceDiagnostic() -> String {
        guard let canvas = ModelDriver.canvas(), let window = canvas.window,
              let scene = window.windowScene else { return "canvas/window/scene unavailable" }
        let screen = scene.screen
        let layer = canvas.metalLayer
        let desc = canvas.surfaceDescriptor()
        let proxies = canvas.subviews.compactMap { $0 as? KeyInputProxy }.map {
            "proxy frame=\($0.frame) firstResponder=\($0.isFirstResponder) session=\(String(describing: $0.session))"
        }.joined(separator: "\n")
        return """
        deviceOrientation=\(UIDevice.current.orientation.rawValue) sceneOrientation=\(scene.interfaceOrientation.rawValue)
        scene=\(scene.coordinateSpace.bounds) screen=\(screen.bounds) fixed=\(screen.fixedCoordinateSpace.bounds) coordinate=\(screen.coordinateSpace.bounds) scale=\(screen.scale) nativeScale=\(screen.nativeScale)
        window bounds=\(window.bounds) frame=\(window.frame) transform=\(window.transform) safeArea=\(window.safeAreaInsets)
        canvas bounds=\(canvas.bounds) frame=\(canvas.frame) transform=\(canvas.transform) safeArea=\(canvas.safeAreaInsets) generation=\(canvas.viewGeneration)
        canvasInWindow=\(canvas.convert(canvas.bounds, to: window)) canvasInScreen=\(canvas.convert(canvas.bounds, to: screen.coordinateSpace))
        layer bounds=\(layer.bounds) contentsScale=\(layer.contentsScale) drawable=\(layer.drawableSize) presentation=\(String(describing: layer.presentation()?.bounds))
        surface width=\(desc?.width_px ?? 0) height=\(desc?.height_px ?? 0) scale=\(desc?.scale ?? 0)
        \(proxies)
        """
    }

    func activate(_ identifier: String, timeout: TimeInterval = 15) async throws {
        try await activate(identifier, timeout: timeout) { $0.identifier == identifier }
    }

    func activate(label: String, timeout: TimeInterval = 15) async throws {
        try await activate("\"\(label)\"", timeout: timeout) { $0.label == label }
    }

    /// A row of the title bar's File menu (the menu opens first).
    func file(_ item: String) async throws {
        try await activate("onecad.menu.file")
        try await activate(item)
    }

    /// The active document's summary with the active tab's editor state
    /// (`view`), or nil while the active tab holds no document.
    func summary() async -> [String: Any]? {
        guard let summary = try? await hooks.documentSummary(in: test), !summary.isEmpty else { return nil }
        return summary
    }

    /// Polls summaries (yielding to frames between them) until `condition`.
    @discardableResult
    func waitForSummary(_ what: String, timeout: TimeInterval = 60,
                        _ condition: ([String: Any]) -> Bool) async throws -> [String: Any] {
        let deadline = Date().addingTimeInterval(timeout)
        var last: [String: Any] = [:]
        while Date() < deadline {
            if let summary = await summary() {
                last = summary
                if condition(summary) { return summary }
            }
            try await test.nextRunLoopTick()
        }
        throw SemanticHooks.Failure(description: "timed out: \(what); last summary: \(last)")
    }

    func openPath(_ url: URL) {
        let status = url.path.withCString { onecad_ios_open_path(hooks.app, $0) }
        XCTAssertEqual(status, ONECAD_IOS_OK, "open_path \(url.lastPathComponent)")
    }

    /// Opens a copy of the gate's Mac-authored plate (named `name`, so its
    /// path is this test's own) and waits until it regenerated in-process.
    func openMacPlate(as name: String) async throws -> [String: Any] {
        let fixture = try XCTUnwrap(ProcessInfo.processInfo.environment["ONECAD_MAC_FIXTURE"],
                                    "TEST_RUNNER_ONECAD_MAC_FIXTURE not set")
        let copy = try Self.copyIntoDocuments(URL(fileURLWithPath: fixture), as: "\(name).onecad")
        openPath(copy)
        return try await waitForSummary("plate \(name) regenerated", timeout: 90) {
            Self.features($0) == ["Sketch", "Extrude"] && ($0["bodies"] as? [Any])?.count == 1
                && !Self.busy($0)
        }
    }

    /// File › New Part: a new tab with an empty document.
    func newPart() async throws -> [String: Any] {
        let before = await summary().map(Self.tabCount) ?? 0
        try await file("onecad.menu.file.new")
        return try await waitForSummary("new part tab") {
            Self.tabCount($0) == before + 1 && Self.features($0).isEmpty && !Self.busy($0)
        }
    }

    /// An edit through the pill: Sketch on XY, then Finish, appends an empty
    /// Sketch feature to the active document.
    @discardableResult
    func addSketch() async throws -> [String: Any] {
        let before = await summary().map(Self.features) ?? []
        try await activate("onecad.tool.sketch")
        try await activate("onecad.plane.xy")
        try await activate("onecad.tool.finishSketch", timeout: 30)
        return try await waitForSummary("sketch added") {
            Self.features($0) == before + ["Sketch"] && !Self.busy($0)
        }
    }

    /// Shows tab `index` from the strip and waits until it is active.
    func showTab(_ index: Int) async throws -> [String: Any] {
        try await activate("onecad.tab.\(index)")
        return try await waitForSummary("tab \(index) shown") { Self.view($0)["tabIndex"] as? Int == index }
    }

    /// Closes every tab, discarding unsaved work, and resolves a leftover
    /// recovery offer, so tests sharing this process start from Home.
    func closeEveryTab() async throws {
        try await discardRecoveryOffer()
        for _ in 0..<8 {
            guard let summary = await summary(), Self.tabCount(summary) > 0 else { return }
            let count = Self.tabCount(summary)
            try await file("onecad.menu.file.closeTab")
            try await test.waitFor("tab closed", timeout: 30) {
                if node({ $0.label == "Don't save" }) != nil, let target = node({ $0.label == "Don't save" }) {
                    _ = bridge?.activateAccessibility(target.id)
                }
                return exists("onecad.tab.\(count - 1)") == false
            }
        }
    }

    func discardRecoveryOffer() async throws {
        guard node({ $0.label == "Discard recovery" }) != nil else { return }
        try await activate(label: "Discard recovery")
        try await test.waitFor("recovery resolved", timeout: 30) { node { $0.label == "Discard recovery" } == nil }
    }

    // MARK: Summary fields

    static func features(_ summary: [String: Any]) -> [String] {
        (summary["features"] as? [[String: Any]] ?? []).compactMap { $0["opType"] as? String }
    }

    static func document(_ summary: [String: Any]) -> [String: Any] {
        summary["document"] as? [String: Any] ?? [:]
    }

    static func view(_ summary: [String: Any]) -> [String: Any] {
        summary["view"] as? [String: Any] ?? [:]
    }

    static func tabCount(_ summary: [String: Any]) -> Int { summary["tabCount"] as? Int ?? 0 }
    static func title(_ summary: [String: Any]) -> String? { document(summary)["title"] as? String }
    static func dirty(_ summary: [String: Any]) -> Bool? { document(summary)["dirty"] as? Bool }
    static func status(_ summary: [String: Any]) -> String { view(summary)["status"] as? String ?? "" }
    static func busy(_ summary: [String: Any]) -> Bool { view(summary)["busy"] as? Bool ?? true }

    static func copyIntoDocuments(_ source: URL, as name: String) throws -> URL {
        let documents = try FileManager.default.url(for: .documentDirectory, in: .userDomainMask,
                                                    appropriateFor: nil, create: true)
        let directory = documents.appendingPathComponent("acceptance-input", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let target = directory.appendingPathComponent(name)
        if FileManager.default.fileExists(atPath: target.path) {
            try FileManager.default.removeItem(at: target)
        }
        try FileManager.default.copyItem(at: source, to: target)
        return target
    }

    static func sha256(_ url: URL) throws -> String {
        SHA256.hash(data: try Data(contentsOf: url)).map { String(format: "%02x", $0) }.joined()
    }
}
#endif
