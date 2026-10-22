import XCTest
@testable import OneCAD

#if ONECAD_ACCEPTANCE
/// Coordinated persistence against the real in-process kernel (simulator
/// evidence, docs/ios/TESTING.md): a failed overwrite keeps the previous file
/// byte-identical, and a malformed STEP handed over by Files changes nothing.
@MainActor
final class DocumentPersistenceTests: XCTestCase {
    private var driver: DocumentDriver?
    private var artifacts: [URL] = []

    override func tearDown() async throws {
        if let hooks = driver?.hooks { try? hooks.fault("clear") }
        try? await driver?.closeEveryTab()
        // The gate accepts only its own model artifacts in the run directory.
        for url in artifacts {
            try? FileManager.default.removeItem(at: url)
            try? FileManager.default.removeItem(at: URL(fileURLWithPath: url.path + ".provenance.json"))
        }
    }

    private func start() async throws -> DocumentDriver {
        let docs = try await DocumentDriver.attach(self)
        driver = docs
        try await docs.closeEveryTab()
        return docs
    }

    /// Save, edit (a new sketch), then a Save over the same file fails in the container
    /// write: the previous container keeps its SHA-256, the document stays
    /// dirty and bound to it, and the status names the failure. The next
    /// Save replaces the file whole.
    func testFailedOverwriteKeepsThePreviousFileByteIdentical() async throws {
        let docs = try await start()
        _ = try await docs.openMacPlate(as: "persistence-plate")
        let first = try await docs.hooks.saveArtifact("persistence-overwrite", in: self)
        XCTAssertEqual(first["saved"] as? Bool, true, "\(first)")
        let file = URL(fileURLWithPath: try XCTUnwrap(first["path"] as? String))
        artifacts.append(file)
        let previous = try DocumentDriver.sha256(file)
        let edited = try await docs.addSketch()
        XCTAssertEqual(DocumentDriver.dirty(edited), true)
        try docs.hooks.fault("save_failure")
        let failed = try await docs.hooks.saveArtifact("persistence-overwrite", in: self)
        XCTAssertEqual(failed["saved"] as? Bool, false, "\(failed)")
        XCTAssertEqual(failed["blocked"] as? String, "readOnly", "\(failed)")
        XCTAssertEqual(try DocumentDriver.sha256(file), previous, "the failed overwrite touched the previous file")
        let after = try await docs.waitForSummary("save failure reported") { !DocumentDriver.busy($0) }
        XCTAssertEqual(DocumentDriver.dirty(after), true, "a failed save must not report the document saved")
        XCTAssertEqual(DocumentDriver.document(after)["revision"] as? Int,
                       DocumentDriver.document(edited)["revision"] as? Int)
        XCTAssertEqual((DocumentDriver.document(after)["path"] as? String).map { URL(fileURLWithPath: $0).lastPathComponent },
                       file.lastPathComponent, "still bound to its previous file")
        XCTAssertTrue(DocumentDriver.status(after).contains("Permission denied"), DocumentDriver.status(after))
        let saved = try await docs.hooks.saveArtifact("persistence-overwrite", in: self)
        XCTAssertEqual(saved["clean"] as? Bool, true, "\(saved)")
        XCTAssertNotEqual(try DocumentDriver.sha256(file), previous, "the next save replaced the file")
        let clean = try await docs.waitForSummary("saved state") { !DocumentDriver.busy($0) }
        XCTAssertEqual(DocumentDriver.dirty(clean), false)
    }

    /// A malformed STEP from Files ("Open in": copied into Documents/Inbox,
    /// then `open_path`) is refused without changing the open document, and
    /// the editor's status line says why.
    func testMalformedStepFromFilesLeavesTheDocumentUnchanged() async throws {
        let docs = try await start()
        let before = try await docs.openMacPlate(as: "import-plate")
        let inbox = try DocumentInbox.inboxDirectory()
        let step = inbox.appending(path: "malformed-\(UUID().uuidString.prefix(8)).step")
        try Data("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=NOT_AN_ENTITY(;\nENDSEC;\nEND-ISO-10303-21;\n".utf8)
            .write(to: step)
        artifacts.append(step)
        docs.openPath(step)
        let after = try await docs.waitForSummary("import refused") {
            DocumentDriver.tabCount($0) == 1 && !DocumentDriver.busy($0) && !DocumentDriver.status($0).isEmpty
                && DocumentDriver.status($0) != DocumentDriver.status(before)
        }
        XCTAssertEqual(DocumentDriver.features(after), DocumentDriver.features(before))
        XCTAssertEqual(DocumentDriver.document(after)["revision"] as? Int, DocumentDriver.document(before)["revision"] as? Int)
        XCTAssertEqual(DocumentDriver.dirty(after), DocumentDriver.dirty(before))
        XCTAssertEqual(DocumentDriver.document(after)["documentId"] as? String,
                       DocumentDriver.document(before)["documentId"] as? String)
        let status = DocumentDriver.status(after)
        XCTAssertFalse(status.contains("opened") || status.contains("pending"), status)
    }
}
#else
/// Makes a non-Acceptance run visibly skip instead of passing with 0 tests.
final class DocumentPersistenceTests: XCTestCase {
    func testFailedOverwriteKeepsThePreviousFileByteIdentical() throws {
        throw XCTSkip("semantic test hooks exist only in the Acceptance configuration")
    }

    func testMalformedStepFromFilesLeavesTheDocumentUnchanged() throws {
        throw XCTSkip("semantic test hooks exist only in the Acceptance configuration")
    }
}
#endif
