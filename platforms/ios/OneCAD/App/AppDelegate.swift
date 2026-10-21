import UIKit

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    private enum BridgeState {
        case notStarted
        case running(RustBridge)
        case failed(String)
        case destroyed
    }

    /// The ABI allows one `onecad_ios_app` per process. It is created on the
    /// first scene connection so a startup failure can be shown in that scene.
    private var state = BridgeState.notStarted

    static var shared: AppDelegate? {
        UIApplication.shared.delegate as? AppDelegate
    }

    func startBridge() -> Result<RustBridge, BridgeFailure> {
        switch state {
        case .running(let bridge): return .success(bridge)
        case .failed(let message): return .failure(BridgeFailure(message: message))
        case .destroyed: return .failure(BridgeFailure(message: "OneCAD has shut down."))
        case .notStarted: break
        }
        do {
            let bridge = try RustBridge.create(paths: try SandboxPaths.resolve())
            state = .running(bridge)
            return .success(bridge)
        } catch {
            let message = String(describing: error)
            state = .failed(message)
            return .failure(BridgeFailure(message: message))
        }
    }

    var bridge: RustBridge? {
        if case .running(let bridge) = state { return bridge }
        return nil
    }

    func applicationDidReceiveMemoryWarning(_ application: UIApplication) {
        bridge?.lifecycle(ONECAD_IOS_MEMORY_WARNING)
    }

    /// Destroy flushes autosave and retires the kernel within its own bounded
    /// grace, well inside the termination budget.
    func applicationWillTerminate(_ application: UIApplication) {
        guard let bridge else { return }
        bridge.lifecycle(ONECAD_IOS_WILL_TERMINATE)
        bridge.destroy()
        state = .destroyed
    }
}

struct BridgeFailure: Error {
    let message: String
}
