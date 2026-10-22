import Foundation
import os

private let inboxLog = Logger(subsystem: "com.onecad.ipad", category: "documents")

/// Managed-copy policy (LSSupportsOpeningDocumentsInPlace = NO): every
/// external file is copied into Documents/Inbox under file coordination and
/// Rust only ever sees sandbox paths. Runs off the main thread.
enum DocumentInbox {
    static func inboxDirectory() throws -> URL {
        let documents = try FileManager.default.url(
            for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let inbox = documents.appending(path: "Inbox", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
        return inbox
    }

    /// The system already copies "Open in" files into Documents/Inbox when
    /// in-place opening is off; those are used as they are.
    static func isInInbox(_ url: URL) -> Bool {
        guard let inbox = try? inboxDirectory() else { return false }
        let parent = url.resolvingSymlinksInPath().deletingLastPathComponent().standardizedFileURL
        return parent.path == inbox.resolvingSymlinksInPath().standardizedFileURL.path
    }

    /// Sandbox paths that were imported, and the file names that failed.
    /// Failures are kept apart so the host can tell them from a cancel.
    struct Result: Sendable {
        var paths: [String] = []
        var failedNames: [String] = []
    }

    /// Copies each URL into the inbox. Failures are logged and never leave a
    /// partially written file behind.
    static func importCopies(of urls: [URL], scope: SecurityScope = SecurityScope()) async -> Result {
        await Task.detached(priority: .userInitiated) {
            var result = Result()
            for url in urls {
                do {
                    result.paths.append(try importCopy(of: url, scope: scope).path)
                } catch {
                    inboxLog.error("import of \(url.lastPathComponent, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
                    result.failedNames.append(url.lastPathComponent)
                }
            }
            return result
        }.value
    }

    /// The security scope is held for the whole coordinated read and copy
    /// (which runs synchronously inside it) and released on every path; the
    /// copy owns no external resource afterwards.
    static func importCopy(of source: URL, scope: SecurityScope = SecurityScope()) throws -> URL {
        if isInInbox(source) { return source }
        return try scope.withAccess(to: source) {
            let inbox = try inboxDirectory()
            var coordinationError: NSError?
            var outcome: Swift.Result<URL, Error> = .failure(CocoaError(.fileReadUnknown))
            NSFileCoordinator(filePresenter: nil).coordinate(
                readingItemAt: source, options: [.withoutChanges], error: &coordinationError
            ) { readable in
                outcome = Swift.Result { try copyOrReuse(readable, named: source.lastPathComponent, in: inbox) }
            }
            if let coordinationError { throw coordinationError }
            return try outcome.get()
        }
    }

    /// Reopening the same file reuses an identical earlier copy ("name",
    /// "name 2", ...) instead of adding another, so repeated imports of one
    /// file do not grow the inbox. A changed file gets the next free name.
    private static func copyOrReuse(_ readable: URL, named fileName: String, in inbox: URL) throws -> URL {
        let fm = FileManager.default
        let base = (fileName as NSString).deletingPathExtension
        let ext = (fileName as NSString).pathExtension
        var candidate = inbox.appending(path: fileName)
        var counter = 2
        while fm.fileExists(atPath: candidate.path) {
            if fm.contentsEqual(atPath: readable.path, andPath: candidate.path) { return candidate }
            let name = ext.isEmpty ? "\(base) \(counter)" : "\(base) \(counter).\(ext)"
            candidate = inbox.appending(path: name)
            counter += 1
        }
        try fm.copyItem(at: readable, to: candidate)
        return candidate
    }
}

/// A file's security-scoped access, held for exactly the work passed to
/// `withAccess` and released when it returns or throws. The two calls are
/// injectable so tests can prove the pairing: URLs on the managed-copy paths
/// (picker copies, system Inbox files) usually carry no scope to observe.
struct SecurityScope: Sendable {
    var start: @Sendable (URL) -> Bool = { $0.startAccessingSecurityScopedResource() }
    var stop: @Sendable (URL) -> Void = { $0.stopAccessingSecurityScopedResource() }

    func withAccess<T>(to url: URL, _ body: () throws -> T) throws -> T {
        let scoped = start(url)
        defer { if scoped { stop(url) } }
        return try body()
    }
}
