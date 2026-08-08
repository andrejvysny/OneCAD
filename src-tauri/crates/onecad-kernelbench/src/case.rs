use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{AppError, AppResult, SCHEMA_VERSION};

const MAX_CASE_BYTES: u64 = 1_048_576;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Case {
    pub schema_version: u32,
    pub case_id: String,
    pub generator: Generator,
    pub geometry_recipe: GeometryRecipe,
    pub geometry_tags: Vec<String>,
    pub operation: FilletOperation,
    pub expected_domain: ExpectedDomain,
    pub validators: Vec<Validator>,
    pub metamorphs: Vec<Metamorph>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchSpec>,
    pub limits: Limits,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Generator {
    pub name: String,
    pub version: u32,
    pub seed: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeometryRecipe {
    #[serde(rename = "type")]
    pub recipe_type: GeometryType,
    pub dimensions: Vec<f64>,
    pub feature_index: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GeometryType {
    Box,
    #[serde(rename = "valence3Corner")]
    Valence3,
    #[serde(rename = "valence4Corner")]
    Valence4,
    OverflowWedge,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilletOperation {
    #[serde(rename = "type")]
    pub operation_type: OperationType,
    pub radius: f64,
    pub continuity: Continuity,
    pub radius_mode: RadiusMode,
    pub selector: Selector,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationType {
    Fillet,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Continuity {
    G1,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RadiusMode {
    Constant,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selector {
    pub mode: SelectorMode,
    pub topology_role: TopologyRole,
    pub provenance: SelectorProvenance,
    pub anchors: Vec<SelectorAnchor>,
    pub surface_descriptors: Vec<SurfaceDescriptor>,
    pub adjacency: SelectorAdjacency,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectorMode {
    Single,
    Disconnected,
    Multiple,
    CornerIncident,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TopologyRole {
    VerticalEdge,
    CornerIncidentEdge,
    OverflowEdge,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectorProvenance {
    pub generator: String,
    pub recipe_type: GeometryType,
    pub feature_index: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectorAnchor {
    pub kind: AnchorKind,
    pub point: [f64; 3],
    pub frame: AnchorFrame,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AnchorKind {
    EdgeMidpoint,
    CommonVertex,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AnchorFrame {
    RecipeLocal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SurfaceDescriptor {
    pub curve_kind: CurveKind,
    pub adjacent_surface_kinds: Vec<SurfaceKind>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CurveKind {
    Line,
    AnalyticCurve,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SurfaceKind {
    Plane,
    Cylinder,
    Cone,
    Other,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectorAdjacency {
    pub relation: AdjacencyRelation,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AdjacencyRelation {
    Single,
    PairwiseDisconnected,
    Unconstrained,
    IncidentToCommonVertex,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ExpectedDomain {
    Supported,
    ExpectedLimit,
    Exploratory,
    OutsideProductDomain,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Validator {
    #[serde(rename = "type")]
    pub validator_type: ValidatorType,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<MaterialDirection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absolute: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_samples: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point_tolerance: Option<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidatorType {
    ConstantRadius,
    GeneratedBlendFace,
    CylindricalRadius,
    G1BoundaryTangency,
    MaterialChange,
    RemoteSupportsUnchanged,
    DeepAudit,
    RadiusTolerance,
    TangencyTolerance,
    MaterialTolerance,
    MetamorphicEquivalence,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MaterialDirection {
    Decrease,
    Increase,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Metamorph {
    Translation {
        vector: [f64; 3],
    },
    Rotation {
        #[serde(rename = "angleDegrees")]
        angle_degrees: f64,
        axis: [f64; 3],
        center: RotationCenter,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RotationCenter {
    InputCentroid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchSpec {
    pub parameter: SearchParameter,
    pub known_success: f64,
    pub upper_bound: f64,
    pub growth_factor: f64,
    pub max_probes: u32,
    pub relative_precision: f64,
    pub absolute_precision: f64,
    pub relative_offsets: [f64; 4],
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum SearchParameter {
    #[serde(rename = "operation.radius")]
    OperationRadius,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Limits {
    pub resources: ResourceLimits,
    pub quality: QualityLimits,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceLimits {
    pub wall_time_ms: u64,
    pub address_space_bytes: u64,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub artifact_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualityLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_vertex_tolerance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_edge_tolerance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_face_tolerance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_micro_edges: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_sliver_faces: Option<u64>,
}

impl Case {
    pub fn load(path: &Path) -> AppResult<Self> {
        let meta = std::fs::metadata(path).map_err(|e| AppError::cli(format!("case: {e}")))?;
        if meta.len() > MAX_CASE_BYTES {
            return Err(AppError::cli("case exceeds 1 MiB"));
        }
        let bytes = std::fs::read(path).map_err(|e| AppError::cli(format!("case: {e}")))?;
        let case: Self = serde_json::from_slice(&bytes)
            .map_err(|e| AppError::cli(format!("case schema: {e}")))?;
        case.validate()?;
        Ok(case)
    }

    pub fn validate(&self) -> AppResult<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(AppError::cli("unsupported case schemaVersion"));
        }
        if self.generator.name != "fillet-foundation" || self.generator.version != 1 {
            return Err(AppError::cli("unsupported generator"));
        }
        validate_id(&self.case_id, 128)?;
        if !is_seed(&self.generator.seed) {
            return Err(AppError::cli("seed must be 16 lowercase hex digits"));
        }
        crate::case_validation::validate(self)
    }
}

fn validate_id(value: &str, max: usize) -> AppResult<()> {
    let safe = !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b));
    if safe && value != "." && value != ".." {
        Ok(())
    } else {
        Err(AppError::cli("unsafe identifier"))
    }
}

fn is_seed(seed: &str) -> bool {
    seed.len() == 16
        && seed
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_exact_lower_hex() {
        assert!(is_seed("0123456789abcdef"));
        assert!(!is_seed("0123456789ABCDEF"));
        assert!(!is_seed("1234"));
    }

    #[test]
    fn identifier_rejects_paths() {
        assert!(validate_id("case-01", 20).is_ok());
        assert!(validate_id("../case", 20).is_err());
        assert!(validate_id("Uppercase", 20).is_err());
    }

    #[test]
    fn generated_suite_satisfies_strict_case_validation() {
        for generated in crate::suite::t0() {
            generated.case.validate().unwrap();
        }
    }

    #[test]
    fn unknown_operation_field_is_rejected() {
        let text = include_str!(
            "../../../../bench/robustness/regressions/fillet-box-supported-single.json"
        );
        let mut value: serde_json::Value = serde_json::from_str(text).unwrap();
        value["operation"]["unknown"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Case>(value).is_err());
    }
}
