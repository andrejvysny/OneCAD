use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

use crate::campaign::{rebase_artifacts, write_jsonl};
use crate::case::{Case, SearchSpec};
use crate::result::{is_red, refresh_digest, Backend};
use crate::runner;
use crate::suite::{Variant, VariantName};
use crate::AppResult;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Classification {
    Success,
    Refusal,
    Invalid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub lower: f64,
    pub upper: f64,
    pub from: Classification,
    pub to: Classification,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOutcome {
    pub transitions: Vec<Transition>,
    pub monotonic_observed: bool,
    pub probes: usize,
}

pub trait Executor {
    fn classify(&mut self, radius: f64) -> Classification;
}

pub fn explore(executor: &mut impl Executor, spec: &SearchSpec) -> SearchOutcome {
    let mut observations = BTreeMap::new();
    sample_growth(executor, spec, &mut observations);
    sweep_brackets(executor, spec, &mut observations);
    subdivide_transitions(executor, spec, &mut observations);
    probe_offsets(executor, spec, &mut observations);
    let transitions = transitions(&observations);
    let monotonic_observed = monotonic(&observations);
    SearchOutcome {
        transitions,
        monotonic_observed,
        probes: observations.len(),
    }
}

fn sweep_brackets(
    executor: &mut impl Executor,
    spec: &SearchSpec,
    values: &mut BTreeMap<u64, Classification>,
) {
    let brackets: Vec<_> = values
        .keys()
        .copied()
        .collect::<Vec<_>>()
        .windows(2)
        .map(|pair| (f64::from_bits(pair[0]), f64::from_bits(pair[1])))
        .collect();
    for (lower, upper) in brackets {
        for step in 1..8 {
            if values.len() >= spec.max_probes as usize {
                return;
            }
            observe(
                executor,
                values,
                lower + (upper - lower) * step as f64 / 8.0,
            );
        }
    }
}

fn sample_growth(
    executor: &mut impl Executor,
    spec: &SearchSpec,
    values: &mut BTreeMap<u64, Classification>,
) {
    let mut radius = spec.known_success;
    while values.len() < spec.max_probes as usize {
        observe(executor, values, radius);
        if radius >= spec.upper_bound {
            break;
        }
        radius = (radius * spec.growth_factor).min(spec.upper_bound);
    }
}

fn subdivide_transitions(
    executor: &mut impl Executor,
    spec: &SearchSpec,
    values: &mut BTreeMap<u64, Classification>,
) {
    while let Some((lower, upper)) = widest_interval(values, spec) {
        let offset_reserve = transitions(values).len().saturating_mul(8);
        if values.len().saturating_add(offset_reserve) >= spec.max_probes as usize {
            break;
        }
        observe(executor, values, lower + (upper - lower) * 0.5);
    }
}

fn widest_interval(
    values: &BTreeMap<u64, Classification>,
    spec: &SearchSpec,
) -> Option<(f64, f64)> {
    let points: Vec<_> = values
        .iter()
        .map(|(bits, class)| (f64::from_bits(*bits), *class))
        .collect();
    points
        .windows(2)
        .filter_map(|pair| {
            let (lower, from) = pair[0];
            let (upper, to) = pair[1];
            let precision = spec
                .absolute_precision
                .max(spec.relative_precision * upper.abs());
            (from != to && upper - lower > precision).then_some((lower, upper))
        })
        .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))
}

fn probe_offsets(
    executor: &mut impl Executor,
    spec: &SearchSpec,
    values: &mut BTreeMap<u64, Classification>,
) {
    let current = transitions(values);
    for transition in current {
        let center = (transition.lower + transition.upper) * 0.5;
        for offset in spec.relative_offsets {
            for sign in [-1.0, 1.0] {
                if values.len() >= spec.max_probes as usize {
                    return;
                }
                let radius = center * (1.0 + sign * offset);
                if radius > 0.0 && radius <= spec.upper_bound {
                    observe(executor, values, radius);
                }
            }
        }
    }
}

fn observe(executor: &mut impl Executor, values: &mut BTreeMap<u64, Classification>, radius: f64) {
    values
        .entry(radius.to_bits())
        .or_insert_with(|| executor.classify(radius));
}

fn transitions(values: &BTreeMap<u64, Classification>) -> Vec<Transition> {
    let points: Vec<_> = values
        .iter()
        .map(|(bits, class)| (f64::from_bits(*bits), *class))
        .collect();
    points
        .windows(2)
        .filter_map(|pair| {
            let (lower, from) = pair[0];
            let (upper, to) = pair[1];
            (from != to).then_some(Transition {
                lower,
                upper,
                from,
                to,
            })
        })
        .collect()
}

fn monotonic(values: &BTreeMap<u64, Classification>) -> bool {
    let mut transitions = 0;
    let mut previous = None;
    for class in values.values() {
        if previous.is_some_and(|prior| prior != *class) {
            transitions += 1;
        }
        previous = Some(*class);
    }
    transitions <= 1
}

struct RunnerExecutor<'a> {
    runner: &'a Path,
    case: Case,
    backend: Backend,
    out_dir: &'a Path,
    records: Vec<Value>,
}

