use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn summarize(samples: &[Value]) -> Value {
    let mut summary = serde_json::Map::new();
    for member in [
        "geometry",
        "branch",
        "maxResidual",
        "normalizedMaxResidual",
        "dof",
    ] {
        let unique: BTreeSet<_> = samples.iter().map(|s| s[member].to_string()).collect();
        let mut ranges = BTreeMap::new();
        for sample in samples {
            measure(&sample[member], member, &mut ranges);
        }
        let changed: BTreeMap<_, _> = ranges
            .into_iter()
            .filter(|(_, (min, max))| min != max)
            .map(|(path, (min, max))| (path, json!({ "min": min, "max": max, "delta": max - min })))
            .collect();
        let maximum = changed
            .values()
            .map(|range| range["delta"].as_f64().unwrap())
            .fold(0.0_f64, f64::max);
        summary.insert(member.into(), json!({ "distinctExactValues": unique.len(), "maximumNumericDelta": maximum, "changedNumericFields": changed }));
    }
    Value::Object(summary)
}

fn measure(value: &Value, path: &str, ranges: &mut BTreeMap<String, (f64, f64)>) {
    match value {
        Value::Number(number) => {
            let number = number.as_f64().expect("finite numeric evidence");
            let range = ranges.entry(path.into()).or_insert((number, number));
            range.0 = range.0.min(number);
            range.1 = range.1.max(number);
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                measure(child, &format!("{path}[{index}]"), ranges);
            }
        }
        Value::Object(values) => {
            for (key, child) in values {
                measure(child, &format!("{path}.{key}"), ranges);
            }
        }
        _ => {}
    }
}
