import QuartzCore
import UIKit

@MainActor
protocol CanvasSurfaceDelegate: AnyObject {
    /// Size, scale or safe area changed while in a window (attach or resize).
    func canvasSurfaceDidChange(_ view: MetalCanvasView)
    /// The view left its window; the layer must be released by Rust now.
    func canvasSurfaceWillDetach(_ view: MetalCanvasView)
}

@MainActor
protocol CanvasInputSink: AnyObject {
    func touches(_ touches: Set<UITouch>, event: UIEvent?, in view: UIView)
    func estimatedPropertiesUpdated(_ touches: Set<UITouch>, in view: UIView)
    func presses(_ presses: Set<UIPress>, pressed: Bool)
}

/// CAMetalLayer-backed host view. Rust renders into the layer through wgpu;
/// this view only reports geometry, forwards raw input and hosts the
/// accessibility elements rebuilt from the Rust snapshot.
final class MetalCanvasView: UIView {
    static let accessibilityID = "onecad.canvas"

    override class var layerClass: AnyClass { CAMetalLayer.self }

    weak var surfaceDelegate: CanvasSurfaceDelegate?
    weak var inputSink: CanvasInputSink?
    /// Bumped on every bounds or scale change; stamps input batches.
    private(set) var viewGeneration: UInt64 = 1
    private var lastPixelSize = CGSize.zero
    private var lastScale: CGFloat = 0

    var metalLayer: CAMetalLayer {
        // layerClass guarantees the type.
        unsafeDowncast(layer, to: CAMetalLayer.self)
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = true
        isMultipleTouchEnabled = true
        backgroundColor = .black
        accessibilityIdentifier = Self.accessibilityID
        // A container: Rust's snapshot provides the actual elements.
        isAccessibilityElement = false
        registerForTraitChanges([UITraitDisplayScale.self]) { (view: MetalCanvasView, _) in
            view.setNeedsLayout()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    var displayScale: CGFloat {
        let scale = traitCollection.displayScale
        return scale > 0 ? scale : (window?.screen.scale ?? 2)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let scale = displayScale
        let pixels = CGSize(width: (bounds.width * scale).rounded(),
                            height: (bounds.height * scale).rounded())
        metalLayer.contentsScale = scale
        metalLayer.drawableSize = pixels
        if pixels != lastPixelSize || scale != lastScale {
            lastPixelSize = pixels
            lastScale = scale
            viewGeneration &+= 1
        }
        if window != nil { surfaceDelegate?.canvasSurfaceDidChange(self) }
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        if window != nil { surfaceDelegate?.canvasSurfaceDidChange(self) }
    }

    override func willMove(toWindow newWindow: UIWindow?) {
        super.willMove(toWindow: newWindow)
        if newWindow == nil, window != nil { surfaceDelegate?.canvasSurfaceWillDetach(self) }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        // Re-attach on the next layout pass even when the bounds are unchanged.
        if window != nil { setNeedsLayout() }
    }

    /// Nil while the view has no drawable area.
    func surfaceDescriptor() -> onecad_ios_surface_desc? {
        let scale = displayScale
        let width = (bounds.width * scale).rounded()
        let height = (bounds.height * scale).rounded()
        guard width >= 1, height >= 1 else { return nil }
        var desc = onecad_ios_surface_desc()
        desc.struct_size = UInt32(MemoryLayout<onecad_ios_surface_desc>.size)
        // The view retains its layer for as long as the surface is attached.
        desc.ca_metal_layer = Unmanaged.passUnretained(metalLayer).toOpaque()
        desc.width_px = UInt32(width)
        desc.height_px = UInt32(height)
        desc.scale = Float(scale)
        desc.safe_area = onecad_ios_insets(
            top: Float(safeAreaInsets.top), left: Float(safeAreaInsets.left),
            bottom: Float(safeAreaInsets.bottom), right: Float(safeAreaInsets.right))
        return desc
    }

    // MARK: Input

    override var canBecomeFirstResponder: Bool { true }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        inputSink?.touches(touches, event: event, in: self)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        inputSink?.touches(touches, event: event, in: self)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        inputSink?.touches(touches, event: event, in: self)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        inputSink?.touches(touches, event: event, in: self)
    }

    override func touchesEstimatedPropertiesUpdated(_ touches: Set<UITouch>) {
        inputSink?.estimatedPropertiesUpdated(touches, in: self)
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        inputSink?.presses(presses, pressed: true)
        super.pressesBegan(presses, with: event)
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        inputSink?.presses(presses, pressed: false)
        super.pressesEnded(presses, with: event)
    }

    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        inputSink?.presses(presses, pressed: false)
        super.pressesCancelled(presses, with: event)
    }
}
