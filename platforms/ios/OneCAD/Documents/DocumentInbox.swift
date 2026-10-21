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

    /// Copies each URL into the inbox and returns the sandbox paths that
    /// succeeded. Failures are logged and skipped, never partially written.
    static func importCopies(of urls: [URL]) async -> [String] {
        await Task.detached(priority: .userInitiated) {
            urls.compactMap { url -> String? in
                do {
                    return try importCopy(of: url).path
                } catch {
                    inboxLog.error("import of \(url.lastPathComponent, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
                    return nil
                }
            }
        }.value
    }

    /// Security scope is held only around the coordinated read; the copy
    /// itself owns no external resource afterwards.
    static func importCopy(of source: URL) throws -> URL {
        if isInInbox(source) { return source }
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        let destination = try uniqueDestination(for: source.lastPathComponent)
        var coordinationError: NSError?
        var copyError: Error?
        NSFileCoordinator(filePresenter: nil).coordinate(
            readingItemAt: source, options: [.withoutChanges], error: &coordinationError
        ) { readable in
            do {
                try FileManager.default.copyItem(at: readable, to: destination)
            } catch {
                copyError = error
            }
        }
        if let error = coordinationError ?? copyError { throw error }
        return destination
    }

    private static func uniqueDestination(for fileName: String) throws -> URL {
        let inbox = try inboxDirectory()
        let base = (fileName as NSString).deletingPathExtension
        let ext = (fileName as NSString).pathExtension
        var candidate = inbox.appending(path: fileName)
        var counter = 2
        while FileManager.default.fileExists(atPath: candidate.path) {
            let name = ext.isEmpty ? "\(base) \(counter)" : "\(base) \(counter).\(ext)"
            candidate = inbox.appending(path: name)
            counter += 1
        }
        return candidate
    }
}
