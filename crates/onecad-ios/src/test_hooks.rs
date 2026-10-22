//! Acceptance-only hooks (`onecad_ios_test.h`), compiled only with the
//! nondefault `test-hooks` feature. Synthetic samples go through the same
//! normalizer, router and route context as `onecad_ios_input_submit`
//! (`IosApp::submit_touches`) and every reply and log record about them is
//! labeled synthetic.
use crate::abi::{OpaqueApp, fail, utf8, with_app};
use crate::app::{Failure, IosApp, Outcome};
use crate::ffi::{self, Status, TouchSample};
use serde_json::{Value, json};
use std::ffi::{CString, c_char};

/// One replay sample in the trace fixture schema (`docs/ios/INPUT.md`
/// appendix), so `fixtures/ios/traces` batches replay as written.
fn sample(value: &Value) -> Result<TouchSample, String> {
    let raw = onecad_interaction::touch::trace::sample(value)?;
    Ok(TouchSample {
        contact_id: raw.contact_id,
        kind: raw.kind,
        phase: raw.phase,
        provenance: raw.provenance,
        present: raw.present,
        estimated: raw.estimated,
        expects_update: raw.expects_update,
        timestamp_s: raw.timestamp_s,
        x_pt: raw.x_pt,
        y_pt: raw.y_pt,
        force: raw.force,
        max_force: raw.max_force,
        altitude_rad: raw.altitude_rad,
        azimuth_rad: raw.azimuth_rad,
        roll_rad: raw.roll_rad,
        hover_z: raw.hover_z,
        estimation_index: raw.estimation_index,
        tap_count: raw.tap_count,
        reserved: 0,
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
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("malformed sample: {error}"))?;
            let generation = request
                .get("generation")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if samples.len() > crate::input::BATCH_MAX {
                return Err(format!(
                    "replay batch exceeds {} samples",
                    crate::input::BATCH_MAX
                ));
            }
            state.accept_input().map_err(|failure| failure.message)?;
            // The trace record of a synthetic batch says so (not Pencil hardware evidence).
            tracing::info!(
                synthetic = true,
                samples = samples.len(),
                generation,
                "onecad_ios test replay batch"
            );
            let overflowed = state
                .submit_touches(&samples, generation)
                .map_err(|failure| failure.message)?;
            Ok(json!({"synthetic": true, "accepted": samples.len(), "overflowed": overflowed}))
        }
        Some("prediction") => {
            let contact = request
                .get("contact")
                .and_then(Value::as_u64)
                .ok_or("contact")?;
            let prediction = state.input.prediction(contact);
            Ok(json!({"synthetic": true, "prediction": prediction.map(|p| [p.x, p.y])}))
        }
        Some("input_state") => input_state(&state.input, request),
        Some(cmd) if crate::test_hooks_semantic::COMMANDS.contains(&cmd) => {
            crate::test_hooks_semantic::command(state, cmd, request)
        }
        Some(other) => Err(format!("unsupported command: {other}")),
        None => Err("missing cmd".into()),
    }
}

/// Reads the live production input pipeline without draining or resetting it.
fn input_state(input: &crate::input::Input, request: &Value) -> Result<Value, String> {
    let contact = request
        .get("contact")
        .and_then(Value::as_u64)
        .ok_or("input_state needs contact")?;
    let index = request
        .get("estimationIndex")
        .and_then(Value::as_u64)
        .ok_or("input_state needs estimationIndex")?;
    let stats = input.normalizer_stats();
    Ok(json!({
        "synthetic": true,
        "liveContacts": input.live_contacts(),
        "stats": {
            "invalid": stats.invalid,
            "duplicateCoalesced": stats.duplicate_coalesced,
            "stale": stats.stale,
            "unknownContact": stats.unknown_contact,
            "oldGeneration": stats.old_generation,
            "refusedContacts": stats.refused_contacts,
            "estimationMatched": stats.estimation_matched,
            "estimationUnmatched": stats.estimation_unmatched,
            "mergedMoves": stats.merged_moves,
        },
        "pose": input.retained_pose(contact, index).map(pose_state),
        "prediction": input.prediction(contact).map(|point| [point.x, point.y]),
        "pencilPosition": input.pencil_position().map(|point| [point.x, point.y]),
    }))
}

fn pose_state(pose: onecad_interaction::touch::Pose) -> Value {
    json!({
        "force": pose.force,
        "maxForce": pose.max_force,
        "altitude": pose.altitude,
        "azimuth": pose.azimuth,
        "roll": pose.roll,
        "hoverZ": pose.hover_z,
        "estimated": pose.estimated,
        "expectsUpdate": pose.expects_update,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_state_reads_retained_pose_without_draining_or_resetting_input() {
        let mut input = crate::input::Input::default();
        let mut stroke = json!({
            "contact": 77, "kind": "pencil", "phase": "began", "t": 1.0,
            "x": 10, "y": 20, "force": 0.25, "max_force": 1,
            "estimated": ["force"], "expects_update": ["force"], "estimation_index": 9,
        });
        let query = json!({"contact": 77, "estimationIndex": 9});
        input
            .submit(
                &[sample(&stroke).unwrap()],
                1,
                &onecad_interaction::touch::RouteContext::CANVAS,
            )
            .unwrap();
        let initial = input_state(&input, &query).unwrap();
        assert_eq!(initial["liveContacts"], 1);
        assert_eq!(initial["pose"]["force"], 0.25);
        assert_eq!(input_state(&input, &query).unwrap(), initial);
        assert!(!input.drain().events.is_empty());
        stroke["provenance"] = json!("estimation_update");
        stroke["force"] = json!(0.75);
        stroke["estimated"] = json!([]);
        stroke["expects_update"] = json!([]);
        stroke["x"] = json!(999);
        input
            .submit(
                &[sample(&stroke).unwrap()],
                1,
                &onecad_interaction::touch::RouteContext::CANVAS,
            )
            .unwrap();
        let refined = input_state(&input, &query).unwrap();
        assert_eq!(refined["pose"]["force"], 0.75);
        assert_eq!(refined["pose"]["estimated"], 0);
        assert_eq!(refined["stats"]["estimationMatched"], 1);
        assert_eq!(refined["pencilPosition"], initial["pencilPosition"]);
        assert_eq!(refined["liveContacts"], initial["liveContacts"]);
        assert!(input.drain().events.is_empty());
    }

    /// Every batch of every trace fixture replays as written.
    #[test]
    fn trace_fixture_samples_parse_as_replay_samples() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ios/traces");
        let mut parsed = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let trace: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            for step in trace["steps"].as_array().unwrap() {
                for value in step["batch"]["samples"].as_array().into_iter().flatten() {
                    let replayed = sample(value).unwrap_or_else(|e| panic!("{path:?}: {e}"));
                    let traced = onecad_interaction::touch::trace::sample(value).unwrap();
                    assert_eq!(replayed.present, traced.present);
                    assert_eq!(replayed.estimation_index, traced.estimation_index);
                    parsed += 1;
                }
            }
        }
        assert!(parsed > 50, "{parsed} samples");
        let named =
            json!({"contact": 1, "kind": "finger", "phase": "began", "t": 0.0, "x": 1, "y": 2});
        assert_eq!(sample(&named).unwrap().kind, ffi::CONTACT_FINGER);
        assert!(sample(&json!({"contact": 1, "t": 0.0, "x": 1, "y": 2})).is_err());
    }
}
