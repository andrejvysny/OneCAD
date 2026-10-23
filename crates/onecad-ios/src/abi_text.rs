//! Versioned native draft operations. No operation calls a CAD mutation service.
use crate::{
    abi::{self, OpaqueApp},
    app::Failure,
    ffi::Status,
};
use onecad_widgets::text_session::{self, Edit, Operation, Range};
use std::ffi::{CString, c_char};

#[repr(C)]
pub struct TextSnapshot {
    pub struct_size: u32,
    pub version: u32,
    pub generation: u64,
    pub revision: u64,
    pub sequence: u64,
    pub field_id: u64,
    pub text: *const c_char,
    pub text_len: u32,
    pub selection_start: u32,
    pub selection_end: u32,
    pub marked_start: u32,
    pub marked_end: u32,
    pub active: u32,
    pub field_x: f32,
    pub field_y: f32,
    pub field_w: f32,
    pub field_h: f32,
    pub caret_x: f32,
    pub caret_y: f32,
    pub caret_w: f32,
    pub caret_h: f32,
    pub label: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TextOperation {
    pub struct_size: u32,
    pub version: u32,
    pub generation: u64,
    pub revision: u64,
    pub sequence: u64,
    pub kind: u32,
    pub start: u32,
    pub end: u32,
    pub selected_start: u32,
    pub selected_end: u32,
    pub text_len: u32,
    pub text: *const c_char,
}

/// # Safety
/// `app` is live; `out` is writable for its size. Strings live until next text snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_text_snapshot(
    app: *mut OpaqueApp,
    out: *mut TextSnapshot,
) -> Status {
    // SAFETY: the caller supplies the live handle and writable size-tagged output.
    unsafe {
        abi::with_app(app, |app| {
            abi::check_size(out)?;
            let current = text_session::snapshot(&app.ctx);
            app.text_strings.clear();
            app.text_strings.push(
                CString::new(current.as_ref().map_or("", |s| &s.text))
                    .map_err(|_| Failure::new(Status::InvalidArgument, "text contains NUL"))?,
            );
            app.text_strings.push(
                CString::new(current.as_ref().map_or("", |s| &s.label))
                    .map_err(|_| Failure::new(Status::InvalidArgument, "label contains NUL"))?,
            );
            let mut result = empty(app.text_strings[0].as_ptr(), app.text_strings[1].as_ptr());
            if let Some(s) = current {
                result.active = 1;
                result.generation = s.generation;
                result.revision = s.revision;
                result.sequence = s.sequence;
                result.field_id = s.field.value();
                result.text_len = s.text.len() as u32;
                result.selection_start = s.selection.start;
                result.selection_end = s.selection.end;
                if let Some(marked) = s.marked {
                    result.marked_start = marked.start;
                    result.marked_end = marked.end;
                }
                (
                    result.field_x,
                    result.field_y,
                    result.field_w,
                    result.field_h,
                ) = rect(s.rect);
                (
                    result.caret_x,
                    result.caret_y,
                    result.caret_w,
                    result.caret_h,
                ) = rect(s.caret);
            }
            out.write(result);
            Ok(())
        })
    }
}

fn rect(r: egui::Rect) -> (f32, f32, f32, f32) {
    (r.min.x, r.min.y, r.width(), r.height())
}
fn empty(text: *const c_char, label: *const c_char) -> TextSnapshot {
    TextSnapshot {
        struct_size: std::mem::size_of::<TextSnapshot>() as u32,
        version: 1,
        generation: 0,
        revision: 0,
        sequence: 0,
        field_id: 0,
        text,
        text_len: 0,
        selection_start: 0,
        selection_end: 0,
        marked_start: u32::MAX,
        marked_end: u32::MAX,
        active: 0,
        field_x: 0.,
        field_y: 0.,
        field_w: 0.,
        field_h: 0.,
        caret_x: 0.,
        caret_y: 0.,
        caret_w: 0.,
        caret_h: 0.,
        label,
    }
}

/// # Safety
/// `app` is live; `operation` is readable and its text covers `text_len` bytes, not NUL terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_text_apply(
    app: *mut OpaqueApp,
    operation: *const TextOperation,
) -> Status {
    // SAFETY: the caller supplies the sized operation and readable bounded UTF-8 buffer.
    unsafe {
        abi::with_app(app, |app| {
            app.accept_input()?;
            let op = abi::read_struct(operation)?;
            if op.version != 1
                || op.text_len as usize > text_session::MAX_UTF16 * 4
                || (op.text_len > 0 && op.text.is_null())
            {
                return Err(Failure::new(
                    Status::InvalidArgument,
                    "invalid text operation version or length",
                ));
            }
            let bytes = if op.text_len == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(op.text.cast::<u8>(), op.text_len as usize)
            };
            let text = std::str::from_utf8(bytes)
                .map_err(|_| Failure::new(Status::InvalidArgument, "invalid text UTF-8"))?;
            if text.contains('\0') {
                return Err(Failure::new(Status::InvalidArgument, "text contains NUL"));
            }
            let range = Range {
                start: op.start,
                end: op.end,
            };
            let edit = match op.kind {
                0 => Edit::Replace(range, text.into()),
                1 => Edit::Select(range),
                2 => Edit::Mark(
                    text.into(),
                    Range {
                        start: op.selected_start,
                        end: op.selected_end,
                    },
                ),
                3 => Edit::Unmark,
                4 => Edit::Cancel,
                5 => Edit::Confirm(false),
                6 => Edit::Confirm(true),
                7 => Edit::Blur,
                _ => {
                    return Err(Failure::new(
                        Status::InvalidArgument,
                        "unknown text operation",
                    ));
                }
            };
            let accepted = text_session::apply(
                &app.ctx,
                Operation {
                    generation: op.generation,
                    revision: op.revision,
                    sequence: op.sequence,
                    edit,
                },
            );
            if !accepted {
                return Err(Failure::new(
                    Status::State,
                    "stale or invalid text operation",
                ));
            }
            app.input_changed();
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_layouts_match_c_header_arm64() {
        assert_eq!(std::mem::size_of::<TextOperation>(), 64);
        assert_eq!(std::mem::offset_of!(TextOperation, text), 56);
        assert_eq!(std::mem::size_of::<TextSnapshot>(), 112);
        assert_eq!(std::mem::offset_of!(TextSnapshot, label), 104);
    }
}
