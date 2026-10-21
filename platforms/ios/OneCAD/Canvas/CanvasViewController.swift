import QuartzCore
import UIKit

/// Root controller of the single scene: owns the canvas view and wires every
/// native input/output channel to the Rust bridge. Holds no CAD state.
@MainActor
final class CanvasViewController: UIViewController {
    let bridge: RustBridge
    private let canvas = MetalCanvasView(frame: .zero)
    private let touchInput = TouchInputController()
    private var frameLoop: FrameLoop?
    private var capabilities: CapabilityTracker?
    private var navigation: NavigationGestures?
    private var pencil: PencilController?
    private var textInput: TextInputController?
    private var accessibility: AccessibilityBridge?
    private var hostRequests: HostRequestCoordinator?
    private var surfaceAttached = false
    private var reportedKernelTerminal = false

    init(bridge: RustBridge) {
        self.bridge = bridge
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func loadView() {
        view = canvas
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        frameLoop = FrameLoop { [weak self] in self?.runFrame() }
        canvas.surfaceDelegate = self
        canvas.inputSink = self
        touchInput.delivery = self
        bridge.wakeTarget = self
        wireInputChannels()
        wireOutputChannels()
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (controller: CanvasViewController, _) in
            controller.sendInterfaceStyle()
        }
    }

