import UIKit

/// Native name prompt for SAVE_DOCUMENT. The result is always
/// Documents/<name>.onecad; Rust performs the actual atomic write.
@MainActor
enum SaveNamePrompt {
    static let fileExtension = "onecad"

    static func present(on host: UIViewController, title: String, suggestedName: String,
                        completion: @escaping (String?) -> Void) {
        let alert = UIAlertController(title: title.isEmpty ? "Save Document" : title,
                                      message: nil, preferredStyle: .alert)
        alert.addTextField { field in
            field.text = baseName(suggestedName)
            field.placeholder = "Name"
            field.clearButtonMode = .whileEditing
            field.accessibilityIdentifier = "onecad.save.name"
        }
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in completion(nil) })
        alert.addAction(UIAlertAction(title: "Save", style: .default) { [weak alert] _ in
            let typed = alert?.textFields?.first?.text ?? ""
            guard let url = destination(for: typed) else {
                completion(nil)
                return
            }
            confirmOverwriteIfNeeded(url, on: host, completion: completion)
        })
        host.present(alert, animated: true)
    }

    /// Strips path separators and leading dots so the name cannot escape
    /// Documents; nil for an empty name.
    static func destination(for typed: String) -> URL? {
        var name = baseName(typed)
            .replacingOccurrences(of: "/", with: "-")
            .replacingOccurrences(of: ":", with: "-")
        while name.hasPrefix(".") { name.removeFirst() }
        name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty,
              let documents = try? FileManager.default.url(
                  for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        else { return nil }
        return documents.appending(path: name).appendingPathExtension(fileExtension)
    }

    private static func baseName(_ name: String) -> String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let suffix = "." + fileExtension
        return trimmed.lowercased().hasSuffix(suffix) ? String(trimmed.dropLast(suffix.count)) : trimmed
    }

    private static func confirmOverwriteIfNeeded(_ url: URL, on host: UIViewController,
                                                 completion: @escaping (String?) -> Void) {
        guard FileManager.default.fileExists(atPath: url.path) else {
            completion(url.path)
            return
        }
        let confirm = UIAlertController(
            title: "Replace \u{201C}\(url.lastPathComponent)\u{201D}?",
            message: "A document with this name already exists.", preferredStyle: .alert)
        confirm.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in completion(nil) })
        confirm.addAction(UIAlertAction(title: "Replace", style: .destructive) { _ in completion(url.path) })
        host.present(confirm, animated: true)
    }
}
