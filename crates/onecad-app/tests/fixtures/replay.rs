use std::path::Path;
use std::process::Stdio;

use onecad_protocol::client::ProtocolClient;
use serde_json::Value;

fn matches(expected: &Value, actual: &Value, tolerance: f64) {
    if expected == "$any" {
        return;
    }
    match expected {
        Value::Object(fields) => {
            let actual = actual.as_object().expect("expected object");
            for (key, value) in fields {
                if key != "tolerance" {
                    matches(
                        value,
                        actual.get(key).expect("missing expected key"),
                        tolerance,
                    );
                }
            }
        }
        Value::Array(values) => {
            let found = actual.as_array().expect("expected array");
            assert_eq!(values.len(), found.len(), "{expected} versus {actual}");
            for (value, found) in values.iter().zip(found) {
                matches(value, found, tolerance);
            }
        }
        Value::Number(number) => assert!(
            (number.as_f64().unwrap() - actual.as_f64().expect("expected number")).abs()
                <= tolerance,
            "{expected} versus {actual}, tolerance {tolerance}"
        ),
        _ => assert_eq!(expected, actual),
    }
}

async fn connect(worker: &Path) -> (tokio::process::Child, ProtocolClient) {
    let mut child = tokio::process::Command::new(worker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        ProtocolClient::connect(child.stdout.take().unwrap(), child.stdin.take().unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    (child, client)
}

fn check_truth(sketch: &str, last: &Value) {
    let parsed =
        onecad_app::worker::wire::parse_sketch_upsert_truth_checked(sketch, &last["result"])
            .unwrap();
    assert_eq!(
        serde_json::to_value(parsed.truth.reasons).unwrap(),
        last["result"]["reasons"]
    );
    assert_eq!(
        parsed.truth.violations.len(),
        last["result"]["violations"].as_array().unwrap().len()
    );
    if let Some(positions) = last["result"].get("positions") {
        assert_eq!(
            serde_json::to_value(parsed.solved_positions).unwrap(),
            *positions
        );
    }
    if let Some(curves) = last["result"].get("curves") {
        assert_eq!(serde_json::to_value(parsed.solved_curves).unwrap(), *curves);
    }
}

pub async fn replay(worker: &Path, fixture: &Path) {
    let (mut child, client) = connect(worker).await;
    let source = std::fs::read_to_string(fixture).unwrap();
    let mut last = serde_json::json!({"v":1,"t":"hello","seq":0,"result":client.hello()});
    let mut sketch = String::new();
    for line in source
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
    {
        let round: Value = serde_json::from_str(line).unwrap();
        if let Some(send) = round.get("send") {
            let verb = send["verb"].as_str().unwrap();
            if let Some(id) = send["args"]["sketchId"].as_str() {
                sketch = id.to_string();
            }
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                client.request(verb, send["args"].clone()),
            )
            .await
            .unwrap()
            .unwrap();
            last = serde_json::to_value(response).unwrap();
            last["v"] = serde_json::json!(1);
            last["t"] = serde_json::json!("resp");
            if last["ok"] == true && verb == "BeginGesture" {
                let parsed = onecad_app::worker::wire::parse_begin_gesture(
                    send["args"]["gestureId"].as_u64().unwrap(),
                    &last["result"],
                );
                assert_eq!(
                    serde_json::to_value(parsed.point_offset).unwrap(),
                    last["result"]
                        .get("pointOffset")
                        .cloned()
                        .unwrap_or(Value::Null)
                );
            }
            if last["ok"] == true && verb == "SolveDrag" {
                let parsed = onecad_app::worker::wire::parse_solve_drag(&last["result"]);
                if last["result"].get("diagnostics").is_some() {
                    assert_eq!(
                        parsed.diagnostics.as_ref().unwrap()[0]
                            .reason_code
                            .as_deref(),
                        last["result"]["diagnostics"][0]["reasonCode"].as_str()
                    );
                }
            }
            if last["ok"] == true
                && matches!(verb, "SketchUpsert" | "EndGesture")
                && last["result"]["cancelled"] != true
            {
                check_truth(&sketch, &last);
            }
        } else if let Some(expect) = round.get("expect") {
            let tolerance = expect["tolerance"]["abs"].as_f64().unwrap_or(0.0);
            matches(expect, &last, tolerance);
        }
    }
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}
