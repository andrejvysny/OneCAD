import UIKit

/// One UIAccessibilityElement per Rust node id. Elements are reused across
/// snapshots so VoiceOver focus and XCUITest references stay stable.
final class CanvasAccessibilityElement: UIAccessibilityElement {
    let nodeID: UInt64
    /// Frame in canvas coordinates; screen coordinates are derived from it.
    var localFrame = CGRect.zero
    var activate: ((UInt64) -> Bool)?

    init(container: UIView, nodeID: UInt64) {
        self.nodeID = nodeID
        super.init(accessibilityContainer: container)
    }

    override func accessibilityActivate() -> Bool {
        activate?(nodeID) ?? false
    }
}

@MainActor
final class AccessibilityBridge {
    private weak var container: UIView?
    private let bridge: RustBridge
    private var elements: [UInt64: CanvasAccessibilityElement] = [:]
    private var order: [UInt64] = []
    var onActivate: (() -> Void)?

    init(container: UIView, bridge: RustBridge) {
        self.container = container
        self.bridge = bridge
    }

    /// Called when a frame reports A11Y_CHANGED.
    func rebuild() {
        guard let container, let snapshot = bridge.accessibilitySnapshot() else { return }
        var next: [UInt64: CanvasAccessibilityElement] = [:]
        var nextOrder: [UInt64] = []
        for node in snapshot.nodes where node.flags & ONECAD_IOS_A11Y_HIDDEN == 0 {
            let element = elements[node.id] ?? makeElement(node.id, in: container)
            configure(element, with: node)
            next[node.id] = element
            nextOrder.append(node.id)
        }
        let structureChanged = nextOrder != order
        elements = next
        order = nextOrder
        container.accessibilityElements = nextOrder.compactMap { next[$0] }
        refreshFrames()
        if structureChanged { UIAccessibility.post(notification: .layoutChanged, argument: nil) }
    }

    /// Screen coordinates go stale when the canvas moves (rotation, split view).
    func refreshFrames() {
        guard let container else { return }
        for element in elements.values {
            element.accessibilityFrame = UIAccessibility.convertToScreenCoordinates(
                element.localFrame, in: container)
        }
    }

    private func makeElement(_ id: UInt64, in container: UIView) -> CanvasAccessibilityElement {
        let element = CanvasAccessibilityElement(container: container, nodeID: id)
        element.activate = { [weak self] nodeID in
            MainActor.assumeIsolated { self?.activate(nodeID) ?? false }
        }
        return element
    }

    private func activate(_ id: UInt64) -> Bool {
        let handled = bridge.activateAccessibility(id)
        if handled { onActivate?() }
        return handled
    }

    private func configure(_ element: CanvasAccessibilityElement, with node: AccessibilityNode) {
        element.localFrame = node.frame
        element.accessibilityIdentifier = node.identifier.isEmpty ? nil : node.identifier
        element.accessibilityLabel = node.label.isEmpty ? nil : node.label
        element.accessibilityValue = node.value.isEmpty ? nil : node.value
        element.accessibilityTraits = Self.traits(role: node.role, flags: node.flags)
    }

    static func traits(role: UInt32, flags: UInt32) -> UIAccessibilityTraits {
        var traits: UIAccessibilityTraits = switch role {
        case ONECAD_IOS_ROLE_BUTTON.rawValue, ONECAD_IOS_ROLE_MENU_ITEM.rawValue,
             ONECAD_IOS_ROLE_TAB.rawValue: .button
        case ONECAD_IOS_ROLE_CHECKBOX.rawValue: .toggleButton
        case ONECAD_IOS_ROLE_STATIC_TEXT.rawValue: .staticText
        case ONECAD_IOS_ROLE_IMAGE.rawValue: .image
        case ONECAD_IOS_ROLE_CANVAS.rawValue: .allowsDirectInteraction
        default: []
        }
        if flags & ONECAD_IOS_A11Y_DISABLED != 0 { traits.insert(.notEnabled) }
        if flags & ONECAD_IOS_A11Y_SELECTED != 0 { traits.insert(.selected) }
        return traits
    }
}
