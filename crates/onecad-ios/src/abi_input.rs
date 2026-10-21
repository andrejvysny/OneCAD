//! Exported input, text, accessibility and host-request functions of
//! `onecad_ios.h`. Every one runs through `abi::with_app`; inputs are copied into
//! the app's queues before returning and drawn on the next frame.
use crate::abi::{OpaqueApp, read_struct, utf8, with_app};
use crate::app::{Failure, IosApp, Outcome};
use crate::ffi::{self, Status};
use crate::input::BATCH_MAX;
use std::ffi::{CString, c_char};

impl IosApp {
    /// Input changes what the next frame shows; let the host schedule it.
    fn input_changed(&self) {
        self.ctx.request_repaint();
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `batch` points at a readable batch whose
/// `samples` holds `count` samples.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_input_submit(
    app: *mut OpaqueApp,
    batch: *const ffi::InputBatch,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            let batch = read_struct(batch)?;
            let count = batch.count as usize;
            if count > BATCH_MAX || (count > 0 && batch.samples.is_null()) {
                return Err(Failure::new(
                    Status::InvalidArgument,
                    "batch count or samples",
                ));
            }
            let samples = if count == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts(batch.samples, count)
            };
            let overflowed = state.input.submit(samples, batch.view_generation);
            state.input_changed();
            if overflowed {
                return Err(Failure::new(
                    Status::Overflow,
                    "pending input full; intermediate movement was coalesced",
                ));
            }
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_input_cancel_all(app: *mut OpaqueApp, reason: u32) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            if reason > ffi::CANCEL_TEXT_FOCUS {
                return Err(Failure::new(
                    Status::InvalidArgument,
                    "unknown cancel reason",
                ));
            }
            state.input.cancel_all();
            state.input_changed();
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `nav` is readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_navigation_submit(
    app: *mut OpaqueApp,
    nav: *const ffi::Navigation,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            if !state.input.navigation(&read_struct(nav)?) {
                return Err(Failure::new(
                    Status::InvalidArgument,
                    "navigation phase or values",
                ));
            }
            state.input_changed();
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `ev` is readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_pencil_event_submit(
    app: *mut OpaqueApp,
    ev: *const ffi::PencilEvent,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |_| {
            read_struct(ev)?;
            Err(Failure::new(
                Status::Unavailable,
                "Pencil barrel tap and squeeze are not routed to tools in this build",
            ))
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `caps` is readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_capabilities_set(
    app: *mut OpaqueApp,
    caps: *const ffi::Capabilities,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            state.capabilities = read_struct(caps)?;
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `ev` is readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_key_submit(
    app: *mut OpaqueApp,
    ev: *const ffi::KeyEvent,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            state.input.key(&read_struct(ev)?);
            state.input_changed();
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `utf8` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_text_insert(
    app: *mut OpaqueApp,
    text: *const c_char,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            state.input.insert_text(&utf8(text, "text")?);
            state.input_changed();
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_text_delete_backward(app: *mut OpaqueApp) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            state.input.delete_backward();
            state.input_changed();
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `text` is NULL or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_text_set_marked(
    app: *mut OpaqueApp,
    text: *const c_char,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            let marked = if text.is_null() {
                String::new()
            } else {
                utf8(text, "marked text")?
            };
            state.input.set_marked(&marked);
            state.input_changed();
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_keyboard_frame(app: *mut OpaqueApp, overlap_pt: f32) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_app(app, |state| state.set_keyboard_overlap(overlap_pt)) }
}

/// # Safety
/// `app` is NULL or a live app pointer; the three out pointers are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_a11y_snapshot(
    app: *mut OpaqueApp,
    out_nodes: *mut *const ffi::A11yNode,
    out_count: *mut u32,
    out_generation: *mut u64,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            if out_nodes.is_null() || out_count.is_null() || out_generation.is_null() {
                return Err(Failure::new(Status::InvalidArgument, "out pointer is NULL"));
            }
            let (nodes, count) = state.a11y.snapshot();
            out_nodes.write(nodes);
            out_count.write(count);
            out_generation.write(state.a11y.generation());
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_a11y_activate(app: *mut OpaqueApp, id: u64) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_app(app, |state| activate(state, id)) }
}

fn activate(state: &mut IosApp, id: u64) -> Outcome {
    use crate::a11y::Activation;
    let activation = state
        .a11y
        .activation(id)
        .ok_or_else(|| Failure::new(Status::InvalidArgument, "unknown accessibility node"))?;
    match activation {
        Activation::Click(node) => {
            let request = egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: node,
                data: None,
            };
            state
                .input
                .push_event(egui::Event::AccessKitActionRequest(request));
        }
        Activation::Tap(pos) => state.input.synthesized_tap(pos),
    }
    state.input_changed();
    Ok(())
}

/// # Safety
/// `app` is NULL or a live app pointer; `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_host_request_next(
    app: *mut OpaqueApp,
    out: *mut ffi::HostRequest,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            if out.is_null() {
                return Err(Failure::new(Status::InvalidArgument, "out is NULL"));
            }
            let request = state
                .host
                .pop()
                .ok_or_else(|| Failure::new(Status::State, "no pending host request"))?;
            out.write(state.materialize(request));
            Ok(())
        })
    }
}

impl IosApp {
    /// C view of a request; its strings live until the next `request_next`.
    fn materialize(&mut self, request: crate::host::Request) -> ffi::HostRequest {
        let fields = [
            request.label,
            request.extensions,
            request.path,
            request.suggested_name,
            request.text,
        ];
        self.request_strings = fields
            .into_iter()
            .map(|text| CString::new(text.replace('\0', "")).unwrap_or_default())
            .collect();
        let text = |index: usize| self.request_strings[index].as_ptr();
        ffi::HostRequest {
            id: request.id,
            kind: request.kind,
            multiple: u32::from(request.multiple),
            label: text(0),
            extensions: text(1),
            path: text(2),
            suggested_name: text(3),
            text: text(4),
        }
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `paths` holds `count` NUL-terminated
/// strings (it may be NULL when `count` is 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_host_request_complete(
    app: *mut OpaqueApp,
    id: u64,
    paths: *const *const c_char,
    count: u32,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            if count > 0 && paths.is_null() {
                return Err(Failure::new(Status::InvalidArgument, "paths is NULL"));
            }
            let mut owned = Vec::with_capacity(count as usize);
            for index in 0..count as usize {
                owned.push(utf8(*paths.add(index), "path")?);
            }
            if !state.host.complete(id, owned) {
                return Err(Failure::new(Status::InvalidArgument, "unknown request id"));
            }
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `path` is NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_open_path(app: *mut OpaqueApp, path: *const c_char) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            let path = std::path::PathBuf::from(utf8(path, "path")?);
            if !path.is_absolute() {
                return Err(Failure::new(
                    Status::InvalidArgument,
                    "path must be absolute",
                ));
            }
            state.editor()?.open_path(path);
            state.input_changed();
            Ok(())
        })
    }
}
