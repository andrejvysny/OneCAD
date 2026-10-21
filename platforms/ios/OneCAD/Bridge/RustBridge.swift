import Foundation
import os

let bridgeLog = Logger(subsystem: "com.onecad.ipad", category: "bridge")

/// Receives wake-ups from any Rust thread after they have hopped to main.
@MainActor
protocol FrameWakeTarget: AnyObject {
    func wakeRequested()
}

/// Indirection handed to Rust as `wake_ctx`. Rust may call `wake` from any
/// thread until `onecad_ios_app_destroy` returns, so the box is kept alive by
/// `RustBridge` until after destroy and only holds its target weakly.
@MainActor
final class WakeBox {
    weak var target: FrameWakeTarget?
}

/// `onecad_ios_wake_fn`: runs on any thread, so it only schedules work on
/// main and never calls back into the library (ABI contract).
private func onecadWake(_ ctx: UnsafeMutableRawPointer?) {
    guard let ctx else { return }
    let box = Unmanaged<WakeBox>.fromOpaque(ctx).takeUnretainedValue()
    DispatchQueue.main.async {
        MainActor.assumeIsolated { box.target?.wakeRequested() }
    }
}

enum BridgeError: Error, CustomStringConvertible {
    case abiMismatch(found: UInt32)
    case createFailed(String)

    var description: String {
        switch self {
        case .abiMismatch(let found):
            "OneCAD core ABI \(found) does not match the app (expected \(ONECAD_IOS_ABI_VERSION))."
        case .createFailed(let message):
            message
        }
    }
}

/// Owns the process's single `onecad_ios_app`. Every ABI call except the
/// version query must happen on the creating (main) thread, which the main
/// actor isolation enforces at compile time.
@MainActor
final class RustBridge {
    private(set) var app: OpaquePointer?
    private let wakeBox: Unmanaged<WakeBox>
    /// Set after the first PANIC/TERMINAL status; only destroy is valid then.
    private(set) var terminalMessage: String?
    var onTerminal: ((String) -> Void)?

    private init(app: OpaquePointer, wakeBox: Unmanaged<WakeBox>) {
        self.app = app
        self.wakeBox = wakeBox
    }

    var wakeTarget: FrameWakeTarget? {
        get { wakeBox.takeUnretainedValue().target }
        set { wakeBox.takeUnretainedValue().target = newValue }
    }

    static func create(paths: SandboxPaths) throws -> RustBridge {
        let abi = onecad_ios_abi_version()
        guard abi == ONECAD_IOS_ABI_VERSION else { throw BridgeError.abiMismatch(found: abi) }
        let box = Unmanaged.passRetained(WakeBox())
        var out: OpaquePointer?
        let status = paths.withCStrings { docs, support, caches, tmp in
            var config = onecad_ios_config()
            config.struct_size = UInt32(MemoryLayout<onecad_ios_config>.size)
            config.documents_dir = docs
            config.app_support_dir = support
            config.caches_dir = caches
            config.tmp_dir = tmp
            config.wake = onecadWake
            config.wake_ctx = box.toOpaque()
            return onecad_ios_app_create(&config, &out)
        }
        guard status == ONECAD_IOS_OK, let out else {
            box.release()
            // No app exists, so there is no per-app error string to read.
            throw BridgeError.createFailed("OneCAD could not start (status \(status.rawValue)).")
        }
        return RustBridge(app: out, wakeBox: box)
    }

    /// Bounded by the library's destroy grace. No wake callback runs after
    /// it returns, which is what makes releasing the box safe.
    func destroy() {
        guard let app else { return }
        let status = onecad_ios_app_destroy(app)
        if status != ONECAD_IOS_OK {
            bridgeLog.error("destroy returned \(status.rawValue)")
        }
        self.app = nil
        wakeBox.release()
    }

    var lastError: String {
        guard let app else { return "" }
        return String(cString: onecad_ios_last_error(app))
    }

    var isUsable: Bool { app != nil && terminalMessage == nil }

    /// Logs failures and latches the terminal state on PANIC/TERMINAL.
    @discardableResult
    func checked(_ status: onecad_ios_status, _ call: StaticString) -> onecad_ios_status {
        guard status != ONECAD_IOS_OK else { return status }
        let message = lastError
        bridgeLog.error("\(call, privacy: .public) -> \(status.rawValue): \(message, privacy: .public)")
        if status == ONECAD_IOS_ERR_PANIC || status == ONECAD_IOS_ERR_TERMINAL, terminalMessage == nil {
            let text = message.isEmpty ? "OneCAD stopped and must be relaunched." : message
            terminalMessage = text
            onTerminal?(text)
        }
        return status
    }

    /// Runs `body` with the live handle, or reports ERR_STATE without calling Rust.
    func call(_ name: StaticString, _ body: (OpaquePointer) -> onecad_ios_status) -> onecad_ios_status {
        guard let app, terminalMessage == nil else { return ONECAD_IOS_ERR_STATE }
        return checked(body(app), name)
    }
}

/// Sandbox directories passed to `onecad_ios_app_create`.
struct SandboxPaths: Sendable {
    let documents: URL
    let appSupport: URL
    let caches: URL
    let tmp: URL

    static func resolve() throws -> SandboxPaths {
        let fm = FileManager.default
        let support = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask,
                                 appropriateFor: nil, create: true)
        return SandboxPaths(
            documents: try fm.url(for: .documentDirectory, in: .userDomainMask,
                                  appropriateFor: nil, create: true),
            appSupport: support,
            caches: try fm.url(for: .cachesDirectory, in: .userDomainMask,
                               appropriateFor: nil, create: true),
            tmp: fm.temporaryDirectory)
    }

    func withCStrings<R>(
        _ body: (UnsafePointer<CChar>, UnsafePointer<CChar>, UnsafePointer<CChar>, UnsafePointer<CChar>) -> R
    ) -> R {
        documents.path.withCString { docs in
            appSupport.path.withCString { support in
                caches.path.withCString { caches in
                    tmp.path.withCString { tmp in body(docs, support, caches, tmp) }
                }
            }
        }
    }
}
