import UIKit

/// Native name prompt for SAVE_DOCUMENT. Rust selects the format and performs
/// the atomic write; the prompt confines the selected name to Documents.
@MainActor
enum SaveNamePrompt {
    static let supportedExtensions: Set<String> = ["onecad", "step", "stl", "obj", "3mf"]

    static func present(on host: UIViewController, title: String, suggestedName: String,
                        fileExtension: String, completion: @escaping (String?) -> Void) {
        guard supportedExtensions.contains(fileExtension) else {
            completion(nil)
            return
        }
        let alert = UIAlertController(title: title.isEmpty ? "Save Document" : title,
                                      message: nil, preferredStyle: .alert)
        alert.addTextField { field in
            field.text = baseName(suggestedName, fileExtension: fileExtension)
            field.placeholder = "Name"
            field.clearButtonMode = .whileEditing
            field.accessibilityIdentifier = "onecad.save.name"
        }
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in completion(nil) })
        alert.addAction(UIAlertAction(title: "Save", style: .default) { [weak alert] _ in
            let typed = alert?.textFields?.first?.text ?? ""
            guard let url = destination(for: typed, fileExtension: fileExtension) else {
                completion(nil)
                return
            }
            confirmOverwriteIfNeeded(url, on: host, completion: completion)
        })
        host.present(alert, animated: true)
    }

    /// Strips path separators and leading dots so the name cannot escape
    /// Documents; nil for an empty name.
    static func destination(for typed: String, fileExtension: String) -> URL? {
        guard supportedExtensions.contains(fileExtension),
              !typed.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains)
        else { return nil }
        var name = baseName(typed, fileExtension: fileExtension)
            .replacingOccurrences(of: "/", with: "-")
            .replacingOccurrences(of: ":", with: "-")
            .replacingOccurrences(of: "\\", with: "-")
        while name.hasPrefix(".") {
            name.removeFirst()
            name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty,
              let documents = try? FileManager.default.url(
                  for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        else { return nil }
        return documents.appending(path: name).appendingPathExtension(fileExtension)
    }

    static func baseName(_ name: String, fileExtension: String) -> String {
        var trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let suffix = "." + fileExtension
        while trimmed.lowercased().hasSuffix(suffix) {
            trimmed = String(trimmed.dropLast(suffix.count)).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return trimmed
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
