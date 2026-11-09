use serde_json::{json, Value};
use std::collections::BTreeMap;

fn distribution(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    let quantile = |p: f64| values[((values.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    json!({"count":values.len(),"p50":quantile(0.50),"p95":quantile(0.95),"p99":quantile(0.99),
        "minimum":values[0],"maximum":values[values.len()-1],"method":"nearest rank"})
}

pub(super) fn summarize(records: &[Value]) -> Value {
    let mut groups = BTreeMap::<String, Vec<&Value>>::new();
    for record in records
        .iter()
        .filter(|r| r["record"] == "result" && r["accepted"] == true)
    {
        if !matches!(record["phase"].as_str(), Some("warm" | "cold")) {
            continue;
        }
        let key = format!(
            "batch{}:{}:{}",
            record["batch"],
            record["phase"].as_str().unwrap(),
            record["case"].as_str().unwrap()
        );
        groups.entry(key).or_default().push(record);
    }
    let mut result = BTreeMap::new();
    for (key, records) in groups {
        let mut metrics = BTreeMap::new();
        for metric in ["wallMs", "plannerMs", "workerMs", "meshMs"] {
            let values = records
                .iter()
                .filter_map(|r| r[metric].as_f64())
                .collect::<Vec<_>>();
            if !values.is_empty() {
                metrics.insert(metric, distribution(values));
            }
        }
        result.insert(key, json!({"samples":records.len(),"timingMs":metrics}));
    }
    json!(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_and_failed_warmup_exclusion_are_explicit() {
        let summary = distribution((1..=100).map(f64::from).collect());
        assert_eq!(summary["p50"], 50.0);
        assert_eq!(summary["p95"], 95.0);
        assert_eq!(summary["p99"], 99.0);
        let excluded = json!({"record":"result","accepted":true,"phase":"warmup"});
        let failed = json!({"record":"result","accepted":false,"phase":"warm"});
        assert_eq!(summarize(&[excluded, failed]), json!({}));
    }
}
