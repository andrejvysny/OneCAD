import UIKit

/// Single-scene host (UIApplicationSupportsMultipleScenes = NO).
final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    private var canvasController: CanvasViewController?
    private var backgroundTask = UIBackgroundTaskIdentifier.invalid
    /// No flush-completion signal exists in ABI v1, so background time is held
    /// for a bounded grace while Rust's service runtime writes the autosave.
    private static let backgroundFlushGrace: TimeInterval = 5

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession,
               options connectionOptions: UIScene.ConnectionOptions) {
        guard let windowScene = scene as? UIWindowScene, let app = AppDelegate.shared else { return }
        let window = UIWindow(windowScene: windowScene)
        self.window = window
        switch app.startBridge() {
        case .success(let bridge):
            let controller = CanvasViewController(bridge: bridge)
            bridge.onTerminal = { [weak self] message in self?.showFatal(.relaunchRequired, message) }
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
    }

    func sceneDidEnterBackground(_ scene: UIScene) {
        guard let controller = canvasController else { return }
        beginBackgroundFlush()
        controller.sceneDidEnterBackground()
    }

    func sceneWillEnterForeground(_ scene: UIScene) {
        endBackgroundFlush()
        canvasController?.sceneWillEnterForeground()
    }

    func sceneDidBecomeActive(_ scene: UIScene) {
        canvasController?.sceneDidBecomeActive()
    }

    private func beginBackgroundFlush() {
        endBackgroundFlush()
        backgroundTask = UIApplication.shared.beginBackgroundTask(withName: "OneCAD autosave flush") {
            [weak self] in
            self?.endBackgroundFlush()
        }
        let task = backgroundTask
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.backgroundFlushGrace) { [weak self] in
            MainActor.assumeIsolated {
                // A later background entry owns a newer task; leave that one alone.
                guard let self, self.backgroundTask == task else { return }
                self.endBackgroundFlush()
            }
        }
    }

    private func endBackgroundFlush() {
        guard backgroundTask != .invalid else { return }
        UIApplication.shared.endBackgroundTask(backgroundTask)
        backgroundTask = .invalid
    }

    // MARK: Files "Open in"

    func scene(_ scene: UIScene, openURLContexts contexts: Set<UIOpenURLContext>) {
        openExternal(contexts)
    }

    /// Copies each file into Documents/Inbox under its security scope, then
    /// hands only the sandbox path to Rust.
    private func openExternal(_ contexts: Set<UIOpenURLContext>) {
        let urls = contexts.map(\.url).filter(\.isFileURL)
        guard !urls.isEmpty else { return }
        Task { [weak self] in
            let paths = await DocumentInbox.importCopies(of: urls)
            guard let controller = self?.canvasController else { return }
            for path in paths { controller.bridge.openPath(path) }
            controller.requestFrame()
        }
    }
}
