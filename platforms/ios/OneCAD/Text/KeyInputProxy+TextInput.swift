import UIKit

/// UTF-16 offsets belong only to the focus generation that created them.
final class IndexPosition: UITextPosition {
    let offset: Int
    let generation: UInt64
    init(_ offset: Int, generation: UInt64) { self.offset = offset; self.generation = generation }
}

final class IndexRange: UITextRange {
    let lower: IndexPosition
    let upper: IndexPosition
    init(_ lower: Int, _ upper: Int, generation: UInt64) {
        self.lower = IndexPosition(min(lower, upper), generation: generation)
        self.upper = IndexPosition(max(lower, upper), generation: generation)
    }
    override var start: UITextPosition { lower }
    override var end: UITextPosition { upper }
    override var isEmpty: Bool { lower.offset == upper.offset }
}

/// Character tokenization must not need positions inside a surrogate pair.
/// Preserve the adapter's scalar fence while returning complete graphemes.
final class DraftTokenizer: UITextInputStringTokenizer {
    private weak var input: KeyInputProxy?

    init(input: KeyInputProxy) {
        self.input = input
        super.init(textInput: input)
    }

    private func context(_ position: UITextPosition) -> (NativeTextSnapshot, Int)? {
        guard let session = input?.session, let position = position as? IndexPosition,
              position.generation == session.generation, session.scalarBoundary(position.offset)
        else { return nil }
        return (session, position.offset)
    }

    private func backward(_ direction: UITextDirection) -> Bool {
        direction.rawValue == UITextStorageDirection.backward.rawValue
            || direction.rawValue == UITextLayoutDirection.left.rawValue
            || direction.rawValue == UITextLayoutDirection.up.rawValue
    }

    private func character(_ session: NativeTextSnapshot, at offset: Int,
                           direction: UITextDirection) -> NSRange? {
        let text = session.text as NSString
        if backward(direction) {
            return offset > 0 ? text.rangeOfComposedCharacterSequence(at: offset - 1) : nil
        }
        return offset < text.length ? text.rangeOfComposedCharacterSequence(at: offset) : nil
    }

    override func rangeEnclosingPosition(_ position: UITextPosition, with granularity: UITextGranularity,
                                         inDirection direction: UITextDirection) -> UITextRange? {
        guard let (session, offset) = context(position) else { return nil }
        guard granularity == .character else {
            return super.rangeEnclosingPosition(position, with: granularity, inDirection: direction)
        }
        guard let range = character(session, at: offset, direction: direction) else { return nil }
        return IndexRange(range.location, range.upperBound, generation: session.generation)
    }

    override func isPosition(_ position: UITextPosition, atBoundary granularity: UITextGranularity,
                             inDirection direction: UITextDirection) -> Bool {
        guard let (session, offset) = context(position) else { return false }
        guard granularity == .character else {
            return super.isPosition(position, atBoundary: granularity, inDirection: direction)
        }
        if offset == 0 || offset == session.text.utf16.count { return true }
        return (session.text as NSString).rangeOfComposedCharacterSequence(at: offset - 1).upperBound == offset
    }

    override func position(from position: UITextPosition, toBoundary granularity: UITextGranularity,
                           inDirection direction: UITextDirection) -> UITextPosition? {
        guard let (session, offset) = context(position) else { return nil }
        guard granularity == .character else {
            return super.position(from: position, toBoundary: granularity, inDirection: direction)
        }
        guard let range = character(session, at: offset, direction: direction) else { return nil }
        return IndexPosition(backward(direction) ? range.location : range.upperBound, generation: session.generation)
    }

    override func isPosition(_ position: UITextPosition, withinTextUnit granularity: UITextGranularity,
                             inDirection direction: UITextDirection) -> Bool {
        guard let (session, offset) = context(position) else { return false }
        return granularity == .character
            ? character(session, at: offset, direction: direction) != nil
            : super.isPosition(position, withinTextUnit: granularity, inDirection: direction)
    }
}

extension KeyInputProxy {
    private func offset(_ position: UITextPosition) -> Int? {
        guard let position = position as? IndexPosition, let session,
              position.generation == session.generation,
              session.scalarBoundary(position.offset) else { return nil }
        return position.offset
    }

