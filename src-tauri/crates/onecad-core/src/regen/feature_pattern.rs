//! Effective lowering and fail-closed validation for `FeaturePattern`.
//!
//! Persisted records contain only source record identities. Regen copies the
//! current, already variable-substituted source operations into a wire-only
//! `sourceOps` block on the effective pattern record. Stored history is never
//! mutated, so editing a source remains authoritative.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{json, Value};

use crate::document::record::{KnownOperation, Operation, OperationRecord};
use crate::ids::{BodyId, RecordId};

pub const FEATURE_PATTERN_SEMANTICS_VERSION: u8 = 1;
pub const FEATURE_PATTERN_MIN_COUNT: u32 = 2;
pub const FEATURE_PATTERN_MAX_COUNT: u32 = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeaturePatternError {
    pub step_index: usize,
    pub record_id: RecordId,
    pub message: String,
    pub disposition: FeaturePatternFailureDisposition,
    pub source_record_id: Option<RecordId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeaturePatternFailureDisposition {
    InvalidCommand,
    NeedsRepair,
}

#[derive(Debug)]
struct LowerFailure {
    message: String,
    disposition: FeaturePatternFailureDisposition,
    source_record_id: Option<RecordId>,
}

impl LowerFailure {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            disposition: FeaturePatternFailureDisposition::InvalidCommand,
            source_record_id: None,
        }
    }
    fn repair(source: RecordId, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            disposition: FeaturePatternFailureDisposition::NeedsRepair,
            source_record_id: Some(source),
        }
    }
}

/// Lower every applied, unsuppressed pattern in place. Invalid patterns retain
/// their persisted shape and are returned by step; callers must mark those
/// steps Error and stop planning before the first one.
pub fn lower_feature_patterns(
    records: &mut [OperationRecord],
    applied: usize,
) -> BTreeMap<usize, FeaturePatternError> {
    // `sourceOps` is reserved, derived wire context. Never trust a value loaded
    // through the forward-compatible `extra` map.
    for record in records.iter_mut() {
        if let Operation::Known(KnownOperation::FeaturePattern(params)) = &mut record.op {
            params.extra.remove("sourceOps");
        }
    }
    let snapshot = records.to_vec();
    let positions: HashMap<RecordId, usize> = snapshot
        .iter()
        .enumerate()
        .map(|(index, record)| (record.record_id, index))
        .collect();
    let mut errors = BTreeMap::new();
    for step in 0..applied.min(records.len()) {
        if records[step].suppressed {
            continue;
        }
        let Operation::Known(KnownOperation::FeaturePattern(params)) = &records[step].op else {
            continue;
        };
        match lower_one(step, params, &snapshot, &positions, applied) {
            Ok(source_ops) => {
                let Operation::Known(KnownOperation::FeaturePattern(params)) =
                    &mut records[step].op
                else {
                    unreachable!();
                };
                params
                    .extra
                    .insert("sourceOps".into(), Value::Array(source_ops));
            }
            Err(failure) => {
                errors.insert(
                    step,
                    FeaturePatternError {
                        step_index: step,
                        record_id: records[step].record_id,
                        message: failure.message,
                        disposition: failure.disposition,
                        source_record_id: failure.source_record_id,
                    },
                );
            }
        }
    }
    errors
}

