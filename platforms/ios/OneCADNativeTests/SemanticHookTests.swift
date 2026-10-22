import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Native acceptance through the semantic hooks, against the real in-process
/// kernel. Artifacts land in Documents/acceptance/<run id>/ for
/// `cargo xtask ios-simulator-gate` (docs/ios/SIMULATOR_GATE.md).
@MainActor
final class SemanticHookTests: XCTestCase {
    override func tearDown() async throws {
        if let app = AppDelegate.shared?.bridge?.app {
            try? SemanticHooks(app: app).fault("clear")
        }
    }

    func testKernelIdentityIsTheInProcessSimulatorKernel() async throws {
        let hooks = try await SemanticHooks.attach(self)
        let identity = try hooks.kernelIdentity()
        let embedded = try XCTUnwrap(identity["embedded"] as? [String: Any], "no linked identity")
        XCTAssertEqual(embedded["kind"] as? String, "embedded")
        XCTAssertEqual(embedded["platform"] as? String, "iphonesimulator")
        XCTAssertEqual(embedded["testHooks"] as? Bool, true)
        try await waitFor("document host started", timeout: 30) {
            (try hooks.kernelIdentity())["backend"] as? String != nil
        }
        XCTAssertEqual(try hooks.kernelIdentity()["backend"] as? String, "in-process")
    }

    /// Opens the gate's Mac-authored plate, checks it regenerated here, and
    /// re-saves it for the Mac oracle (`mac-plate-resaved`).
    func testMacFixtureRoundTrip() async throws {
        let documents = try await DocumentDriver.attach(self)
        try await documents.closeEveryTab()
        let hooks = documents.hooks
        let runID = try XCTUnwrap(hooks.runID, "ONECAD_RUN_ID (TEST_RUNNER_ONECAD_RUN_ID) not set")
        XCTAssertFalse(runID.isEmpty)
        let opened = try await documents.openMacPlate(as: "mac-round-trip")
        let solved = try await ModelDriver(documents: documents).solveFirstSketch()
        try assertSamePlate(opened, solved)
        let saved = try await hooks.saveArtifact("mac-plate-resaved", in: self)
        XCTAssertEqual(saved["saved"] as? Bool, true, "\(saved)")
        XCTAssertEqual(saved["clean"] as? Bool, true, "\(saved)")
        _ = try await hooks.exportStepArtifact("mac-plate-resaved", in: self)
        _ = try await hooks.writeSemantic("mac-plate-resaved", in: self)
        try await documents.closeEveryTab()
    }

    private func assertSamePlate(_ opened: [String: Any], _ solved: [String: Any]) throws {
        XCTAssertEqual(DocumentDriver.document(solved)["documentId"] as? String,
                       DocumentDriver.document(opened)["documentId"] as? String)
        let records = { (summary: [String: Any]) in summary["features"] as? [[String: Any]] ?? [] }
        XCTAssertEqual(records(solved).compactMap { $0["id"] as? String },
                       records(opened).compactMap { $0["id"] as? String })
        XCTAssertEqual(DocumentDriver.features(solved), ["Sketch", "Extrude"])
        let before = try XCTUnwrap((opened["sketches"] as? [[String: Any]])?.first)
        let after = try XCTUnwrap((solved["sketches"] as? [[String: Any]])?.first)
        XCTAssertEqual(after["id"] as? String, before["id"] as? String)
        XCTAssertEqual(after["entities"] as? Int, 14)
        XCTAssertEqual(after["constraints"] as? Int, 13)
        XCTAssertEqual(after["status"] as? String, "FullyConstrained")
        XCTAssertEqual(after["dof"] as? Int, 0)
    }

