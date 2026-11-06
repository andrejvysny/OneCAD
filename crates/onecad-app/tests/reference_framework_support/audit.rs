use super::*;
use std::path::Path;

#[cfg(feature = "composition-test")]
pub(crate) async fn authored_float_snapshot(h: &Harness) -> Option<Value> {
    if std::env::var_os("ONECAD_FRAMEWORK_JSON_BIT_AUDIT").is_none() {
        return None;
    }
    let mut guard = h.state.runtime.lock().await;
    let payload = guard.as_mut().unwrap().build_save_payload(
        onecad_core::io::container::SaveMeta::default(),
        onecad_app::document_runtime::SaveCaches::none(),
    );
    Some(serde_json::to_value(payload.diagnostic_document()).unwrap())
}

#[cfg(not(feature = "composition-test"))]
pub(crate) async fn authored_float_snapshot(_h: &Harness) -> Option<Value> {
    assert!(
        std::env::var_os("ONECAD_FRAMEWORK_JSON_BIT_AUDIT").is_none(),
        "audit requires composition-test feature"
    );
    None
}

pub(crate) fn audit_saved_floats(before: Option<Value>, path: &Path) {
    let Some(before) = before else { return };
    let saved = onecad_core::io::container::ContainerReader::open(path).unwrap();
    let after = serde_json::to_value(saved.document()).unwrap();
    let mut counts = (0, 0);
    compare(&before, &after, "$", &mut counts);
    eprintln!(
        "Framework authored Document f64 values={}, changed={}",
        counts.0, counts.1
    );
    assert!(counts.0 > 0, "no authored f64 values audited");
    assert_eq!(counts.1, 0, "saved Document changed authored f64 bits");
}

fn compare(a: &Value, b: &Value, path: &str, counts: &mut (usize, usize)) {
    match (a, b) {
        (Value::Number(a), b) if a.is_f64() => {
            counts.0 += 1;
            let (x, y) = (
                a.as_f64().unwrap(),
                b.as_f64().expect("saved f64 changed type"),
            );
            if x.to_bits() != y.to_bits() {
                counts.1 += 1;
                if counts.1 <= 12 {
                    eprintln!(
                        "f64 mismatch {path}: {x:?} {:016x} -> {y:?} {:016x}",
                        x.to_bits(),
                        y.to_bits()
                    );
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "array shape changed at {path}");
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &format!("{path}[{index}]"), counts);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len(), "object shape changed at {path}");
            for (key, a) in a {
                compare(a, b.get(key).unwrap(), &format!("{path}.{key}"), counts);
            }
        }
        _ => {}
    }
}