fn lower_one(
    pattern_step: usize,
    params: &crate::document::record::FeaturePatternParams,
    records: &[OperationRecord],
    positions: &HashMap<RecordId, usize>,
    applied: usize,
) -> Result<Vec<Value>, LowerFailure> {
    validate_header(params).map_err(LowerFailure::invalid)?;
    let mut previous = None;
    let mut selected = Vec::with_capacity(params.source_record_ids.len());
    for id in &params.source_record_ids {
        let Some(&step) = positions.get(id) else {
            return Err(LowerFailure::repair(
                *id,
                format!("FeaturePattern source record not found: {id}"),
            ));
        };
        if step >= pattern_step || step >= applied {
            return Err(LowerFailure::repair(
                *id,
                format!("FeaturePattern source {id} is not an earlier applied feature"),
            ));
        }
        if previous.is_some_and(|p| step <= p) {
            return Err(LowerFailure::invalid(
                "FeaturePattern sourceRecordIds must be unique and in timeline order",
            ));
        }
        previous = Some(step);
        selected.push(&records[step]);
    }
    for record in &selected {
        if record.suppressed {
            return Err(LowerFailure::repair(
                record.record_id,
                format!("FeaturePattern source {} is suppressed", record.record_id),
            ));
        }
        if matches!(record.op, Operation::Opaque(_)) {
            return Err(LowerFailure::invalid(format!(
                "FeaturePattern executable adapter does not support source {} ({})",
                record.record_id,
                operation_name(&record.op)
            )));
        }
    }
    validate_lineage(&selected).map_err(LowerFailure::invalid)?;
    validate_executable_chain(&selected).map_err(LowerFailure::invalid)?;
    selected
        .into_iter()
        .map(|record| {
            let value = serde_json::to_value(&record.op)
                .map_err(|error| LowerFailure::invalid(format!("FeaturePattern source serialization failed: {error}")))?;
            let object = value.as_object().ok_or_else(|| LowerFailure::invalid("FeaturePattern source op is not an object"))?;
            let mut inputs = record.op.derive_inputs();
            merge_inputs(&mut inputs, &record.inputs);
            Ok(json!({
                "sourceRecordId": record.record_id,
                "opType": object.get("opType").cloned().unwrap_or(Value::Null),
                "params": object.get("params").cloned().unwrap_or(Value::Object(Default::default())),
                "inputs": inputs,
                "determinism": record.determinism,
            }))
        })
        .collect()
}

fn merge_inputs(
    target: &mut crate::document::record::OperationInputs,
    stored: &crate::document::record::OperationInputs,
) {
    for id in &stored.records {
        if !target.records.contains(id) {
            target.records.push(*id);
        }
    }
    for id in &stored.bodies {
        if !target.bodies.contains(id) {
            target.bodies.push(*id);
        }
    }
    for id in &stored.sketches {
        if !target.sketches.contains(id) {
            target.sketches.push(*id);
        }
    }
    for id in &stored.elements {
        if !target.elements.contains(id) {
            target.elements.push(id.clone());
        }
    }
}

fn validate_header(params: &crate::document::record::FeaturePatternParams) -> Result<(), String> {
    if params.semantics_version != FEATURE_PATTERN_SEMANTICS_VERSION {
        return Err(format!(
            "unsupported FeaturePattern semanticsVersion {}",
            params.semantics_version
        ));
    }
    if !(FEATURE_PATTERN_MIN_COUNT..=FEATURE_PATTERN_MAX_COUNT).contains(&params.count) {
        return Err(format!("FeaturePattern count must be in [{FEATURE_PATTERN_MIN_COUNT},{FEATURE_PATTERN_MAX_COUNT}]"));
    }
    if params.source_record_ids.is_empty() {
        return Err("FeaturePattern requires at least one source record".into());
    }
    let norm2 = |v: &crate::math::Vec3| v.x * v.x + v.y * v.y + v.z * v.z;
    let finite3 = |v: &crate::math::Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
    match &params.layout {
        crate::document::record::FeaturePatternLayout::Linear { direction, spacing } => {
            if !finite3(direction)
                || norm2(direction) <= 1.0e-20
                || !spacing.value.is_finite()
                || spacing.value.abs() < 1.0e-9
            {
                return Err(
                    "FeaturePattern Linear requires nonzero finite direction and spacing".into(),
                );
            }
        }
        crate::document::record::FeaturePatternLayout::Circular {
            axis_origin,
            axis_direction,
            angle_deg,
        } => {
            if !axis_origin.x.is_finite()
                || !axis_origin.y.is_finite()
                || !axis_origin.z.is_finite()
                || !finite3(axis_direction)
                || norm2(axis_direction) <= 1.0e-20
                || !angle_deg.value.is_finite()
                || angle_deg.value == 0.0
                || angle_deg.value.abs() > 360.0
            {
                return Err("FeaturePattern Circular requires finite origin and nonzero finite axis and angle".into());
            }
        }
    }
    Ok(())
}

