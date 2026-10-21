import UIKit
import UniformTypeIdentifiers

/// Drains Rust's host-request queue and serves requests one at a time with
/// native UI. Every request is completed exactly once; an empty path list
/// means cancelled (or "done" for requests that produce no file).
@MainActor
final class HostRequestCoordinator: NSObject, UIDocumentPickerDelegate {
    static let documentType = UTType(exportedAs: "com.onecad.document")
    /// No system STEP type exists and `public.*` is reserved for Apple, so
    /// Info.plist imports `org.iso.step` (ISO 10303-21) for .step/.stp.
    static let stepType = UTType(importedAs: "org.iso.step")

    private let bridge: RustBridge
    private weak var presenter: UIViewController?
    private var pending: [HostRequest] = []
    private var active: HostRequest?
    var onCompleted: (() -> Void)?

    init(bridge: RustBridge, presenter: UIViewController) {
        self.bridge = bridge
        self.presenter = presenter
    }

    /// Called when a frame reports HOST_REQUESTS. Requests are copied out
    /// immediately because Rust's pointers die on the next pop.
    func drain() {
        while let request = bridge.nextHostRequest() { pending.append(request) }
        serveNext()
    }

    private func serveNext() {
        guard active == nil, !pending.isEmpty else { return }
        let request = pending.removeFirst()
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
        guard let host = presentingController else {
            complete(paths: [])
            return
        }
        picker.delegate = self
        host.present(picker, animated: true)
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard let request = active else { return }
        if request.kind == ONECAD_IOS_REQ_EXPORT_FILE.rawValue {
            // Success reports the sandbox file that was published.
            complete(paths: [request.path])
            return
        }
        Task { [weak self] in
            let paths = await DocumentInbox.importCopies(of: urls)
            self?.complete(paths: paths)
        }
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        complete(paths: [])
    }

    // MARK: Save

    private func presentSavePrompt(_ request: HostRequest) {
        guard let host = presentingController else {
            complete(paths: [])
            return
        }
        SaveNamePrompt.present(on: host, title: request.label, suggestedName: request.suggestedName) {
            [weak self] path in
            self?.complete(paths: path.map { [$0] } ?? [])
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
