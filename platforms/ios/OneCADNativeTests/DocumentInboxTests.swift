import XCTest
@testable import OneCAD

/// Managed-copy policy (docs/ios/HOST.md, Files and documents): an external
/// file's security scope covers the whole coordinated copy into
/// Documents/Imported and is released on every path. Pure host behaviour; no
/// Rust call, so it runs in every test configuration.
final class DocumentInboxTests: XCTestCase {
    /// Records scope calls and what the inbox held when the scope ended.
    final class ScopeLog: @unchecked Sendable {
        private let lock = NSLock()
        private var events: [String] = []
        let granted: Bool

        init(granted: Bool = true) { self.granted = granted }

        var all: [String] { lock.withLock { events } }

        func record(_ event: String) { lock.withLock { events.append(event) } }

        func scope(checking copy: URL? = nil) -> SecurityScope {
            SecurityScope(
                start: { url in
                    self.record("start \(url.lastPathComponent)")
                    return self.granted
                },
                stop: { url in
                    let copied = copy.map { FileManager.default.fileExists(atPath: $0.path) } ?? false
                    self.record("stop \(url.lastPathComponent) copied=\(copied)")
                })
        }
    }

    private var created: [URL] = []

    override func tearDownWithError() throws {
        for url in created { try? FileManager.default.removeItem(at: url) }
        created = []
    }

    /// A source outside the inbox, as a provider or picker hands it over.
    private func externalFile(_ contents: String = "ISO-10303-21;\n") throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "inbox-tests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        created.append(directory)
        let source = directory.appending(path: "provided-\(UUID().uuidString.prefix(8)).step")
        try Data(contents.utf8).write(to: source)
        return source
    }

    private func inboxCopy(of source: URL) throws -> URL {
        let copy = try DocumentInbox.workingDirectory().appending(path: source.lastPathComponent)
        created.append(copy)
        return copy
    }

    func testScopeCoversTheWholeCopyAndEndsAfterIt() throws {
        let source = try externalFile()
        let expected = try inboxCopy(of: source)
        let log = ScopeLog()
        let copy = try DocumentInbox.importCopy(of: source, scope: log.scope(checking: expected))
        XCTAssertEqual(copy.lastPathComponent, source.lastPathComponent)
        XCTAssertEqual(try Data(contentsOf: copy), try Data(contentsOf: source))
        let name = source.lastPathComponent
        XCTAssertEqual(log.all, ["start \(name)", "stop \(name) copied=true"],
                       "the scope ends exactly once, after the copy exists")
    }

    func testScopeIsReleasedWhenTheCopyFails() throws {
        let missing = try externalFile().deletingLastPathComponent().appending(path: "missing.step")
        let log = ScopeLog()
        XCTAssertThrowsError(try DocumentInbox.importCopy(of: missing, scope: log.scope()))
        XCTAssertEqual(log.all, ["start missing.step", "stop missing.step copied=false"])
    }

    func testAnUngrantedScopeIsNeverStopped() throws {
        let source = try externalFile()
        _ = try inboxCopy(of: source)
        let log = ScopeLog(granted: false)
        _ = try DocumentInbox.importCopy(of: source, scope: log.scope())
        XCTAssertEqual(log.all, ["start \(source.lastPathComponent)"])
    }

    /// The asynchronous batch the picker and "Open in" paths use: every scope
    /// it starts is stopped, and a failure is reported apart from the paths.
    func testABatchReleasesEveryScopeAndSeparatesFailures() async throws {
        let good = try externalFile()
        _ = try inboxCopy(of: good)
        let missing = good.deletingLastPathComponent().appending(path: "gone.step")
        let log = ScopeLog()
        let result = await DocumentInbox.importCopies(of: [good, missing], scope: log.scope())
        XCTAssertEqual(result.paths.map { URL(fileURLWithPath: $0).lastPathComponent }, [good.lastPathComponent])
        XCTAssertEqual(result.failedNames, ["gone.step"])
        let events = log.all
        XCTAssertEqual(events.filter { $0.hasPrefix("start") }.count, 2)
        XCTAssertEqual(events.filter { $0.hasPrefix("stop") }.count, 2)
    }
    func testInboxIsOnlyASourceAndEditedCopiesAreNeverReplaced() throws {
        let source = try DocumentInbox.inboxDirectory().appending(path: "incoming-\(UUID()).onecad")
        created.append(source)
        try Data("original".utf8).write(to: source)
        let first = try DocumentInbox.importCopy(of: source)
        created.append(first)
        XCTAssertFalse(DocumentInbox.isInInbox(first))
        XCTAssertEqual(try DocumentInbox.importCopy(of: source), first)
        try Data("accepted edit".utf8).write(to: first)
        let second = try DocumentInbox.importCopy(of: source)
        created.append(second)
        XCTAssertNotEqual(first, second)
        XCTAssertEqual(try Data(contentsOf: first), Data("accepted edit".utf8))
        XCTAssertEqual(try Data(contentsOf: source), Data("original".utf8))
    }

    func testInterruptedCopyNeverAdvertisesPartialDestination() throws {
        let source = try externalFile()
        let expected = try inboxCopy(of: source)
        let log = ScopeLog()
        XCTAssertThrowsError(try DocumentInbox.importCopy(of: source, scope: log.scope()) { _, target in
            try Data("partial".utf8).write(to: target)
            throw CocoaError(.fileWriteUnknown)
        })
        XCTAssertFalse(FileManager.default.fileExists(atPath: expected.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
        XCTAssertEqual(log.all.count, 2)
    }

}