fn validate_executable_chain(selected: &[&OperationRecord]) -> Result<(), String> {
    let types: Vec<&str> = selected.iter().map(|record| record.op.op_type()).collect();
    if matches!(types.first(), Some(&"Hole")) && types[1..].iter().all(|kind| *kind == "Chamfer") {
        return Ok(());
    }
    if types.len() >= 2
        && types[0] == "Sketch"
        && types[1] == "Extrude"
        && types[2..]
            .iter()
            .all(|kind| matches!(*kind, "Hole" | "Fillet" | "Chamfer"))
    {
        let Operation::Known(KnownOperation::Sketch(sketch)) = &selected[0].op else {
            unreachable!();
        };
        let Operation::Known(KnownOperation::Extrude(extrude)) = &selected[1].op else {
            unreachable!();
        };
        if extrude.profile.as_ref().map(|profile| profile.sketch) != Some(sketch.sketch) {
            return Err(format!(
                "FeaturePattern source {} Extrude must reference selected Sketch {}",
                selected[1].record_id, selected[0].record_id
            ));
        }
        let independent = extrude.boolean_mode == crate::document::record::BooleanMode::NewBody
            && extrude.target_body.is_none();
        let shared_host = matches!(
            extrude.boolean_mode,
            crate::document::record::BooleanMode::Add | crate::document::record::BooleanMode::Cut
        ) && extrude.target_body.is_some();
        if extrude.mode != crate::document::record::ExtrudeMode::Blind
            || extrude.two_directions
            || (!independent && !shared_host)
        {
            return Err(format!(
                "FeaturePattern source {} supports only one-direction Blind NewBody Extrude or the bounded single-host Add/Cut chain",
                selected[1].record_id
            ));
        }
        return Ok(());
    }
    if let Some(revolve_index) = revolve_creator_index(&types) {
        validate_revolve_chain(selected, revolve_index)?;
        return Ok(());
    }
    let unsupported_index = match types.first().copied() {
        Some("Sketch") => unsupported_sketch_chain_index(&types),
        Some("Hole") => types[1..]
            .iter()
            .position(|kind| *kind != "Chamfer")
            .map_or(0, |index| index + 1),
        _ => 0,
    };
    let unsupported = selected[unsupported_index];
    Err(format!(
        "FeaturePattern executable adapter does not support source {} ({}) in this chain",
        unsupported.record_id,
        operation_name(&unsupported.op)
    ))
}

fn unsupported_sketch_chain_index(types: &[&str]) -> usize {
    let sketches = types.iter().take_while(|kind| **kind == "Sketch").count();
    if sketches > 2 {
        return 2;
    }
    if sketches == types.len() {
        return sketches.saturating_sub(1);
    }
    let creator = types[sketches];
    if (sketches == 1 && creator == "Extrude") || creator == "Revolve" {
        let extrude = creator == "Extrude";
        return types[sketches + 1..]
            .iter()
            .position(|kind| {
                !matches!(*kind, "Fillet" | "Chamfer") && !(extrude && *kind == "Hole")
            })
            .map_or(sketches, |index| index + sketches + 1);
    }
    sketches
}

fn revolve_creator_index(types: &[&str]) -> Option<usize> {
    let sketches = types.iter().take_while(|kind| **kind == "Sketch").count();
    (matches!(sketches, 1 | 2)
        && types.get(sketches) == Some(&"Revolve")
        && types[sketches + 1..]
            .iter()
            .all(|kind| matches!(*kind, "Fillet" | "Chamfer")))
    .then_some(sketches)
}

