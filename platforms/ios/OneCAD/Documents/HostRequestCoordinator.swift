import UIKit
import UniformTypeIdentifiers

/// Serves Rust's host requests one at a time with native UI, popping the next
/// one only when the previous is completed, so the backlog stays in Rust's
/// bounded (64) queue rather than an unbounded host copy. Every request is
/// completed exactly once; an empty path list means cancelled (or "done" for
/// requests that produce no file).
@MainActor
final class HostRequestCoordinator: NSObject, UIDocumentPickerDelegate {
    static let documentType = UTType(exportedAs: "com.onecad.document")
    /// No system STEP type exists and `public.*` is reserved for Apple, so
    /// Info.plist imports `org.iso.step` (ISO 10303-21) for .step/.stp.
    static let stepType = UTType(importedAs: "org.iso.step")

    private let bridge: RustBridge
    private weak var presenter: UIViewController?
    private var active: HostRequest?
    var onCompleted: (() -> Void)?

    init(bridge: RustBridge, presenter: UIViewController) {
        self.bridge = bridge
        self.presenter = presenter
    }

    /// Called when a frame reports HOST_REQUESTS. Popped requests are copied
    /// out (HostRequest) because Rust's pointers die on the next pop.
    func drain() {
        serveNext()
    }

    private func serveNext() {
        guard active == nil, let request = bridge.nextHostRequest() else { return }
        active = request
        switch request.kind {
        case ONECAD_IOS_REQ_OPEN_DOCUMENT.rawValue: presentOpenPicker(request)
        case ONECAD_IOS_REQ_SAVE_DOCUMENT.rawValue: presentSavePrompt(request)
        case ONECAD_IOS_REQ_EXPORT_FILE.rawValue: presentExportPicker(request)
        case ONECAD_IOS_REQ_HAPTIC.rawValue: playHaptic(request)
        case ONECAD_IOS_REQ_COPY_TEXT.rawValue: copyText(request)
        case ONECAD_IOS_REQ_OPEN_URL.rawValue: openURL(request)
        default:
            bridgeLog.error("unknown host request kind \(request.kind)")
            complete(paths: [])
        }
    }

    func complete(paths: [String]) {
        guard let request = active else { return }
        active = nil
        bridge.completeHostRequest(request.id, paths: paths)
        onCompleted?()
        serveNext()
    }

    private var presentingController: UIViewController? {
        var top = presenter
        while let next = top?.presentedViewController { top = next }
        return top
    }

    // MARK: Open / export

    static func contentTypes(for extensions: [String]) -> [UTType] {
        let types = extensions.map { ext -> UTType in
            switch ext {
            case "onecad": return documentType
            case "step", "stp": return stepType
            default: return UTType(filenameExtension: ext) ?? .data
            }
        }
        return types.isEmpty ? [documentType] : types
    }

    private func presentOpenPicker(_ request: HostRequest) {
        let picker = UIDocumentPickerViewController(
            forOpeningContentTypes: Self.contentTypes(for: request.extensions), asCopy: true)
        picker.allowsMultipleSelection = request.multiple
        present(picker)
    }

    private func presentExportPicker(_ request: HostRequest) {
        guard !request.path.isEmpty, FileManager.default.fileExists(atPath: request.path) else {
            bridgeLog.error("export request \(request.id) names no existing file")
            complete(paths: [])
            return
        }
        let picker = UIDocumentPickerViewController(
            forExporting: [URL(fileURLWithPath: request.path)], asCopy: true)
        present(picker)
    }

    private func present(_ picker: UIDocumentPickerViewController) {
        picker.delegate = self
        withReadyPresenter { $0.present(picker, animated: true) }
    }

    /// UIKit silently refuses to present from a controller that is off window
    /// or mid-transition, which would leave the request active forever and
    /// block every later one (haptics, copy). A running transition is waited
    /// out; no on-window presenter completes the request as cancelled.
    /// (Refusal cannot be detected after the fact: the document picker's
    /// presentation is delayed, so `presentingViewController` stays nil for a
    /// while even when it succeeds.)
    private func withReadyPresenter(_ body: @escaping (UIViewController) -> Void) {
        guard let host = presentingController, host.viewIfLoaded?.window != nil else {
            bridgeLog.error("no on-window presenter for host request; completing as cancelled")
            complete(paths: [])
            return
        }
        if let coordinator = host.transitionCoordinator,
           coordinator.animate(alongsideTransition: nil, completion: { [weak self] _ in
               self?.withReadyPresenter(body)
           }) {
            return
        }
        body(host)
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard let request = active else { return }
        if request.kind == ONECAD_IOS_REQ_EXPORT_FILE.rawValue {
            // Success reports the sandbox file that was published.
            complete(paths: [request.path])
            return
        }
        Task { [weak self] in
            let result = await DocumentInbox.importCopies(of: urls)
            self?.finishImport(result)
        }
    }

    /// The ABI has no failure completion (empty = cancelled), so a failed copy
    /// is told apart natively: the alert is shown first and the request is
    /// completed when it is dismissed, keeping requests strictly serial.
    private func finishImport(_ result: DocumentInbox.Result) {
        guard !result.failedNames.isEmpty else {
            complete(paths: result.paths)
            return
        }
        let alert = ImportFailureAlert.make(failedNames: result.failedNames) { [weak self] in
            self?.complete(paths: result.paths)
        }
        // If no presenter is ready this completes as cancelled, which also
        // drops the successful paths; the failure is still logged.
        withReadyPresenter { $0.present(alert, animated: true) }
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        complete(paths: [])
    }

    // MARK: Save

    private func presentSavePrompt(_ request: HostRequest) {
        withReadyPresenter { [weak self] host in
            SaveNamePrompt.present(on: host, title: request.label, suggestedName: request.suggestedName) { path in
                self?.complete(paths: path.map { [$0] } ?? [])
            }
        }
    }

    // MARK: Fire-and-forget requests

    private func playHaptic(_ request: HostRequest) {
        let style: UIImpactFeedbackGenerator.FeedbackStyle = switch request.text {
        case "light": .light
        case "heavy": .heavy
        case "soft": .soft
        case "rigid": .rigid
        default: .medium
        }
        if let view = presenter?.view {
            UIImpactFeedbackGenerator(style: style, view: view).impactOccurred()
        }
        complete(paths: [])
    }

    private func copyText(_ request: HostRequest) {
        UIPasteboard.general.string = request.text
        complete(paths: [])
    }

    /// Only web links leave the app; other schemes are refused.
    private func openURL(_ request: HostRequest) {
        if let url = URL(string: request.text), ["http", "https"].contains(url.scheme?.lowercased()) {
            UIApplication.shared.open(url)
        } else {
            bridgeLog.error("refused to open URL with unsupported scheme")
        }
        complete(paths: [])
    }
}
