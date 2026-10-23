import UIKit

/// Acknowledged Rust draft, never accepted CAD geometry. All offsets are UTF-16.
struct NativeTextSnapshot {
    var generation: UInt64
    var revision: UInt64
    var sequence: UInt64
    var fieldID: UInt64
    var text: String
    var selection: NSRange
    var marked: NSRange?
    var label: String
    var fieldRect: CGRect
    var caretRect: CGRect
}

struct NativeTextOperation {
    var generation: UInt64
    var revision: UInt64
    var sequence: UInt64
    var kind: UInt32
    var range = NSRange(location: 0, length: 0)
    var selection = NSRange(location: 0, length: 0)
    var text = ""
}

extension NativeTextSnapshot {
    func valid(_ range: NSRange) -> Bool {
        let (end, overflow) = range.location.addingReportingOverflow(range.length)
        guard !overflow, range.location >= 0, range.length >= 0,
              end <= text.utf16.count else { return false }
        return scalarBoundary(range.location) && scalarBoundary(end)
    }

    func scalarBoundary(_ offset: Int) -> Bool {
        guard offset >= 0, offset <= text.utf16.count else { return false }
        let index = text.utf16.index(text.utf16.startIndex, offsetBy: offset)
        return index.samePosition(in: text.unicodeScalars) != nil
    }
}