fn validate_revolve_chain(
    selected: &[&OperationRecord],
    revolve_index: usize,
) -> Result<(), String> {
    let Operation::Known(KnownOperation::Revolve(revolve)) = &selected[revolve_index].op else {
        unreachable!();
    };
    let Some(profile) = &revolve.profile else {
        return Err(format!(
            "FeaturePattern source {} Revolve requires an explicit profile",
            selected[revolve_index].record_id
        ));
    };
    let Some(crate::document::refs::AxisRef::SketchLine { sketch: axis, .. }) = &revolve.axis
    else {
        return Err(format!(
            "FeaturePattern source {} supports only a selected sketch-line Revolve axis",
            selected[revolve_index].record_id
        ));
    };
    if revolve.boolean_mode != crate::document::record::BooleanMode::NewBody
        || revolve.target_body.is_some()
    {
        return Err(format!(
            "FeaturePattern source {} supports only NewBody Revolve",
            selected[revolve_index].record_id
        ));
    }
    let selected_sketches: BTreeSet<_> = selected[..revolve_index]
        .iter()
        .filter_map(|record| match &record.op {
            Operation::Known(KnownOperation::Sketch(sketch)) => Some(sketch.sketch),
            _ => None,
        })
        .collect();
    let required: BTreeSet<_> = [profile.sketch, *axis].into_iter().collect();
    if selected_sketches != required || selected_sketches.len() != revolve_index {
        return Err(format!(
            "FeaturePattern source {} Revolve must directly consume every selected Sketch",
            selected[revolve_index].record_id
        ));
    }
    Ok(())
}

fn operation_name(op: &Operation) -> String {
    serde_json::to_value(op)
        .ok()
        .and_then(|value| {
            value
                .get("opType")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Unknown".into())
}

/// Enforce one final host lineage. Bodies produced earlier in the selected chain
/// are internal temporaries; every other body input must be the same shared host.
fn validate_lineage(selected: &[&OperationRecord]) -> Result<(), String> {
    let mut internal = BTreeSet::<BodyId>::new();
    let mut external = BTreeSet::<BodyId>::new();
    let mut internal_sketches = BTreeSet::new();
    let mut lineage = BTreeSet::<BodyId>::new();
    let mut saw_geometry = false;
    for record in selected {
        let inputs = record.op.derive_inputs();
        let body_connected = inputs.bodies.iter().any(|body| lineage.contains(body));
        let sketch_connected = inputs
            .sketches
            .iter()
            .any(|sketch| internal_sketches.contains(sketch));
        if saw_geometry && !body_connected && !sketch_connected && !record.outputs.is_empty() {
            return Err(format!(
                "FeaturePattern source {} is disconnected from the selected lineage",
                record.record_id
            ));
        }
        if let Operation::Known(KnownOperation::Boolean(params)) = &record.op {
            if !internal.contains(&params.tool_body) {
                return Err(format!(
                    "FeaturePattern Boolean source {} consumes an external tool body",
                    record.record_id
                ));
            }
        }
        for body in &inputs.bodies {
            if !internal.contains(body) {
                external.insert(*body);
            }
        }
        internal.extend(record.outputs.iter().copied());
        lineage.extend(inputs.bodies.iter().copied());
        lineage.extend(record.outputs.iter().copied());
        if let Operation::Known(KnownOperation::Sketch(params)) = &record.op {
            internal_sketches.insert(params.sketch);
        }
        saw_geometry |= !record.outputs.is_empty() || !inputs.bodies.is_empty();
    }
    if external.len() > 1 {
        return Err("FeaturePattern source chain consumes more than one external body".into());
    }
    if !saw_geometry {
        return Err("FeaturePattern source chain produces no body lineage".into());
    }
    Ok(())
}

include!("feature_pattern_tests.rs");
