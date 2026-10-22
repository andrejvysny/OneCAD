import UIKit

/// UTF-16 offset into the proxy's marked text.
final class IndexPosition: UITextPosition {
    let offset: Int

    init(_ offset: Int) {
        self.offset = offset
    }
}

final class IndexRange: UITextRange {
    let lower: IndexPosition
    let upper: IndexPosition

    init(_ lower: Int, _ upper: Int) {
        self.lower = IndexPosition(min(lower, upper))
        self.upper = IndexPosition(max(lower, upper))
    }

    override var start: UITextPosition { lower }
    override var end: UITextPosition { upper }
    override var isEmpty: Bool { lower.offset == upper.offset }
}

/// The remaining UITextInput surface. The proxy's "document" is only the
/// current marked text: committed text lives in Rust and is never mirrored.
extension KeyInputProxy {
    private func clamp(_ offset: Int) -> Int { max(0, min(offset, markedLength)) }

    private func offset(_ position: UITextPosition) -> Int {
        (position as? IndexPosition)?.offset ?? 0
    }

    func text(in range: UITextRange) -> String? {
        guard let range = range as? IndexRange else { return nil }
        let ns = markedText as NSString
        let lower = clamp(range.lower.offset)
        return ns.substring(with: NSRange(location: lower, length: clamp(range.upper.offset) - lower))
    }

    func replace(_ range: UITextRange, withText text: String) {
        insertText(text)
    }

    /// Rust owns the committed selection; only the IME's selection inside the
    /// marked text is meaningful here, and UIKit-initiated changes are ignored.
    var selectedTextRange: UITextRange? {
        get {
            let lower = clamp(markedSelection.location)
            return IndexRange(lower, clamp(lower + markedSelection.length))
        }
        set { _ = newValue }
    }

    var beginningOfDocument: UITextPosition { IndexPosition(0) }
    var endOfDocument: UITextPosition { IndexPosition(markedLength) }

    func textRange(from fromPosition: UITextPosition, to toPosition: UITextPosition) -> UITextRange? {
        IndexRange(offset(fromPosition), offset(toPosition))
    }

    func position(from position: UITextPosition, offset delta: Int) -> UITextPosition? {
        let target = offset(position) + delta
        return (0...markedLength).contains(target) ? IndexPosition(target) : nil
    }

    func position(from position: UITextPosition, in direction: UITextLayoutDirection,
                  offset delta: Int) -> UITextPosition? {
        switch direction {
        case .left, .up: self.position(from: position, offset: -delta)
        default: self.position(from: position, offset: delta)
        }
    }

    func compare(_ position: UITextPosition, to other: UITextPosition) -> ComparisonResult {
        let a = offset(position)
        let b = offset(other)
        return a < b ? .orderedAscending : (a > b ? .orderedDescending : .orderedSame)
    }

    func offset(from: UITextPosition, to toPosition: UITextPosition) -> Int {
        offset(toPosition) - offset(from)
    }

    func position(within range: UITextRange, farthestIn direction: UITextLayoutDirection) -> UITextPosition? {
        switch direction {
        case .left, .up: range.start
        default: range.end
        }
    }

    func characterRange(byExtending position: UITextPosition,
                        in direction: UITextLayoutDirection) -> UITextRange? {
        let at = offset(position)
        switch direction {
        case .left, .up: return IndexRange(0, at)
        default: return IndexRange(at, markedLength)
        }
    }

    func baseWritingDirection(for position: UITextPosition,
                              in direction: UITextStorageDirection) -> NSWritingDirection {
        .natural
    }

    func setBaseWritingDirection(_ writingDirection: NSWritingDirection, for range: UITextRange) {}

    func firstRect(for range: UITextRange) -> CGRect { caretRect }

    func caretRect(for position: UITextPosition) -> CGRect { caretRect }

    func selectionRects(for range: UITextRange) -> [UITextSelectionRect] { [] }

    func closestPosition(to point: CGPoint) -> UITextPosition? { endOfDocument }

    func closestPosition(to point: CGPoint, within range: UITextRange) -> UITextPosition? { range.end }

    func characterRange(at point: CGPoint) -> UITextRange? { nil }
}