    private func offsets(_ range: UITextRange) -> NSRange? {
        guard let lower = offset(range.start), let upper = offset(range.end), upper >= lower else { return nil }
        return NSRange(location: lower, length: upper - lower)
    }

    func text(in range: UITextRange) -> String? {
        guard let range = offsets(range), let session, session.valid(range) else { return nil }
        return (session.text as NSString).substring(with: range)
    }

    func replace(_ range: UITextRange, withText text: String) {
        guard let range = offsets(range) else { return }
        submit(ONECAD_IOS_TEXT_REPLACE.rawValue, range: range, text: text)
    }

    var selectedTextRange: UITextRange? {
        get {
            guard let session else { return nil }
            return IndexRange(session.selection.location, session.selection.upperBound, generation: session.generation)
        }
        set {
            guard let newValue, let range = offsets(newValue) else { return }
            submit(ONECAD_IOS_TEXT_SELECT.rawValue, range: range)
        }
    }

    var beginningOfDocument: UITextPosition { IndexPosition(0, generation: session?.generation ?? 0) }
    var endOfDocument: UITextPosition { IndexPosition(session?.text.utf16.count ?? 0, generation: session?.generation ?? 0) }

    func textRange(from fromPosition: UITextPosition, to toPosition: UITextPosition) -> UITextRange? {
        guard let lower = offset(fromPosition), let upper = offset(toPosition), let session else { return nil }
        return IndexRange(lower, upper, generation: session.generation)
    }

    func position(from position: UITextPosition, offset delta: Int) -> UITextPosition? {
        guard let at = offset(position), let session else { return nil }
        let (target, overflow) = at.addingReportingOverflow(delta)
        guard !overflow, session.scalarBoundary(target) else { return nil }
        return IndexPosition(target, generation: session.generation)
    }

    func position(from position: UITextPosition, in direction: UITextLayoutDirection,
                  offset delta: Int) -> UITextPosition? {
        switch direction {
        case .left, .up:
            guard delta != Int.min else { return nil }
            return self.position(from: position, offset: -delta)
        default: return self.position(from: position, offset: delta)
        }
    }

    func compare(_ position: UITextPosition, to other: UITextPosition) -> ComparisonResult {
        guard let a = offset(position), let b = offset(other) else { return .orderedSame }
        return a < b ? .orderedAscending : (a > b ? .orderedDescending : .orderedSame)
    }
    func offset(from: UITextPosition, to toPosition: UITextPosition) -> Int {
        guard let lower = offset(from), let upper = offset(toPosition) else { return 0 }
        return upper - lower
    }
    func position(within range: UITextRange, farthestIn direction: UITextLayoutDirection) -> UITextPosition? {
        guard offsets(range) != nil else { return nil }
        return direction == .left || direction == .up ? range.start : range.end
    }
    func characterRange(byExtending position: UITextPosition,
                        in direction: UITextLayoutDirection) -> UITextRange? {
        guard let at = offset(position), let session else { return nil }
        let ns = session.text as NSString
        let backward = direction == .left || direction == .up
        guard backward ? at > 0 : at < ns.length else { return nil }
        let range = ns.rangeOfComposedCharacterSequence(at: backward ? at - 1 : at)
        return IndexRange(range.location, range.upperBound, generation: session.generation)
    }
    func baseWritingDirection(for position: UITextPosition,
                              in direction: UITextStorageDirection) -> NSWritingDirection { .natural }
    func setBaseWritingDirection(_ writingDirection: NSWritingDirection, for range: UITextRange) {}
    func firstRect(for range: UITextRange) -> CGRect { offsets(range) == nil ? .null : caretRect }
    func caretRect(for position: UITextPosition) -> CGRect { offset(position) == nil ? .null : caretRect }
    func selectionRects(for range: UITextRange) -> [UITextSelectionRect] { [] }
    // No glyph hit-test geometry is exported by Rust. Do not invent positions for distant points.
    func closestPosition(to point: CGPoint) -> UITextPosition? { nil }
    func closestPosition(to point: CGPoint, within range: UITextRange) -> UITextPosition? { nil }
    func characterRange(at point: CGPoint) -> UITextRange? { nil }
}
