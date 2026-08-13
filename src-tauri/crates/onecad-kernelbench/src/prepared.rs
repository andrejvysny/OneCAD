//! The version-agnostic view of a case that the supervisor actually executes.
//!
//! The supervisor never needs the whole case: it needs the identity it stamps
//! into every result, the resource ceilings it enforces on the child, the
//! metamorph tolerance it compares variants with, and the canonical document to
//! put on the wire. Everything else is the runner's business.
//!
//! Routing both case versions through one struct is what keeps `campaign`,
//! `runner`, `child`, and `result_validation` from growing a `match` on the
//! schema version — the place a v2-only bug would hide.

use serde_json::Value;

use crate::case::{Case, ExpectedDomain, Generator, Limits, ValidatorType};
use crate::case_v2::{CaseV2, MetamorphV2, ValidatorTypeV2};

const DEFAULT_POINT_TOLERANCE: f64 = 1e-6;

#[derive(Clone, Debug)]
pub struct PreparedCase {
    pub case_id: String,
    /// Always the v1 `{name, version, seed}` shape: the RESULT schema is frozen
    /// at v1, so a v2 case reports the same three fields and drops `family`.
    pub generator: Generator,
    pub expected_domain: ExpectedDomain,
    pub limits: Limits,
    /// Tolerance the campaign-level metamorph comparison uses. The runner always
    /// reports `notRun` for metamorphic equivalence — it has no cross-variant
    /// context — so this is read on the supervisor side.
    pub point_tolerance: f64,
    /// The largest radius the operation requests. The `parameterEpsilon` variant
    /// nudges exactly this value, so the expected geometric response is bounded
    /// by it — see `campaign::Relation::Continuity`.
    pub max_radius: f64,
    /// The canonical case document, exactly as it goes on the wire.
    pub json: Value,
    /// Executable metamorphs for this case (empty for v1 cases).
    pub metamorphs: Vec<MetamorphV2>,
}

impl Case {
    pub fn prepared(&self) -> PreparedCase {
        PreparedCase {
            case_id: self.case_id.clone(),
            generator: self.generator.clone(),
            expected_domain: self.expected_domain,
            limits: self.limits.clone(),
            point_tolerance: self
                .validators
                .iter()
                .find(|validator| {
                    matches!(
                        validator.validator_type,
                        ValidatorType::MetamorphicEquivalence
                    )
                })
                .and_then(|validator| validator.point_tolerance)
                .unwrap_or(DEFAULT_POINT_TOLERANCE),
            max_radius: self.operation.radius,
            json: serde_json::to_value(self).unwrap_or(Value::Null),
            metamorphs: Vec::new(),
        }
    }
}

impl CaseV2 {
    pub fn prepared(&self) -> PreparedCase {
        PreparedCase {
            case_id: self.case_id.clone(),
            generator: Generator {
                name: generator_name(self),
                version: self.generator.version,
                seed: self.generator.seed.clone(),
            },
            expected_domain: self.expected_domain,
            limits: self.limits.clone(),
            point_tolerance: self
                .validators
                .iter()
                .find(|validator| {
                    matches!(
                        validator.validator_type,
                        ValidatorTypeV2::MetamorphicEquivalence
                    )
                })
                .and_then(|validator| validator.point_tolerance)
                .unwrap_or(DEFAULT_POINT_TOLERANCE),
            max_radius: self.operation.definition.radius_law.max_radius(),
            json: serde_json::to_value(self).unwrap_or(Value::Null),
            metamorphs: self.metamorphs.clone(),
        }
    }
}

/// The generator name as the wire spells it, taken from the enum's own serde
/// rendering so the two can never drift.
fn generator_name(case: &CaseV2) -> String {
    serde_json::to_value(case.generator.name)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str =
        include_str!("../../../../bench/robustness/examples/fillet-matrix-plane-cylinder-v2.json");
    const V1_REGRESSION: &str =
        include_str!("../../../../bench/robustness/regressions/fillet-box-supported-single.json");

    #[test]
    fn a_v2_case_reports_the_v1_generator_shape() {
        let case: CaseV2 = serde_json::from_str(EXAMPLE).unwrap();
        let prepared = case.prepared();
        assert_eq!(prepared.generator.name, "fillet-matrix");
        // `family` is a v2-only axis and must not leak into a v1 result.
        let generator = serde_json::to_value(&prepared.generator).unwrap();
        assert!(generator.get("family").is_none());
        assert_eq!(prepared.json["schemaVersion"], 2);
    }

    #[test]
    fn both_versions_expose_their_metamorph_tolerance() {
        let v2: CaseV2 = serde_json::from_str(EXAMPLE).unwrap();
        assert_eq!(v2.prepared().point_tolerance, 1e-6);
        let v1: Case = serde_json::from_str(V1_REGRESSION).unwrap();
        // The v1 regression carries no metamorphicEquivalence validator, so the
        // documented default applies rather than a silent zero.
        assert_eq!(v1.prepared().point_tolerance, DEFAULT_POINT_TOLERANCE);
    }
}
