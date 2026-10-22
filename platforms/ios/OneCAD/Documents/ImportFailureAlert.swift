import UIKit

/// Native report for an import whose copy into Documents/Inbox failed, so a
/// failure is not mistaken for a cancel (the ABI completion cannot carry it).
@MainActor
enum ImportFailureAlert {
    static let accessibilityID = "onecad.import.failed"

    static func make(failedNames: [String], onDismiss: @escaping () -> Void) -> UIAlertController {
        let names = failedNames.map { "\u{201C}\($0)\u{201D}" }.joined(separator: ", ")
        let alert = UIAlertController(
            title: failedNames.count == 1 ? "Couldn\u{2019}t Import File" : "Couldn\u{2019}t Import Files",
            message: "\(names) could not be copied into OneCAD.", preferredStyle: .alert)
        alert.view.accessibilityIdentifier = accessibilityID
        alert.addAction(UIAlertAction(title: "OK", style: .default) { _ in onDismiss() })
        return alert
    }
}