impl Executor for RunnerExecutor<'_> {
    fn classify(&mut self, radius: f64) -> Classification {
        self.case.operation.radius = radius;
        let variant = Variant {
            name: VariantName::Base,
            translation: None,
            rotation: None,
        };
        let artifact_dir = self
            .out_dir
            .join("artifacts")
            .join("search")
            .join(&self.case.case_id)
            .join(self.backend.wire())
            .join(format!("{:016x}", radius.to_bits()));
        let _ = std::fs::create_dir_all(&artifact_dir);
        let mut record = runner::execute(
            self.runner,
            &self.case,
            self.backend,
            &variant,
            &artifact_dir,
        );
        rebase_artifacts(&mut record, self.out_dir, &artifact_dir);
        record["probeRadius"] = json!(radius);
        let class = classify_record(&record);
        characterize_safe_refusal(&mut record);
        self.records.push(record);
        class
    }
}

fn characterize_safe_refusal(record: &mut Value) {
    let completed = record.get("executionState").and_then(Value::as_str) == Some("completed");
    let rejected = record.get("operationState").and_then(Value::as_str) == Some("rejected");
    if completed && rejected {
        record["verdict"] = json!("characterization");
    }
}

fn classify_record(value: &Value) -> Classification {
    if value.get("verdict").and_then(Value::as_str) == Some("pass") {
        Classification::Success
    } else if matches!(
        value.get("operationState").and_then(Value::as_str),
        Some("rejected")
    ) {
        Classification::Refusal
    } else {
        Classification::Invalid
    }
}

pub fn run_search(
    runner_path: &Path,
    case: Case,
    backends: &[Backend],
    out_dir: &Path,
) -> AppResult<i32> {
    std::fs::create_dir_all(out_dir)
        .map_err(|e| crate::AppError::environment(format!("out dir: {e}")))?;
    let spec = case
        .search
        .clone()
        .ok_or_else(|| crate::AppError::cli("case has no search block"))?;
    let mut all_records = Vec::new();
    let mut red = false;
    for backend in backends {
        let mut executor = RunnerExecutor {
            runner: runner_path,
            case: case.clone(),
            backend: *backend,
            out_dir,
            records: Vec::new(),
        };
        let outcome = explore(&mut executor, &spec);
        red |= executor.records.iter().any(is_red);
        for record in &mut executor.records {
            let radius = record
                .as_object_mut()
                .and_then(|object| object.remove("probeRadius"))
                .and_then(|value| value.as_f64())
                .unwrap_or(case.operation.radius);
            record["search"] = search_evidence(&outcome, radius);
            refresh_digest(record);
        }
        all_records.extend(executor.records);
        eprintln!(
            "{} probes={} transitions={} monotonic={}",
            backend.wire(),
            outcome.probes,
            outcome.transitions.len(),
            outcome.monotonic_observed
        );
    }
    write_jsonl(&out_dir.join("results.jsonl"), &all_records)?;
    Ok(if red { 1 } else { 0 })
}

fn search_evidence(outcome: &SearchOutcome, probe_radius: f64) -> Value {
    let transitions: Vec<_> = outcome
        .transitions
        .iter()
        .map(|item| {
            json!({
                "lower":item.lower,
                "upper":item.upper,
                "lowerState":transition_state(item.from),
                "upperState":transition_state(item.to)
            })
        })
        .collect();
    json!({
        "parameter":"operation.radius",
        "probeRadius":probe_radius,
        "probeCount":outcome.probes,
        "monotonicObserved":outcome.monotonic_observed,
        "transitions":transitions
    })
}

fn transition_state(classification: Classification) -> &'static str {
    match classification {
        Classification::Success => "success",
        Classification::Refusal => "rejected",
        Classification::Invalid => "invalid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Synthetic;
    impl Executor for Synthetic {
        fn classify(&mut self, radius: f64) -> Classification {
            if radius < 2.0 || (4.0..6.0).contains(&radius) {
                Classification::Success
            } else {
                Classification::Refusal
            }
        }
    }

    struct HiddenIsland;
    impl Executor for HiddenIsland {
        fn classify(&mut self, radius: f64) -> Classification {
            if radius < 2.0 || (2.2..2.4).contains(&radius) {
                Classification::Success
            } else {
                Classification::Refusal
            }
        }
    }

    #[test]
    fn detects_non_monotonic_classification() {
        let spec = SearchSpec {
            parameter: crate::case::SearchParameter::OperationRadius,
            known_success: 1.0,
            upper_bound: 8.0,
            growth_factor: 2.0,
            max_probes: 128,
            relative_precision: 1e-6,
            absolute_precision: 1e-6,
            relative_offsets: [1e-2, 1e-4, 1e-6, 1e-8],
        };
        let result = explore(&mut Synthetic, &spec);
        assert!(!result.monotonic_observed);
        assert!(result.transitions.len() >= 3);
    }

    #[test]
    fn bracket_sweep_finds_hidden_transition_island() {
        let result = explore(&mut HiddenIsland, &spec());
        assert!(!result.monotonic_observed);
        assert!(result.transitions.len() >= 3);
    }

    fn spec() -> SearchSpec {
        SearchSpec {
            parameter: crate::case::SearchParameter::OperationRadius,
            known_success: 1.0,
            upper_bound: 8.0,
            growth_factor: 2.0,
            max_probes: 128,
            relative_precision: 1e-6,
            absolute_precision: 1e-6,
            relative_offsets: [1e-2, 1e-4, 1e-6, 1e-8],
        }
    }
}
