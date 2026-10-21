//! Acceptance-only hooks (`onecad_ios_test.h`), compiled only with the
//! nondefault `test-hooks` feature. Synthetic samples go through the same
//! `Input::submit` normalization as `onecad_ios_input_submit` and every reply
//! about them is labeled synthetic.
use crate::abi::{OpaqueApp, fail, utf8, with_app};
use crate::app::{Failure, IosApp, Outcome};
use crate::ffi::{self, Status, TouchSample};
use serde_json::{Value, json};
use std::ffi::{CString, c_char};

fn sample(value: &Value) -> Option<TouchSample> {
    let number = |key: &str| value.get(key).and_then(Value::as_f64);
    let integer = |key: &str| value.get(key).and_then(Value::as_u64);
    Some(TouchSample {
        contact_id: integer("contact")?,
        kind: u32::try_from(integer("kind").unwrap_or(u64::from(ffi::CONTACT_PENCIL))).ok()?,
        phase: u32::try_from(integer("phase")?).ok()?,
        provenance: u32::try_from(integer("provenance").unwrap_or(0)).ok()?,
        timestamp_s: number("t")?,
        x_pt: number("x")?,
        y_pt: number("y")?,
        ..Default::default()
    })
}

fn command(state: &mut IosApp, request: &Value) -> Result<Value, String> {
    match request.get("cmd").and_then(Value::as_str) {
        Some("ping") => Ok(json!({"abi": ffi::ABI_VERSION})),
        Some("replay") => {
            let samples = request
                .get("samples")
                .and_then(Value::as_array)
                .ok_or("replay needs samples")?
                .iter()
                .map(sample)
                .collect::<Option<Vec<_>>>()
                .ok_or("malformed sample")?;
            let generation = request
                .get("generation")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let overflowed = state.input.submit(&samples, generation);
            state.ctx.request_repaint();
            Ok(json!({"synthetic": true, "accepted": samples.len(), "overflowed": overflowed}))
        }
        Some("prediction") => {
            let contact = request
                .get("contact")
                .and_then(Value::as_u64)
                .ok_or("contact")?;
            let prediction = state.input.prediction(contact);
            Ok(
                json!({"synthetic": true, "prediction": prediction.map(|p| [p.position.x, p.position.y])}),
            )
        }
        Some(other) => Err(format!("unsupported command: {other}")),
        None => Err("missing cmd".into()),
    }
}

fn reply(state: &mut IosApp, json_text: &str) -> String {
    let result = serde_json::from_str::<Value>(json_text)
        .map_err(|error| error.to_string())
        .and_then(|request| command(state, &request));
    match result {
        Ok(Value::Object(mut fields)) => {
            fields.insert("ok".into(), Value::Bool(true));
            Value::Object(fields).to_string()
        }
        Ok(other) => json!({"ok": true, "value": other}).to_string(),
        Err(error) => json!({"ok": false, "error": error}).to_string(),
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `json_utf8` is NUL-terminated;
/// `out_json_utf8` is writable and receives a string for
/// `onecad_ios_test_string_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_test_command(
    app: *mut OpaqueApp,
    json_utf8: *const c_char,
    out_json_utf8: *mut *mut c_char,
) -> Status {
    if out_json_utf8.is_null() {
        return fail(Failure::new(
            Status::InvalidArgument,
            "out_json_utf8 is NULL",
        ));
    }
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| -> Outcome {
            let text = reply(state, &utf8(json_utf8, "json")?);
            let text =
                CString::new(text).map_err(|e| Failure::new(Status::Internal, e.to_string()))?;
            out_json_utf8.write(text.into_raw());
            Ok(())
        })
    }
}

/// # Safety
/// `s` is NULL or a string returned by `onecad_ios_test_command`, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_test_string_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: `s` came from `CString::into_raw` and is freed exactly once.
        drop(unsafe { CString::from_raw(s) });
    }
}
