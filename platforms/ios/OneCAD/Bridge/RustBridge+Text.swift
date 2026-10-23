import UIKit

extension RustBridge {
    func textSnapshot() -> NativeTextSnapshot? {
        var value = onecad_ios_text_snapshot_v1()
        value.struct_size = UInt32(MemoryLayout<onecad_ios_text_snapshot_v1>.size)
        guard call("text_snapshot", { onecad_ios_text_snapshot($0, &value) }) == ONECAD_IOS_OK,
              value.active != 0, let text = value.text else { return nil }
        return NativeTextSnapshot(
            generation: value.generation, revision: value.revision, sequence: value.sequence,
            fieldID: value.field_id,
            text: String(decoding: UnsafeBufferPointer(start: UnsafeRawPointer(text).assumingMemoryBound(to: UInt8.self),
                                                       count: Int(value.text_len)), as: UTF8.self),
            selection: NSRange(location: Int(value.selection_start), length: Int(value.selection_end - value.selection_start)),
            marked: value.marked_start == UInt32.max ? nil : NSRange(location: Int(value.marked_start), length: Int(value.marked_end - value.marked_start)),
            label: value.label.map { String(cString: $0) } ?? "Text",
            fieldRect: CGRect(x: CGFloat(value.field_x), y: CGFloat(value.field_y), width: CGFloat(value.field_w), height: CGFloat(value.field_h)),
            caretRect: CGRect(x: CGFloat(value.caret_x), y: CGFloat(value.caret_y), width: CGFloat(value.caret_w), height: CGFloat(value.caret_h)))
    }

    /// Always refresh, including rejection: Swift advances only from Rust's acknowledged snapshot.
    func applyText(_ operation: NativeTextOperation) -> NativeTextSnapshot? {
        let (rangeEnd, rangeOverflow) = operation.range.location.addingReportingOverflow(operation.range.length)
        let (selectionEnd, selectionOverflow) = operation.selection.location.addingReportingOverflow(operation.selection.length)
        guard !rangeOverflow, !selectionOverflow, let start = UInt32(exactly: operation.range.location),
              let end = UInt32(exactly: rangeEnd),
              let selectedStart = UInt32(exactly: operation.selection.location),
              let selectedEnd = UInt32(exactly: selectionEnd) else { return textSnapshot() }
        var value = onecad_ios_text_operation_v1()
        value.struct_size = UInt32(MemoryLayout<onecad_ios_text_operation_v1>.size)
        value.version = 1
        value.generation = operation.generation
        value.revision = operation.revision
        value.sequence = operation.sequence
        value.kind = operation.kind
        value.start = start
        value.end = end
        value.selected_start = selectedStart
        value.selected_end = selectedEnd
        operation.text.utf8CString.withUnsafeBufferPointer { buffer in
            value.text = buffer.baseAddress
            value.text_len = UInt32(buffer.count - 1)
            _ = call("text_apply") { onecad_ios_text_apply($0, &value) }
        }
        return textSnapshot()
    }
}