    private func wireInputChannels() {
        let bridge = bridge
        let caps = CapabilityTracker { bridge.setCapabilities($0) }
        capabilities = caps
        navigation = NavigationGestures(view: canvas) { [weak self] nav in
            self?.bridge.submitNavigation(nav)
            self?.requestFrame()
        }
        pencil = PencilController(view: canvas, capabilities: caps) { [weak self] event in
            self?.bridge.submitPencilEvent(event)
            self?.requestFrame()
        }
        let hover = UIHoverGestureRecognizer(target: self, action: #selector(hoverChanged(_:)))
        hover.cancelsTouchesInView = false
        canvas.addGestureRecognizer(hover)
    }

    private func wireOutputChannels() {
        let text = TextInputController(bridge: bridge, canvas: canvas)
        text.onInput = { [weak self] in self?.requestFrame() }
        text.onCapabilities = { [weak self] in self?.capabilities?.observe($0) }
        textInput = text
        let a11y = AccessibilityBridge(container: canvas, bridge: bridge)
        a11y.onActivate = { [weak self] in self?.requestFrame() }
        accessibility = a11y
        let requests = HostRequestCoordinator(bridge: bridge, presenter: self)
        requests.onCompleted = { [weak self] in
            self?.restoreKeyFocus()
            self?.requestFrame()
        }
        hostRequests = requests
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        canvas.becomeFirstResponder()
        frameLoop?.start()
        sendInterfaceStyle()
        capabilities?.publish()
        requestFrame()
    }

    override func viewWillTransition(to size: CGSize,
                                     with coordinator: UIViewControllerTransitionCoordinator) {
        super.viewWillTransition(to: size, with: coordinator)
        // Contacts captured in the old geometry cannot continue in the new one.
        cancelContacts(reason: ONECAD_IOS_CANCEL_ORIENTATION)
        coordinator.animate(alongsideTransition: nil) { [weak self] _ in
            self?.accessibility?.refreshFrames()
            self?.requestFrame()
        }
    }

    @objc private func hoverChanged(_ recognizer: UIHoverGestureRecognizer) {
        touchInput.hover(recognizer, in: canvas)
    }

    func requestFrame() {
        frameLoop?.requestFrame()
    }

    /// Pickers and alerts take first responder; hardware keys must reach the
    /// canvas again afterwards unless a text field owns the keyboard.
    private func restoreKeyFocus() {
        guard textInput?.isEditing != true, !canvas.isFirstResponder else { return }
        canvas.becomeFirstResponder()
    }

    private func cancelContacts(reason: UInt32) {
        bridge.cancelAllInput(reason: reason)
        touchInput.resetContacts()
        requestFrame()
    }

    private func sendInterfaceStyle() {
        let dark = traitCollection.userInterfaceStyle == .dark
        bridge.lifecycle(dark ? ONECAD_IOS_TRAIT_DARK : ONECAD_IOS_TRAIT_LIGHT)
        requestFrame()
    }

    // MARK: Frame

    private func runFrame() {
        guard surfaceAttached, bridge.isUsable else { return }
        let (status, result) = bridge.frame(time: CACurrentMediaTime())
        // PANIC/TERMINAL are latched by the bridge, which swaps in the error view.
        guard status == ONECAD_IOS_OK else { return }
        frameLoop?.scheduleRepaint(after: result.repaint_after_s)
        if result.flags & ONECAD_IOS_FRAME_REDRAW_REQUESTED != 0 { requestFrame() }
        textInput?.apply(result)
        if result.flags & ONECAD_IOS_FRAME_A11Y_CHANGED != 0 { accessibility?.rebuild() }
        if result.flags & ONECAD_IOS_FRAME_HOST_REQUESTS != 0 { hostRequests?.drain() }
        if result.flags & ONECAD_IOS_FRAME_KERNEL_TERMINAL != 0, !reportedKernelTerminal {
            // The editor keeps rendering its own relaunch-required state.
            reportedKernelTerminal = true
            bridgeLog.error("kernel reported relaunch-required")
        }
    }

    // MARK: Scene lifecycle (forwarded by SceneDelegate)

    func sceneWillResignActive() {
        cancelContacts(reason: ONECAD_IOS_CANCEL_SCENE_INACTIVE)
        bridge.lifecycle(ONECAD_IOS_WILL_RESIGN_ACTIVE)
    }

    func sceneDidEnterBackground() {
        // No Metal work is allowed in the background.
        frameLoop?.setSuspended(true)
        bridge.lifecycle(ONECAD_IOS_DID_ENTER_BACKGROUND)
    }

    func sceneWillEnterForeground() {
        bridge.lifecycle(ONECAD_IOS_WILL_ENTER_FOREGROUND)
        frameLoop?.setSuspended(false)
    }

    func sceneDidBecomeActive() {
        bridge.lifecycle(ONECAD_IOS_DID_BECOME_ACTIVE)
        capabilities?.publish()
        requestFrame()
    }

    /// Stops frames and releases the surface before the view goes away.
    func shutdown() {
        frameLoop?.stop()
        frameLoop?.setSuspended(true)
        detachSurface()
    }

    private func detachSurface() {
        guard surfaceAttached else { return }
        surfaceAttached = false
        bridge.detachSurface()
    }
}

// MARK: - Surface

extension CanvasViewController: CanvasSurfaceDelegate {
    func canvasSurfaceDidChange(_ view: MetalCanvasView) {
        guard bridge.isUsable, let desc = view.surfaceDescriptor() else { return }
        if surfaceAttached {
            bridge.resizeSurface(desc)
        } else {
            surfaceAttached = bridge.attachSurface(desc) == ONECAD_IOS_OK
        }
        accessibility?.refreshFrames()
        requestFrame()
    }

    func canvasSurfaceWillDetach(_ view: MetalCanvasView) {
        detachSurface()
    }
}

// MARK: - Input

extension CanvasViewController: CanvasInputSink, InputDelivery, FrameWakeTarget {
    func touches(_ touches: Set<UITouch>, event: UIEvent?, in view: UIView) {
        touchInput.touches(touches, event: event, in: view)
    }

    func estimatedPropertiesUpdated(_ touches: Set<UITouch>, in view: UIView) {
        touchInput.estimatedPropertiesUpdated(touches, in: view)
    }

    func presses(_ presses: Set<UIPress>, pressed: Bool) {
        textInput?.handlePresses(presses, pressed: pressed)
    }

    func deliver(samples: [onecad_ios_touch_sample]) {
        bridge.submit(samples: samples, viewGeneration: canvas.viewGeneration)
        requestFrame()
    }

    func observeCapabilities(_ flags: UInt32) {
        capabilities?.observe(flags)
    }

    /// Rust wake-ups only mark a frame as needed; they never run one inline.
    func wakeRequested() {
        requestFrame()
    }
}
