use crate::dto::{SketchConflictReason, SketchResidualDimension, SketchTruthDto, SketchUpsertDto};
use onecad_core::sketch::Sketch;
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn parse(result: &Value) -> Result<SketchTruthDto, String> {
    // Diagnostics use the shared bounded reader rather than serde's unbounded
    // path; unknown additive fields remain compatible.
    let source = result
        .as_object()
        .ok_or("Sketch solve result must be an object")?;
    let fields = [
        "reasons",
        "unassessed",
        "violations",
        "normalizedMaxResidual",
    ]
    .into_iter()
    .filter_map(|key| {
        source
            .get(key)
            .map(|value| (key.to_string(), value.clone()))
    })
    .collect();
    let mut truth: SketchTruthDto = serde_json::from_value(Value::Object(fields))
        .map_err(|e| format!("Sketch solve truth: {e}"))?;
    truth.diagnostics = super::wire::parse_diagnostics(result.get("diagnostics"));
    validate_numbers(&truth)?;
    Ok(truth)
}

fn validate_numbers(truth: &SketchTruthDto) -> Result<(), String> {
    if truth
        .normalized_max_residual
        .is_some_and(|q| !q.is_finite() || q < 0.0)
    {
        return Err("Sketch solve truth: invalid normalized maximum".into());
    }
    for violation in &truth.violations {
        if !violation.residual.is_finite()
            || !violation.tolerance.is_finite()
            || !violation.normalized.is_finite()
            || violation.residual < 0.0
            || violation.tolerance <= 0.0
            || violation.residual <= violation.tolerance
            || violation.normalized <= 1.0
        {
            return Err("Sketch solve truth: invalid typed violation".into());
        }
        let expected = violation.residual / violation.tolerance;
        let rounding = 8.0 * f64::EPSILON * expected.abs().max(1.0);
        if !expected.is_finite()
            || (violation.normalized - expected).abs() > rounding
            || (violation.dimension == SketchResidualDimension::Length
                && violation.tolerance != 1e-4)
            || truth
                .normalized_max_residual
                .is_some_and(|q| q < violation.normalized)
        {
            return Err("Sketch solve truth: inconsistent typed violation evidence".into());
        }
    }
    Ok(())
}

pub(crate) fn validate(sketch: &Sketch, result: &SketchUpsertDto) -> Result<(), String> {
    let truth = &result.truth;
    validate_numbers(truth)?;
    if !truth.unassessed.is_empty() {
        return Err("Sketch solve truth: unresolved constraints cannot be adopted".into());
    }
    let ids: BTreeSet<_> = sketch
        .constraints()
        .iter()
        .map(|c| c.id().to_string())
        .collect();
    for id in truth
        .reasons
        .keys()
        .chain(truth.violations.iter().map(|v| &v.constraint_id))
    {
        if !ids.contains(id) {
            return Err(format!("Sketch solve truth: foreign constraint {id}"));
        }
    }
    for violation in &truth.violations {
        if truth.reasons.get(&violation.constraint_id)
            != Some(&SketchConflictReason::ResidualViolation)
            || !result.conflicting.contains(&violation.constraint_id)
        {
            return Err("Sketch solve truth: violation lacks its named conflict reason".into());
        }
    }
    if !truth.reasons.is_empty()
        && (result.status != crate::dto::SketchSolveStatus::Conflicting
            || truth
                .reasons
                .keys()
                .any(|id| !result.conflicting.contains(id)))
    {
        return Err("Sketch solve truth: conflict evidence disagrees with solve state".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_and_unknown_additions_remain_compatible() {
        assert_eq!(
            parse(&json!({"future":true})).unwrap(),
            SketchTruthDto::default()
        );
        assert_eq!(
            parse(&json!({"normalizedMaxResidual":0}))
                .unwrap()
                .normalized_max_residual,
            Some(0.0)
        );
    }

    #[test]
    fn end_reader_rejects_malformed_truth_without_geometry_alias_validation() {
        for value in [
            json!({"reasons":[]}),
            json!({"unassessed":"x"}),
            json!({"violations":{}}),
            json!({"normalizedMaxResidual":-1}),
            json!({"normalizedMaxResidual":"NaN"}),
            json!({"reasons":{"c":"fabricated"}}),
            json!({"violations":[{"constraintId":"c","dimension":"degrees",
                "residual":1,"tolerance":0.1,"normalized":10}]}),
            json!({"violations":[{"constraintId":"c","dimension":"mm",
                "residual":1,"tolerance":0,"normalized":10}]}),
        ] {
            assert!(
                super::super::wire::parse_sketch_upsert_truth_checked("sketch", &value).is_err(),
                "{value}"
            );
        }
        let legacy_alias = json!({"positions":{"circle.center":[1,2]}});
        assert!(
            super::super::wire::parse_sketch_upsert_truth_checked("sketch", &legacy_alias).is_ok()
        );
    }
}

#[cfg(test)]
mod numeric_tests {
    use super::*;
    use serde_json::json;

    fn value(residual: f64, normalized: f64) -> Value {
        json!({"normalizedMaxResidual":normalized,"violations":[{
            "constraintId":"c","dimension":"mm","residual":residual,
            "tolerance":1e-4,"normalized":normalized}]})
    }

    #[test]
    fn normalized_evidence_accepts_rounding_boundary_and_large_finite_values() {
        for residual in [1.000000000000001e-4, 1e196] {
            let normalized = residual / 1e-4;
            let encoded = serde_json::to_vec(&value(residual, normalized)).unwrap();
            assert!(parse(&serde_json::from_slice(&encoded).unwrap()).is_ok());
        }
        let mut rounded = value(0.001, 10.0);
        rounded["violations"][0]["normalized"] = json!(10.0 - 4.0 * f64::EPSILON * 10.0);
        assert!(parse(&rounded).is_ok());
    }

    #[test]
    fn false_normalization_budget_and_maximum_are_refused() {
        let original = value(0.001, 10.0);
        let mut wrong = original.clone();
        wrong["violations"][0]["normalized"] = json!(2.0);
        assert!(parse(&wrong).is_err());
        wrong = original.clone();
        wrong["violations"][0]["tolerance"] = json!(0.0005);
        wrong["violations"][0]["normalized"] = json!(2.0);
        assert!(parse(&wrong).is_err());
        wrong = original;
        wrong["normalizedMaxResidual"] = json!(9.0);
        assert!(parse(&wrong).is_err());
    }
}