    /// The two injectable faults reach the paths a user's actions take: a
    /// Save As fails in the container write and the editor reports it; kernel
    /// work with a blocked crash journal is counted, never fatal.
    func testInjectedFaultsHitTheNormalPaths() async throws {
        let hooks = try await SemanticHooks.attach(self)
        try await openMacFixture(hooks)
        try hooks.fault("save_failure")
        let failed = try await hooks.saveArtifact("save-failure", in: self)
        XCTAssertEqual(failed["saved"] as? Bool, false, "\(failed)")
        XCTAssertEqual(failed["injectedFailure"] as? Bool, true, "\(failed)")
        // The editor's own status names the container write's EISDIR.
        let status = failed["status"] as? String ?? ""
        XCTAssertTrue(status.contains("Is a directory"), "\(failed)")
        let before = try hooks.kernelIdentity()["journalWriteFailures"] as? Int ?? 0
        let armed = try hooks.fault("journal_write")
        XCTAssertEqual(armed["journalWrite"] as? Bool, true)
        // Mass properties and tessellation are kernel jobs the journal records.
        _ = try await hooks.documentSummary(in: self)
        let after = try hooks.kernelIdentity()["journalWriteFailures"] as? Int ?? 0
        XCTAssertGreaterThan(after, before, "a blocked journal write was not counted")
        let cleared = try hooks.fault("clear")
        XCTAssertEqual(cleared["journalWrite"] as? Bool, false)
        let summary = try await hooks.documentSummary(in: self)
        XCTAssertFalse(summary.isEmpty, "the document is still served after clear")
    }

    /// Opens (or finds open) the gate's Mac-authored plate and waits until it
    /// regenerated in-process.
    private func openMacFixture(_ hooks: SemanticHooks) async throws {
        let fixture = try XCTUnwrap(ProcessInfo.processInfo.environment["ONECAD_MAC_FIXTURE"],
                                    "TEST_RUNNER_ONECAD_MAC_FIXTURE not set")
        let copy = try copyIntoDocuments(URL(fileURLWithPath: fixture))
        let status = copy.path.withCString { onecad_ios_open_path(hooks.app, $0) }
        XCTAssertEqual(status, ONECAD_IOS_OK)
        // Each summary waits for its ticket on the run loop, so this loop
        // yields to frames between polls; it ends once the plate regenerated.
        var summary: [String: Any] = [:]
        let deadline = Date().addingTimeInterval(90)
        while Date() < deadline, !Self.isPlate(summary) {
            summary = (try? await hooks.documentSummary(in: self)) ?? [:]
            if !Self.isPlate(summary) { try await nextRunLoopTick() }
        }
        XCTAssertTrue(Self.isPlate(summary), "\(summary)")
    }

    private static func isPlate(_ summary: [String: Any]) -> Bool {
        let features = (summary["features"] as? [[String: Any]] ?? []).compactMap { $0["opType"] as? String }
        let bodies = summary["bodies"] as? [[String: Any]] ?? []
        return features == ["Sketch", "Extrude"] && bodies.count == 1
    }

    private func copyIntoDocuments(_ source: URL) throws -> URL {
        let documents = try FileManager.default.url(for: .documentDirectory, in: .userDomainMask,
                                                    appropriateFor: nil, create: true)
        let directory = documents.appendingPathComponent("acceptance-input", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let target = directory.appendingPathComponent(source.lastPathComponent)
        if FileManager.default.fileExists(atPath: target.path) {
            try FileManager.default.removeItem(at: target)
        }
        try FileManager.default.copyItem(at: source, to: target)
        return target
    }
}
#else
/// Makes a non-Acceptance run visibly skip instead of passing with 0 tests.
final class SemanticHookTests: XCTestCase {
    func testKernelIdentityIsTheInProcessSimulatorKernel() throws {
        throw XCTSkip("semantic test hooks exist only in the Acceptance configuration")
    }

    func testMacFixtureRoundTrip() throws {
        throw XCTSkip("semantic test hooks exist only in the Acceptance configuration")
    }

    func testInjectedFaultsHitTheNormalPaths() throws {
        throw XCTSkip("semantic test hooks exist only in the Acceptance configuration")
    }
}
#endif
