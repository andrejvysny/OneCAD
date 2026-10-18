use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SketchConflictReason {
    RankConflict,
    ResidualViolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SketchResidualDimension {
    #[serde(rename = "mm")]
    Length,
    #[serde(rename = "rad")]
    Angle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchResidualViolationDto {
    pub constraint_id: String,
    pub dimension: SketchResidualDimension,
    pub residual: f64,
    pub tolerance: f64,
    pub normalized: f64,
    #[serde(default)]
    pub component: String,
}

/// Exact retained-pose evidence. Legacy workers omit these additive fields.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchTruthDto {
    #[serde(default)]
    pub reasons: BTreeMap<String, SketchConflictReason>,
    #[serde(default)]
    pub unassessed: Vec<String>,
    #[serde(default)]
    pub violations: Vec<SketchResidualViolationDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_max_residual: Option<f64>,
    #[serde(default, skip_deserializing)]
    pub diagnostics: Vec<onecad_core::regen::Diagnostic>,
}
