import UIKit

/// Single-scene host (UIApplicationSupportsMultipleScenes = NO).
final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    private var canvasController: CanvasViewController?
    /// Kept after the canvas is replaced by the fatal view: lifecycle events
    /// (autosave flush on background) must still reach Rust then.
    private var bridge: RustBridge?
    private var backgroundTask = UIBackgroundTaskIdentifier.invalid
    private var backgroundToken: UInt64 = 0
    private var backgroundGeneration: UInt64 = 0
    private var recoveryPoll: DispatchSourceTimer?

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession,
               options connectionOptions: UIScene.ConnectionOptions) {
        guard let windowScene = scene as? UIWindowScene, let app = AppDelegate.shared else { return }
        let window = UIWindow(windowScene: windowScene)
        self.window = window
        switch app.startBridge() {
        case .success(let bridge):
            let controller = CanvasViewController(bridge: bridge)
            bridge.onTerminal = { [weak self] message in self?.showFatal(.relaunchRequired, message) }
            self.bridge = bridge
            canvasController = controller
            window.rootViewController = controller
        case .failure(let failure):
            window.rootViewController = FatalErrorViewController(kind: .startupFailed, message: failure.message)
        }
        window.makeKeyAndVisible()
        openExternal(connectionOptions.urlContexts)
    }

    func sceneDidDisconnect(_ scene: UIScene) {
        canvasController?.shutdown()
        canvasController = nil
    }

    private func showFatal(_ kind: FatalErrorViewController.Kind, _ message: String) {
        canvasController?.shutdown()
        canvasController = nil
        window?.rootViewController = FatalErrorViewController(kind: kind, message: message)
    }

    // MARK: Lifecycle

    func sceneWillResignActive(_ scene: UIScene) {
        canvasController?.sceneWillResignActive()
        bridge?.lifecycle(ONECAD_IOS_WILL_RESIGN_ACTIVE)
    }

    func sceneDidEnterBackground(_ scene: UIScene) {
        guard let bridge else { return }
        canvasController?.sceneDidEnterBackground()
        beginBackgroundFlush()
        bridge.lifecycle(ONECAD_IOS_DID_ENTER_BACKGROUND)
        observeBackgroundFlush()
    }

    func sceneWillEnterForeground(_ scene: UIScene) {
        endBackgroundFlush(cancel: true)
        bridge?.lifecycle(ONECAD_IOS_WILL_ENTER_FOREGROUND)
        canvasController?.sceneWillEnterForeground()
    }

    func sceneDidBecomeActive(_ scene: UIScene) {
        bridge?.lifecycle(ONECAD_IOS_DID_BECOME_ACTIVE)
        canvasController?.sceneDidBecomeActive()
    }

    private func beginBackgroundFlush() {
        endBackgroundFlush(cancel: true)
        backgroundToken &+= 1
        let token = backgroundToken
        backgroundTask = UIApplication.shared.beginBackgroundTask(withName: "OneCAD autosave flush") {
            [weak self] in
            guard let self, self.backgroundToken == token else { return }
            self.endBackgroundFlush(expired: true)
        }
    }

    private func observeBackgroundFlush() {
        guard let bridge else { return }
        let (status, snapshot, _) = bridge.recoverySnapshot()
        guard status == ONECAD_IOS_OK else { endBackgroundFlush(cancel: true); return }
        backgroundGeneration = snapshot.generation
        guard backgroundTask != .invalid else {
            bridge.finishRecoveryWaiting(generation: backgroundGeneration, expired: true)
            return
        }
        let token = backgroundToken
        let timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(deadline: .now(), repeating: .milliseconds(50))
        timer.setEventHandler { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.backgroundToken == token else { return }
                self.pollBackgroundFlush()
            }
        }
        recoveryPoll = timer
        timer.resume()
    }

    private func pollBackgroundFlush() {
        guard let bridge else { endBackgroundFlush(cancel: true); return }
        let (status, snapshot, documents) = bridge.recoverySnapshot()
        guard status == ONECAD_IOS_OK else { endBackgroundFlush(cancel: true); return }
        guard snapshot.generation == backgroundGeneration else { endBackgroundFlush(cancel: true); return }
        guard snapshot.status != 0 else { return }
        bridgeLog.info("Background recovery completed: \(documents, privacy: .private)")
        endBackgroundFlush()
    }

    private func endBackgroundFlush(expired: Bool = false, cancel: Bool = false) {
        recoveryPoll?.cancel()
        recoveryPoll = nil
        if backgroundGeneration != 0 && (expired || cancel) {
            bridge?.finishRecoveryWaiting(generation: backgroundGeneration, expired: expired)
        }
        backgroundGeneration = 0
        guard backgroundTask != .invalid else { return }
        UIApplication.shared.endBackgroundTask(backgroundTask)
        backgroundTask = .invalid
    }

    // MARK: Files "Open in"

    func scene(_ scene: UIScene, openURLContexts contexts: Set<UIOpenURLContext>) {
        openExternal(contexts)
    }

    /// Copies each file into Documents/Imported under its security scope, then
    /// hands only the sandbox path to Rust.
    private func openExternal(_ contexts: Set<UIOpenURLContext>) {
        let urls = contexts.map(\.url).filter(\.isFileURL)
        guard !urls.isEmpty else { return }
        Task { [weak self] in
            let result = await DocumentInbox.importCopies(of: urls)
            guard let self, let controller = self.canvasController else { return }
            for path in result.paths { controller.bridge.openPath(path) }
            controller.requestFrame()
            if !result.failedNames.isEmpty { self.reportImportFailure(result.failedNames) }
        }
    }

    private func reportImportFailure(_ names: [String]) {
        var top = window?.rootViewController
        while let next = top?.presentedViewController { top = next }
        top?.present(ImportFailureAlert.make(failedNames: names) {}, animated: true)
    }
}
