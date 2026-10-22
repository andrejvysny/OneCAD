//! Recheck observed synthetic Pencil transitions, independently of XCTest assertions.
use super::{ios_verify, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

mod scenarios;
use scenarios::{cancellation, chronology, estimation, predictions};

pub(super) const ARTIFACT: &str = "pencil-replay.trace";
const NAMES: [&str; 4] = [
    "chronology-coalesced",
    "prediction-replacement",
    "estimation-refinement",
    "cancellation-late-ended",
];
const COUNTERS: [&str; 9] = [
    "invalid",
    "duplicateCoalesced",
    "stale",
    "unknownContact",
    "oldGeneration",
    "refusedContacts",
    "estimationMatched",
    "estimationUnmatched",
    "mergedMoves",
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Trace {
    schema_version: u64,
    run_id: String,
    synthetic: bool,
    kernel_identity: Value,
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
struct Scenario {
    name: String,
    observations: Vec<Observation>,
}

#[derive(Deserialize)]
struct Observation {
    request: Value,
    response: Value,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct State {
    live_contacts: u64,
    stats: BTreeMap<String, u64>,
    pose: Value,
    prediction: Option<[f64; 2]>,
    pencil_position: Option<[f64; 2]>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct Sample {
    contact: u64,
    kind: String,
    phase: String,
    provenance: String,
    t: f64,
    x: f64,
    y: f64,
    estimation_index: Option<u64>,
    force: Option<f64>,
    max_force: Option<f64>,
    #[serde(default)]
    estimated: Vec<String>,
    #[serde(default)]
    expects_update: Vec<String>,
}

impl Sample {
    fn position(&self) -> [f64; 2] {
        [self.x, self.y]
    }
}

fn check(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(format!("Pencil replay: {message}").into())
    }
}

pub(super) fn verify(value: &Value, run_id: &str, simulator: &Value) -> Result<()> {
    let trace: Trace = serde_json::from_value(value.clone())?;
    check(trace.schema_version == 1, "unsupported schemaVersion")?;
    check(
        trace.run_id == run_id && !run_id.is_empty(),
        "stale or missing runId",
    )?;
    check(trace.synthetic, "synthetic must be true")?;
    ios_verify::verify_kernel(&trace.kernel_identity, simulator)?;
    verify_shape(&trace, simulator)?;
    let contact = trace.scenarios[0].observations[2].request["contact"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or("Pencil replay: missing contact")?;
    let generation = trace.scenarios[0].observations[3].request["generation"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or("Pencil replay: missing generation")?;
    verify_requests(&trace, contact, generation)?;
    verify_documents(&trace)?;
    let actual = chronology(&trace.scenarios[0])?;
    predictions(&trace.scenarios[1], &actual)?;
    estimation(&trace.scenarios[2], &actual)?;
    cancellation(&trace.scenarios[3], &actual)
}

fn verify_shape(trace: &Trace, simulator: &Value) -> Result<()> {
    check(
        trace.scenarios.len() == NAMES.len(),
        "missing or duplicate scenario",
    )?;
    let sequences: [&[&str]; 4] = [
        &[
            "cancel",
            "document_summary",
            "input_state",
            "replay",
            "input_state",
            "document_summary",
        ],
        &[
            "input_state",
            "replay",
            "input_state",
            "replay",
            "input_state",
            "document_summary",
        ],
        &["input_state", "replay", "input_state", "document_summary"],
        &[
            "input_state",
            "cancel",
            "input_state",
            "replay",
            "input_state",
            "document_summary",
        ],
    ];
    for ((scenario, name), sequence) in trace.scenarios.iter().zip(NAMES).zip(sequences) {
        check(
            scenario.name == name,
            "missing, duplicated or reordered scenario",
        )?;
        verify_sequence(scenario, sequence, simulator)?;
    }
    Ok(())
}

fn verify_sequence(scenario: &Scenario, sequence: &[&str], simulator: &Value) -> Result<()> {
    check(
        scenario.observations.len() == sequence.len(),
        "missing or extra observations",
    )?;
    for (observation, command) in scenario.observations.iter().zip(sequence) {
        if *command == "cancel" {
            check(
                observation.request["abi"] == "onecad_ios_input_cancel_all"
                    && observation.request["reason"] == 0
                    && observation.response["status"] == 0,
                "public cancellation failed or missing",
            )?;
            continue;
        }
        check(
            observation.request["cmd"] == *command && observation.response["ok"] == true,
            "unexpected command or failed response",
        )?;
        if *command == "document_summary" {
            check(
                observation.response["pending"] != true,
                "document observation still pending",
            )?;
            ios_verify::verify_kernel(&observation.response["kernelIdentity"], simulator)?;
        } else {
            check(
                observation.response["synthetic"] == true,
                "unmarked synthetic response",
            )?;
        }
    }
    Ok(())
}

fn samples(observation: &Observation) -> Result<Vec<Sample>> {
    let samples: Vec<Sample> = serde_json::from_value(observation.request["samples"].clone())?;
    check(
        !samples.is_empty()
            && observation.response["accepted"] == samples.len()
            && observation.response["overflowed"] == false,
        "empty, dropped or overflowing replay",
    )?;
    Ok(samples)
}

fn state(observation: &Observation) -> Result<State> {
    let state: State = serde_json::from_value(observation.response.clone())?;
    check(
        COUNTERS.iter().all(|key| state.stats.contains_key(*key)),
        "missing input counters",
    )?;
    for key in ["prediction", "pencilPosition"] {
        check(
            observation.response.get(key).is_some(),
            "missing input point",
        )?;
        let point = &observation.response[key];
        check(
            point.is_null()
                || point
                    .as_array()
                    .is_some_and(|p| p.len() == 2 && p.iter().all(Value::is_number)),
            "malformed input point",
        )?;
    }
    Ok(state)
}

fn verify_requests(trace: &Trace, contact: u64, generation: u64) -> Result<()> {
    let mut latest_time = 0.0;
    for scenario in &trace.scenarios {
        for observation in &scenario.observations {
            match observation.request["cmd"].as_str() {
                Some("input_state") => {
                    check(
                        observation.request["contact"] == contact
                            && observation.request["estimationIndex"] == 102,
                        "mismatched pose query",
                    )?;
                    state(observation)?;
                }
                Some("replay") => {
                    check(
                        observation.request["generation"] == generation,
                        "view generation changed",
                    )?;
                    let batch = samples(observation)?;
                    for sample in &batch {
                        check(
                            sample.contact == contact
                                && sample.kind == "pencil"
                                && sample.t > latest_time,
                            "invalid or mismatched Pencil sample",
                        )?;
                    }
                    latest_time = batch
                        .iter()
                        .map(|sample| sample.t)
                        .fold(latest_time, f64::max);
                }
                _ => {}
            }
        }
    }
    for pair in trace.scenarios.windows(2) {
        let previous = pair[0]
            .observations
            .iter()
            .rev()
            .find(|o| o.request["cmd"] == "input_state")
            .ok_or("missing state")?;
        check(
            state(previous)? == state(&pair[1].observations[0])?,
            "scenario state discontinuity",
        )?;
    }
    Ok(())
}

fn verify_documents(trace: &Trace) -> Result<()> {
    let baseline = &trace.scenarios[0].observations[1].response["summary"];
    check(
        baseline["document"]["documentId"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "missing baseline document",
    )?;
    for key in ["features", "sketches", "bodies"] {
        check(baseline[key].is_array(), "missing baseline document arrays")?;
    }
    for scenario in &trace.scenarios {
        let summary = &scenario
            .observations
            .last()
            .ok_or("empty scenario")?
            .response["summary"];
        for key in ["document", "features", "sketches", "bodies"] {
            check(
                summary.get(key) == baseline.get(key),
                "non-authoring replay changed document",
            )?;
        }
    }
    Ok(())
}

fn delta(before: &State, after: &State, key: &str, expected: u64) -> Result<()> {
    check(
        before
            .stats
            .get(key)
            .zip(after.stats.get(key))
            .is_some_and(|(before, after)| before.checked_add(expected) == Some(*after)),
        &format!("{key} counter delta differs"),
    )
}

fn pose(state: &State, force: f64, mask: u64) -> Result<()> {
    check(
        state.pose["force"] == force
            && state.pose["maxForce"] == 1.0
            && state.pose["estimated"] == mask
            && state.pose["expectsUpdate"] == mask,
        "retained real pose differs",
    )
}

#[cfg(test)]
pub(super) mod tests;
