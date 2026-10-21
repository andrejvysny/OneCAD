import CoreGraphics
import Foundation

/// Host request copied out of Rust-owned memory (valid only until the next
/// `host_request_next`).
struct HostRequest: Sendable {
    let id: UInt64
    let kind: UInt32
    let multiple: Bool
    let label: String
    let extensions: [String]
    let path: String
    let suggestedName: String
    let text: String
}

/// Accessibility node copied out of the Rust snapshot.
struct AccessibilityNode: Sendable {
    let id: UInt64
    let role: UInt32
    let flags: UInt32
    let frame: CGRect
    let identifier: String
    let label: String
    let value: String
}

private func copyString(_ pointer: UnsafePointer<CChar>?) -> String {
    pointer.map { String(cString: $0) } ?? ""
}

extension RustBridge {
    // MARK: Host requests

    /// Pops one request; nil when the queue is empty (ABI returns ERR_STATE).
    func nextHostRequest() -> HostRequest? {
        guard let app, terminalMessage == nil else { return nil }
        var raw = onecad_ios_host_request()
        let status = onecad_ios_host_request_next(app, &raw)
        if status == ONECAD_IOS_ERR_STATE { return nil }
        guard checked(status, "host_request_next") == ONECAD_IOS_OK else { return nil }
        let extensions = copyString(raw.extensions)
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespaces).lowercased() }
            .filter { !$0.isEmpty }
        return HostRequest(
            id: raw.id, kind: raw.kind, multiple: raw.multiple != 0,
            label: copyString(raw.label), extensions: extensions,
            path: copyString(raw.path), suggestedName: copyString(raw.suggested_name),
            text: copyString(raw.text))
    }

    /// Empty `paths` reports cancellation (ABI contract).
    func completeHostRequest(_ id: UInt64, paths: [String]) {
        let owned = paths.map { strdup($0) }
        defer { owned.forEach { free($0) } }
        let pointers: [UnsafePointer<CChar>?] = owned.map { $0.map { UnsafePointer($0) } }
        _ = call("host_request_complete") { app in
            pointers.withUnsafeBufferPointer { buffer in
                onecad_ios_host_request_complete(app, id, buffer.baseAddress, UInt32(buffer.count))
            }
        }
    }

    // MARK: Accessibility

    func accessibilitySnapshot() -> (nodes: [AccessibilityNode], generation: UInt64)? {
        var nodes: UnsafePointer<onecad_ios_a11y_node>?
        var count: UInt32 = 0
        var generation: UInt64 = 0
        let status = call("a11y_snapshot") { onecad_ios_a11y_snapshot($0, &nodes, &count, &generation) }
        guard status == ONECAD_IOS_OK else { return nil }
        guard let nodes, count > 0 else { return ([], generation) }
        let copied = UnsafeBufferPointer(start: nodes, count: Int(count)).map { node in
            AccessibilityNode(
                id: node.id, role: node.role, flags: node.flags,
                frame: CGRect(x: CGFloat(node.x), y: CGFloat(node.y),
                              width: CGFloat(node.w), height: CGFloat(node.h)),
                identifier: copyString(node.identifier), label: copyString(node.label),
                value: copyString(node.value))
        }
        return (copied, generation)
    }

    func activateAccessibility(_ id: UInt64) -> Bool {
        call("a11y_activate") { onecad_ios_a11y_activate($0, id) } == ONECAD_IOS_OK
    }
}
