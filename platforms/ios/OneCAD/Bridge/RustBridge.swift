import Foundation
import QuartzCore
import Synchronization
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
    /// Set while a main-queue hop is outstanding, so a wake storm from Rust
    /// threads queues at most one block instead of one per wake.
    nonisolated let hopPending = Atomic<Bool>(false)
}

/// `onecad_ios_wake_fn`: runs on any thread, so it only schedules work on
/// main and never calls back into the library (ABI contract).
private func onecadWake(_ ctx: UnsafeMutableRawPointer?) {
    guard let ctx else { return }
    let box = Unmanaged<WakeBox>.fromOpaque(ctx).takeUnretainedValue()
    guard !box.hopPending.exchange(true, ordering: .acquiringAndReleasing) else { return }
    DispatchQueue.main.async {
        MainActor.assumeIsolated {
            // Cleared before notifying so a wake during the frame hops again.
            box.hopPending.store(false, ordering: .releasing)
            box.target?.wakeRequested()
        }
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
    /// Strong reference for the header's "retained by the host while
    /// attached": kept until detach succeeds or destroy returns, so tearing
    /// down the canvas (e.g. for the fatal view) cannot free a layer Rust
    /// still holds.
    private var attachedLayer: CAMetalLayer?
    /// Set after the first PANIC/TERMINAL status. Only teardown and lifecycle
    /// calls still reach Rust then (see `call(_:allowWhenTerminal:_:)`).
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
    /// it returns OK, which is what makes releasing the box and layer safe.
    func destroy() {
        guard let app else { return }
        let status = onecad_ios_app_destroy(app)
        self.app = nil
        guard status == ONECAD_IOS_OK else {
            // Producers or the surface may still be live: leak both on purpose
            // rather than risk a callback or present into freed memory.
            bridgeLog.error("destroy returned \(status.rawValue); leaking wake box and layer")
            if let attachedLayer { _ = Unmanaged.passRetained(attachedLayer) }
            return
        }
        attachedLayer = nil
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
            // Deferred: the failing call may be inside layoutSubviews or a touch
            // flush, which must not tear down the root view controller inline.
            DispatchQueue.main.async { [weak self] in
                MainActor.assumeIsolated { self?.onTerminal?(text) }
            }
        }
        return status
    }

    /// Runs `body` with the live handle, or reports ERR_STATE without calling
    /// Rust. After the terminal latch only `allowWhenTerminal` calls go through:
    /// ERR_TERMINAL may mean "kernel relaunch required" with autosave still
    /// working, and Rust itself rejects calls on a poisoned app.
    func call(_ name: StaticString, allowWhenTerminal: Bool = false,
              _ body: (OpaquePointer) -> onecad_ios_status) -> onecad_ios_status {
        guard let app, terminalMessage == nil || allowWhenTerminal else { return ONECAD_IOS_ERR_STATE }
        return checked(body(app), name)
    }

    // MARK: Surface

    @discardableResult
    func attachSurface(_ desc: onecad_ios_surface_desc, layer: CAMetalLayer) -> onecad_ios_status {
        var desc = desc
        desc.ca_metal_layer = Unmanaged.passUnretained(layer).toOpaque()
        let status = call("surface_attach") { onecad_ios_surface_attach($0, &desc) }
        if status == ONECAD_IOS_OK { attachedLayer = layer }
        return status
    }

    /// Attempted even after the terminal latch; on failure the layer stays
    /// retained until destroy releases the surface.
    func detachSurface() {
        guard attachedLayer != nil else { return }
        let status = call("surface_detach", allowWhenTerminal: true) { onecad_ios_surface_detach($0) }
        if status == ONECAD_IOS_OK { attachedLayer = nil }
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
