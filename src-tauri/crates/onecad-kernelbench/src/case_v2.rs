//! Kernelbench case format v2 — `bench/robustness/schemas/case-v2.schema.json`.
//!
//! v1 (`case.rs`) is FROZEN: it is the T0 reproducibility contract and the shape
//! every committed regression is stored in. This module is a separate type set,
//! not an extension of it — nothing here may relax a v1 rule, and a v1 file must
//! never deserialize as v2 or vice versa (`schemaVersion` is a hard const on both
//! sides, checked in [`CaseV2::validate`]).
//!
//! ## What v2 changes and why
//!
//! * **`selector` is top level.** In v1 it hung off the operation. The same
//!   geometry + selection is reused across radius laws and (later) across
//!   operations, and a selector that resolves is a *precondition* for the
//!   operation rather than part of its definition.
//! * **`operation.definition` mirrors the kernel-level `FilletDefinition`** — a
//!   radius law plus a continuity request — instead of a flat `radius` scalar.
//!   Adding `linear` / `controlPoints` is then additive on both sides.
//! * **`geometry` is `{recipe, parameters, tags}`** with parameters strict per
//!   recipe. A free-form bag would let a typo silently generate different
//!   geometry, which is the one failure mode a fuzzing corpus cannot survive.
//!
//! ## What v2 deliberately does NOT do
//!
//! It is not shaped like a persisted OneCAD `OperationRecord`. Benchmark intent
//! and application history are different concepts: history has to round-trip
//! verbatim forever, while a case is free to describe things the product does not
//! expose yet (a `bspline` support, a `faceFullyConsumed` recipe). Coupling them
//! would drag every experiment into the persistence contract.
//!
//! `continuity` admits only `g1`, and `sizeType` only `radius`, even though the
//! roadmap wants G2 and chord width: an option is not a capability, and neither
//! becomes expressible here until a geometric validator can prove the result.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::case::{ExpectedDomain, Limits, MaterialDirection, SearchSpec};
use crate::{AppError, AppResult};

/// The `schemaVersion` this module reads. v1 lives at [`crate::SCHEMA_VERSION`].
pub const SCHEMA_VERSION_V2: u32 = 2;

const MAX_CASE_BYTES: u64 = 1_048_576;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseV2 {
    pub schema_version: u32,
    pub case_id: String,
    pub generator: GeneratorV2,
    pub geometry: Geometry,
    pub operation: Operation,
    pub selector: SelectorV2,
    pub expected_domain: ExpectedDomain,
    pub validators: Vec<ValidatorV2>,
    pub metamorphs: Vec<MetamorphV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchSpec>,
    pub limits: Limits,
}

// ── Generator ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratorV2 {
    /// The operation family. Only fillet exists; the field is what lets a future
    /// chamfer/boolean matrix share this schema instead of forking it.
    pub family: OperationFamily,
    pub name: GeneratorName,
    pub version: u32,
    /// Every generated case must be reproducible from its seed alone.
    pub seed: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationFamily {
    Fillet,
    Boolean,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GeneratorName {
    /// Reproduces the v1 recipe set in v2 shape.
    FilletFoundation,
    /// The expanded support-surface / topology / conditioning matrix.
    FilletMatrix,
    /// Two-body Boolean foundation: explicit boxes and relative placement.
    BooleanFoundation,
}

// ── Geometry ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Geometry {
    pub recipe: RecipeName,
    pub parameters: RecipeParameters,
    /// Cluster labels for reporting only — they never change execution.
    pub tags: Vec<String>,
    /// Which relative-scale band this instance was generated at. Absent means the
    /// recipe's natural size. The sweep is what exposes absolute-tolerance
    /// assumptions in the kernel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_band: Option<ScaleBand>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ScaleBand {
    Micro,
    Unit,
    Kilo,
    Mega,
}

impl ScaleBand {
    /// Representative linear factor for the band (1e-3 / 1 / 1e3 / 1e6).
    pub fn factor(self) -> f64 {
        match self {
            ScaleBand::Micro => 1e-3,
            ScaleBand::Unit => 1.0,
            ScaleBand::Kilo => 1e3,
            ScaleBand::Mega => 1e6,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecipeName {
    // v1 recipes, kept so every v1 case has a v2 equivalent.
    Box,
    #[serde(rename = "valence3Corner")]
    Valence3Corner,
    #[serde(rename = "valence4Corner")]
    Valence4Corner,
    OverflowWedge,
    // The expanded matrix.
    SupportPair,
    ValenceCorner,
    ShortEdge,
    MicroEdge,
    SliverNeighborFace,
    TinyNeighborFace,
    PeriodicSeam,
    NearSeamEdge,
    FaceNearlyConsumed,
    FaceFullyConsumed,
    BlendCollision,
    TwoBoxes,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipeParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_a: Option<SupportSurface>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_b: Option<SupportSurface>,
    /// Angle between the two supports at the shared edge (0 exclusive, 180 inclusive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dihedral_degrees: Option<f64>,
    /// Edges meeting at the selected vertex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valence: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub convexity: Option<Convexity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_length: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neighbor_size: Option<f64>,
    /// Distance from a periodic seam; 0 means ON the seam.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seam_offset: Option<f64>,
    /// Fraction of the target face the blend should eat; `>= 1` is full consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumption_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_dimensions: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_dimensions: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_offset: Option<[f64; 3]>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupportSurface {
    pub kind: SurfaceKindV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor_radius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub half_angle_degrees: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degree: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_points: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SurfaceKindV2 {
    Plane,
    Cylinder,
    Cone,
    Sphere,
    Torus,
    /// Exploratory by policy — freeform supports are generated but never gate.
    Bspline,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Convexity {
    Convex,
    Concave,
    Mixed,
}

// ── Operation ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Operation {
    #[serde(rename = "type")]
    pub operation_type: OperationTypeV2,
    pub definition: OperationDefinition,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationTypeV2 {
    Fillet,
    Boolean,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OperationDefinition {
    Fillet(FilletDefinition),
    Boolean(BooleanDefinition),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BooleanDefinition {
    pub mode: BooleanModeV2,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BooleanModeV2 {
    Fuse,
    Cut,
    Common,
}

impl Operation {
    pub fn max_radius(&self) -> f64 {
        match &self.definition {
            OperationDefinition::Fillet(definition) => definition.radius_law.max_radius(),
            OperationDefinition::Boolean(_) => 0.0,
        }
    }
}

/// Mirrors the kernel-level fillet definition, not the persisted history record.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilletDefinition {
    pub radius_law: RadiusLaw,
    pub continuity: ContinuityV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_type: Option<SizeType>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContinuityV2 {
    /// G2 stays out until a curvature oracle proves OCCT's `SetContinuity`
    /// actually delivers curvature matching. An option is not a capability.
    G1,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SizeType {
    /// Chord width arrives only once a solver measures the RESULTING chord
    /// rather than converting a radius by formula.
    Radius,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum RadiusLaw {
    Constant {
        radius: f64,
    },
    Linear {
        #[serde(rename = "startRadius")]
        start_radius: f64,
        #[serde(rename = "endRadius")]
        end_radius: f64,
    },
    ControlPoints {
        points: Vec<RadiusControlPoint>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RadiusControlPoint {
    /// Normalized position along the contour, in `[0, 1]`.
    pub parameter: f64,
    pub radius: f64,
}

impl RadiusLaw {
    /// The largest radius the law ever requests — what a feasibility probe and
    /// the resource ceilings care about.
    pub fn max_radius(&self) -> f64 {
        match self {
            RadiusLaw::Constant { radius } => *radius,
            RadiusLaw::Linear {
                start_radius,
                end_radius,
            } => start_radius.max(*end_radius),
            RadiusLaw::ControlPoints { points } => {
                points.iter().map(|p| p.radius).fold(0.0, f64::max)
            }
        }
    }
}

// ── Selector ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EdgeSelectorV2 {
    pub mode: SelectorModeV2,
    pub topology_role: TopologyRoleV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub convexity: Option<Convexity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertex_valence: Option<u32>,
    pub provenance: SelectorProvenanceV2,
    /// Recipe-local anchors resolved by nearest-anchor matching. No OCCT ordinal
    /// is ever persisted, and a runner-up inside tolerance is an ambiguous match
    /// rather than a guess.
    pub anchors: Vec<SelectorAnchorV2>,
    pub surface_descriptors: Vec<SurfaceDescriptorV2>,
    pub adjacency: SelectorAdjacencyV2,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SelectorV2 {
    Edge(EdgeSelectorV2),
    BodyRoles(BodyRoleSelectorV2),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BodyRoleSelectorV2 {
    pub mode: BodySelectorModeV2,
    pub target: BodyRoleV2,
    pub tools: Vec<BodyRoleV2>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BodySelectorModeV2 {
    BodyRoles,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BodyRoleV2 {
    Target,
    Tool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SelectorModeV2 {
    Single,
    Disconnected,
    Multiple,
    Chain,
    ClosedContour,
    CornerIncident,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TopologyRoleV2 {
    VerticalEdge,
    CornerIncidentEdge,
    OverflowEdge,
    SupportPairEdge,
    SeamEdge,
    ShortEdge,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectorProvenanceV2 {
    pub generator: GeneratorName,
    pub recipe: RecipeName,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_index: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectorAnchorV2 {
    pub kind: AnchorKindV2,
    pub point: [f64; 3],
    pub frame: AnchorFrameV2,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnchorKindV2 {
    EdgeMidpoint,
    CommonVertex,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnchorFrameV2 {
    RecipeLocal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SurfaceDescriptorV2 {
    pub curve_kind: CurveKindV2,
    pub adjacent_surface_kinds: Vec<SurfaceKindV2>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CurveKindV2 {
    Line,
    AnalyticCurve,
    BsplineCurve,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectorAdjacencyV2 {
    pub relation: AdjacencyRelationV2,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AdjacencyRelationV2 {
    Single,
    PairwiseDisconnected,
    Unconstrained,
    TangentChain,
    ClosedLoop,
    IncidentToCommonVertex,
}

// ── Validators ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidatorV2 {
    #[serde(rename = "type")]
    pub validator_type: ValidatorTypeV2,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub samples: Option<u32>,
}

/// Checks that run against the RESULT geometry.
///
/// v1's semantic check amounted to "the requested constant radius was assigned
/// and some blend face appeared" — which a wrong-but-plausible surface passes.
/// The additions below (cross-section, tangency, self-intersection,
/// manifoldness, tolerance growth, micro topology) measure the output instead of
/// trusting what the builder reported it configured.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ValidatorTypeV2 {
    ConstantRadius,
    GeneratedBlendFace,
    CylindricalRadius,
    G1BoundaryTangency,
    MaterialChange,
    RemoteSupportsUnchanged,
    DeepAudit,
    CrossSectionProfile,
    SupportTangency,
    NoSelfIntersection,
    Manifold,
    ToleranceGrowth,
    MicroTopology,
    History,
    RadiusTolerance,
    TangencyTolerance,
    MaterialTolerance,
    CrossSectionTolerance,
    MetamorphicEquivalence,
    /// Samples the produced blend and checks the radius actually follows the
    /// requested law — the point of variable radius is that the LAW is real, not
    /// that the builder accepted it.
    RadiusLawFollowed,
}

impl ValidatorTypeV2 {
    /// True when the type carries `absolute` + `relative` bounds.
    fn is_tolerance(self) -> bool {
        matches!(
            self,
            ValidatorTypeV2::RadiusTolerance
                | ValidatorTypeV2::TangencyTolerance
                | ValidatorTypeV2::MaterialTolerance
                | ValidatorTypeV2::CrossSectionTolerance
        )
    }
}

// ── Metamorphs ──────────────────────────────────────────────────────────────

/// A transform (or reordering) that must not change whether the operation
/// succeeds. A difference is a numeric-conditioning finding, not a generic
/// operation failure — that distinction is the whole value of the far-origin and
/// scale variants.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum MetamorphV2 {
    Translation {
        vector: [f64; 3],
    },
    /// Same model, rebuilt about a far-away origin — the classic conditioning probe.
    FarOriginTranslation {
        vector: [f64; 3],
    },
    Rotation {
        #[serde(rename = "angleDegrees")]
        angle_degrees: f64,
        axis: [f64; 3],
        center: RotationCenterV2,
    },
    Mirror {
        normal: [f64; 3],
        center: RotationCenterV2,
    },
    UniformScale {
        factor: f64,
        center: RotationCenterV2,
    },
    /// A signed relative nudge to the requested parameter. A result that flips on
    /// ±ε is instability, not a limit.
    ParameterEpsilon {
        parameter: EpsilonParameter,
        #[serde(rename = "relativeDelta")]
        relative_delta: f64,
    },
    /// Selecting the same edges in another order is the same modeling intent.
    EdgeOrderPermutation {
        order: Vec<u32>,
    },
    /// Walking the same contour from a different starting anchor.
    ContourSeed {
        #[serde(rename = "anchorIndex")]
        anchor_index: u32,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum EpsilonParameter {
    #[serde(rename = "operation.radius")]
    OperationRadius,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RotationCenterV2 {
    InputCentroid,
}

// ── Loading and validation ──────────────────────────────────────────────────

impl CaseV2 {
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

    /// Cross-field rules the JSON Schema cannot express, plus the ones it can —
    /// the Rust side is authoritative because it is what actually executes.
    pub fn validate(&self) -> AppResult<()> {
        if self.schema_version != SCHEMA_VERSION_V2 {
            return Err(AppError::cli("unsupported case schemaVersion"));
        }
        validate_id(&self.case_id, 128)?;
        if !is_seed(&self.generator.seed) {
            return Err(AppError::cli("seed must be 16 lowercase hex digits"));
        }
        if self.generator.version == 0 {
            return Err(AppError::cli("generator version must be >= 1"));
        }
        let family_matches = matches!(
            (
                self.generator.family,
                self.generator.name,
                self.operation.operation_type
            ),
            (
                OperationFamily::Fillet,
                GeneratorName::FilletFoundation | GeneratorName::FilletMatrix,
                OperationTypeV2::Fillet
            ) | (
                OperationFamily::Boolean,
                GeneratorName::BooleanFoundation,
                OperationTypeV2::Boolean
            )
        );
        if !family_matches {
            return Err(AppError::cli(
                "generator family, name, and operation type disagree",
            ));
        }
        if self.operation.operation_type == OperationTypeV2::Boolean
            && self.geometry.recipe != RecipeName::TwoBoxes
        {
            return Err(AppError::cli(
                "Boolean foundation requires twoBoxes geometry",
            ));
        }
        if self.operation.operation_type == OperationTypeV2::Boolean && self.search.is_some() {
            return Err(AppError::cli(
                "operation.radius search applies only to fillet",
            ));
        }
        self.validate_geometry()?;
        self.validate_operation()?;
        self.validate_selector()?;
        self.validate_validators()?;
        self.validate_metamorphs()?;
        Ok(())
    }

    fn validate_geometry(&self) -> AppResult<()> {
        let g = &self.geometry;
        if g.tags.is_empty() || g.tags.len() > 32 {
            return Err(AppError::cli("geometry.tags must hold 1..=32 entries"));
        }
        for tag in &g.tags {
            validate_id(tag, 128)?;
        }
        let mut sorted = g.tags.clone();
        sorted.sort();
        sorted.dedup();
        if sorted.len() != g.tags.len() {
            return Err(AppError::cli("geometry.tags must be unique"));
        }

        let p = &g.parameters;
        match g.recipe {
            RecipeName::Box
            | RecipeName::Valence3Corner
            | RecipeName::Valence4Corner
            | RecipeName::OverflowWedge => {
                let dims = p
                    .dimensions
                    .ok_or_else(|| AppError::cli("recipe requires parameters.dimensions"))?;
                if dims.iter().any(|d| !(d.is_finite() && *d > 0.0)) {
                    return Err(AppError::cli("dimensions must be finite and positive"));
                }
                if p.feature_index.is_none() {
                    return Err(AppError::cli("recipe requires parameters.featureIndex"));
                }
            }
            RecipeName::SupportPair => {
                let a = p
                    .support_a
                    .as_ref()
                    .ok_or_else(|| AppError::cli("supportPair requires parameters.supportA"))?;
                let b = p
                    .support_b
                    .as_ref()
                    .ok_or_else(|| AppError::cli("supportPair requires parameters.supportB"))?;
                a.validate("supportA")?;
                b.validate("supportB")?;
                let angle = p.dihedral_degrees.ok_or_else(|| {
                    AppError::cli("supportPair requires parameters.dihedralDegrees")
                })?;
                if !(angle.is_finite() && angle > 0.0 && angle <= 180.0) {
                    return Err(AppError::cli("dihedralDegrees must be in (0, 180]"));
                }
            }
            RecipeName::ValenceCorner => {
                let v = p
                    .valence
                    .ok_or_else(|| AppError::cli("valenceCorner requires parameters.valence"))?;
                if !(3..=8).contains(&v) {
                    return Err(AppError::cli("valence must be in 3..=8"));
                }
            }
            RecipeName::TwoBoxes => {
                for (name, dimensions) in [
                    ("targetDimensions", p.target_dimensions),
                    ("toolDimensions", p.tool_dimensions),
                ] {
                    let dimensions = dimensions
                        .ok_or_else(|| AppError::cli(format!("twoBoxes requires {name}")))?;
                    if dimensions
                        .iter()
                        .any(|value| !value.is_finite() || *value <= 0.0)
                    {
                        return Err(AppError::cli(format!(
                            "{name} must contain finite positive values"
                        )));
                    }
                }
                finite_vector(
                    &p.tool_offset
                        .ok_or_else(|| AppError::cli("twoBoxes requires toolOffset"))?,
                    "toolOffset",
                )?;
                if p.dimensions.is_some()
                    || p.feature_index.is_some()
                    || p.support_a.is_some()
                    || p.support_b.is_some()
                    || p.dihedral_degrees.is_some()
                    || p.valence.is_some()
                    || p.convexity.is_some()
                    || p.edge_length.is_some()
                    || p.neighbor_size.is_some()
                    || p.seam_offset.is_some()
                    || p.consumption_ratio.is_some()
                    || p.separation.is_some()
                {
                    return Err(AppError::cli(
                        "twoBoxes accepts only targetDimensions, toolDimensions, and toolOffset",
                    ));
                }
            }
            _ => {}
        }

        for (name, value) in [
            ("edgeLength", p.edge_length),
            ("neighborSize", p.neighbor_size),
        ] {
            if let Some(v) = value {
                if !(v.is_finite() && v > 0.0) {
                    return Err(AppError::cli(format!("{name} must be finite and positive")));
                }
            }
        }
        for (name, value) in [
            ("seamOffset", p.seam_offset),
            ("separation", p.separation),
            ("consumptionRatio", p.consumption_ratio),
        ] {
            if let Some(v) = value {
                if !(v.is_finite() && v >= 0.0) {
                    return Err(AppError::cli(format!("{name} must be finite and >= 0")));
                }
            }
        }
        Ok(())
    }

    fn validate_operation(&self) -> AppResult<()> {
        let definition = match (&self.operation.operation_type, &self.operation.definition) {
            (OperationTypeV2::Fillet, OperationDefinition::Fillet(definition)) => definition,
            (OperationTypeV2::Boolean, OperationDefinition::Boolean(_)) => return Ok(()),
            _ => return Err(AppError::cli("operation type and definition disagree")),
        };
        match &definition.radius_law {
            RadiusLaw::Constant { radius } => positive(*radius, "radius")?,
            RadiusLaw::Linear {
                start_radius,
                end_radius,
            } => {
                positive(*start_radius, "startRadius")?;
                positive(*end_radius, "endRadius")?;
            }
            RadiusLaw::ControlPoints { points } => {
                if points.len() < 2 || points.len() > 64 {
                    return Err(AppError::cli("controlPoints needs 2..=64 points"));
                }
                let mut previous = f64::NEG_INFINITY;
                for point in points {
                    positive(point.radius, "controlPoint radius")?;
                    if !(point.parameter.is_finite() && (0.0..=1.0).contains(&point.parameter)) {
                        return Err(AppError::cli("controlPoint parameter must be in [0, 1]"));
                    }
                    // Strictly increasing: two radii at one contour parameter is
                    // not a law, it is an ambiguity the kernel would resolve by
                    // accident.
                    if point.parameter <= previous {
                        return Err(AppError::cli(
                            "controlPoint parameters must strictly increase",
                        ));
                    }
                    previous = point.parameter;
                }
            }
        }
        Ok(())
    }

    fn validate_selector(&self) -> AppResult<()> {
        let s = match (&self.operation.operation_type, &self.selector) {
            (OperationTypeV2::Fillet, SelectorV2::Edge(selector)) => selector,
            (OperationTypeV2::Boolean, SelectorV2::BodyRoles(selector)) => {
                if selector.target != BodyRoleV2::Target
                    || selector.tools.as_slice() != [BodyRoleV2::Tool]
                {
                    return Err(AppError::cli(
                        "Boolean selector needs target and exactly one tool role",
                    ));
                }
                return Ok(());
            }
            _ => return Err(AppError::cli("operation and selector families disagree")),
        };
        if s.provenance.recipe != self.geometry.recipe {
            return Err(AppError::cli(
                "selector provenance recipe must match geometry",
            ));
        }
        if s.provenance.generator != self.generator.name {
            return Err(AppError::cli(
                "selector provenance generator must match the case generator",
            ));
        }
        if let (Some(a), Some(b)) = (
            s.provenance.feature_index,
            self.geometry.parameters.feature_index,
        ) {
            if a != b {
                return Err(AppError::cli(
                    "selector provenance featureIndex must match geometry",
                ));
            }
        }
        if s.anchors.is_empty() || s.anchors.len() > 64 {
            return Err(AppError::cli("selector needs 1..=64 anchors"));
        }
        for anchor in &s.anchors {
            if anchor.point.iter().any(|c| !c.is_finite()) {
                return Err(AppError::cli("anchor point must be finite"));
            }
        }
        if s.surface_descriptors.is_empty() || s.surface_descriptors.len() > 64 {
            return Err(AppError::cli("selector needs 1..=64 surfaceDescriptors"));
        }
        for descriptor in &s.surface_descriptors {
            if descriptor.adjacent_surface_kinds.len() != 2 {
                return Err(AppError::cli(
                    "a surface descriptor names exactly 2 supports",
                ));
            }
        }
        if let Some(v) = s.vertex_valence {
            if !(3..=8).contains(&v) {
                return Err(AppError::cli("vertexValence must be in 3..=8"));
            }
        }

        // Mode ⇒ (anchor count, adjacency relation). Keeping these in one table
        // is what stops a case from claiming "chain" while carrying a
        // disconnected adjacency, which would silently benchmark a different
        // selection than the one it names.
        use AdjacencyRelationV2 as R;
        use SelectorModeV2 as M;
        let (min, max, relation) = match s.mode {
            M::Single => (1, 1, R::Single),
            M::Disconnected => (2, 64, R::PairwiseDisconnected),
            M::Multiple => (2, 64, R::Unconstrained),
            M::Chain => (2, 64, R::TangentChain),
            M::ClosedContour => (3, 64, R::ClosedLoop),
            M::CornerIncident => (1, 1, R::IncidentToCommonVertex),
        };
        if s.anchors.len() < min || s.anchors.len() > max {
            return Err(AppError::cli("anchor count does not match selector mode"));
        }
        if s.adjacency.relation != relation {
            return Err(AppError::cli(
                "adjacency relation does not match selector mode",
            ));
        }
        if s.mode == M::CornerIncident {
            if s.topology_role != TopologyRoleV2::CornerIncidentEdge {
                return Err(AppError::cli(
                    "cornerIncident selection must use the cornerIncidentEdge role",
                ));
            }
            if s.anchors[0].kind != AnchorKindV2::CommonVertex {
                return Err(AppError::cli(
                    "cornerIncident selection anchors on the common vertex",
                ));
            }
        }
        Ok(())
    }

    fn validate_validators(&self) -> AppResult<()> {
        if self.validators.is_empty() || self.validators.len() > 48 {
            return Err(AppError::cli("a case needs 1..=48 validators"));
        }
        for v in &self.validators {
            if self.operation.operation_type == OperationTypeV2::Boolean
                && matches!(
                    v.validator_type,
                    ValidatorTypeV2::ConstantRadius
                        | ValidatorTypeV2::GeneratedBlendFace
                        | ValidatorTypeV2::CylindricalRadius
                        | ValidatorTypeV2::G1BoundaryTangency
                        | ValidatorTypeV2::RemoteSupportsUnchanged
                        | ValidatorTypeV2::CrossSectionProfile
                        | ValidatorTypeV2::SupportTangency
                        | ValidatorTypeV2::RadiusTolerance
                        | ValidatorTypeV2::TangencyTolerance
                        | ValidatorTypeV2::CrossSectionTolerance
                        | ValidatorTypeV2::RadiusLawFollowed
                )
            {
                return Err(AppError::cli("Fillet validator does not apply to Boolean"));
            }
            match v.validator_type {
                t if t.is_tolerance() => {
                    let absolute = v
                        .absolute
                        .ok_or_else(|| AppError::cli("tolerance validator needs `absolute`"))?;
                    let relative = v
                        .relative
                        .ok_or_else(|| AppError::cli("tolerance validator needs `relative`"))?;
                    non_negative(absolute, "absolute")?;
                    unit_interval(relative, "relative")?;
                }
                ValidatorTypeV2::MetamorphicEquivalence => {
                    let samples = v.surface_samples.ok_or_else(|| {
                        AppError::cli("metamorphicEquivalence needs `surfaceSamples`")
                    })?;
                    if !(1..=4096).contains(&samples) {
                        return Err(AppError::cli("surfaceSamples must be in 1..=4096"));
                    }
                    let tolerance = v.point_tolerance.ok_or_else(|| {
                        AppError::cli("metamorphicEquivalence needs `pointTolerance`")
                    })?;
                    non_negative(tolerance, "pointTolerance")?;
                }
                ValidatorTypeV2::RadiusLawFollowed => {
                    let samples = v
                        .samples
                        .ok_or_else(|| AppError::cli("radiusLawFollowed needs `samples`"))?;
                    if !(2..=1024).contains(&samples) {
                        return Err(AppError::cli("samples must be in 2..=1024"));
                    }
                    non_negative(
                        v.absolute
                            .ok_or_else(|| AppError::cli("radiusLawFollowed needs `absolute`"))?,
                        "absolute",
                    )?;
                    unit_interval(
                        v.relative
                            .ok_or_else(|| AppError::cli("radiusLawFollowed needs `relative`"))?,
                        "relative",
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_metamorphs(&self) -> AppResult<()> {
        if self.metamorphs.len() > 32 {
            return Err(AppError::cli("at most 32 metamorphs"));
        }
        let anchors = match &self.selector {
            SelectorV2::Edge(selector) => selector.anchors.len(),
            SelectorV2::BodyRoles(_) => 0,
        };
        for m in &self.metamorphs {
            match m {
                MetamorphV2::Translation { vector }
                | MetamorphV2::FarOriginTranslation { vector } => {
                    finite_vector(vector, "metamorph vector")?;
                }
                MetamorphV2::Rotation {
                    angle_degrees,
                    axis,
                    ..
                } => {
                    if !(angle_degrees.is_finite() && (-360.0..=360.0).contains(angle_degrees)) {
                        return Err(AppError::cli("rotation angle must be in [-360, 360]"));
                    }
                    non_zero_vector(axis, "rotation axis")?;
                }
                MetamorphV2::Mirror { normal, .. } => non_zero_vector(normal, "mirror normal")?,
                MetamorphV2::UniformScale { factor, .. } => positive(*factor, "scale factor")?,
                MetamorphV2::ParameterEpsilon { relative_delta, .. } => {
                    if self.operation.operation_type != OperationTypeV2::Fillet {
                        return Err(AppError::cli(
                            "operation.radius metamorph applies only to fillet",
                        ));
                    }
                    if !(relative_delta.is_finite()
                        && (-0.01..=0.01).contains(relative_delta)
                        && *relative_delta != 0.0)
                    {
                        return Err(AppError::cli(
                            "parameterEpsilon relativeDelta must be a non-zero value in [-0.01, 0.01]",
                        ));
                    }
                }
                MetamorphV2::EdgeOrderPermutation { order } => {
                    // A real permutation of the selector's anchors — anything else
                    // silently benchmarks a different selection.
                    if order.len() != anchors {
                        return Err(AppError::cli(
                            "edgeOrderPermutation must cover every selector anchor",
                        ));
                    }
                    let mut seen = order.clone();
                    seen.sort_unstable();
                    seen.dedup();
                    if seen.len() != order.len() {
                        return Err(AppError::cli("edgeOrderPermutation has duplicate indices"));
                    }
                    if order.iter().any(|i| *i as usize >= anchors) {
                        return Err(AppError::cli("edgeOrderPermutation index out of range"));
                    }
                }
                MetamorphV2::ContourSeed { anchor_index } => {
                    if *anchor_index as usize >= anchors {
                        return Err(AppError::cli("contourSeed anchorIndex out of range"));
                    }
                }
            }
        }
        Ok(())
    }
}

impl SupportSurface {
    fn validate(&self, field: &str) -> AppResult<()> {
        let need = |present: bool, what: &str| -> AppResult<()> {
            if present {
                Ok(())
            } else {
                Err(AppError::cli(format!("{field}: {what} kind requires it")))
            }
        };
        match self.kind {
            SurfaceKindV2::Plane | SurfaceKindV2::Other => {}
            SurfaceKindV2::Cylinder | SurfaceKindV2::Sphere => {
                need(self.radius.is_some(), "radius, which this")?;
            }
            SurfaceKindV2::Cone => {
                need(self.radius.is_some(), "radius, which this")?;
                need(
                    self.half_angle_degrees.is_some(),
                    "halfAngleDegrees, which this",
                )?;
            }
            SurfaceKindV2::Torus => {
                need(self.radius.is_some(), "radius, which this")?;
                need(self.minor_radius.is_some(), "minorRadius, which this")?;
            }
            SurfaceKindV2::Bspline => {
                need(self.degree.is_some(), "degree, which this")?;
                need(self.control_points.is_some(), "controlPoints, which this")?;
            }
        }
        for (name, value) in [("radius", self.radius), ("minorRadius", self.minor_radius)] {
            if let Some(v) = value {
                positive(v, name)?;
            }
        }
        if let Some(a) = self.half_angle_degrees {
            if !(a.is_finite() && a > 0.0 && a < 90.0) {
                return Err(AppError::cli(format!(
                    "{field}: halfAngleDegrees ∈ (0, 90)"
                )));
            }
        }
        if let Some(d) = self.degree {
            if !(1..=8).contains(&d) {
                return Err(AppError::cli(format!("{field}: degree ∈ 1..=8")));
            }
        }
        if let Some(n) = self.control_points {
            if !(2..=256).contains(&n) {
                return Err(AppError::cli(format!("{field}: controlPoints ∈ 2..=256")));
            }
        }
        // A torus whose tube is at least its ring radius self-intersects; the
        // generator must not emit one and call it a support.
        if self.kind == SurfaceKindV2::Torus {
            if let (Some(major), Some(minor)) = (self.radius, self.minor_radius) {
                if minor >= major {
                    return Err(AppError::cli(format!(
                        "{field}: torus minorRadius must be < radius"
                    )));
                }
            }
        }
        Ok(())
    }
}

fn positive(value: f64, what: &str) -> AppResult<()> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(AppError::cli(format!("{what} must be finite and positive")))
    }
}

fn non_negative(value: f64, what: &str) -> AppResult<()> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(AppError::cli(format!("{what} must be finite and >= 0")))
    }
}

fn unit_interval(value: f64, what: &str) -> AppResult<()> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(AppError::cli(format!("{what} must be in [0, 1]")))
    }
}

fn finite_vector(v: &[f64; 3], what: &str) -> AppResult<()> {
    if v.iter().all(|c| c.is_finite()) {
        Ok(())
    } else {
        Err(AppError::cli(format!("{what} must be finite")))
    }
}

fn non_zero_vector(v: &[f64; 3], what: &str) -> AppResult<()> {
    finite_vector(v, what)?;
    if v.iter().any(|c| *c != 0.0) {
        Ok(())
    } else {
        Err(AppError::cli(format!("{what} must not be the zero vector")))
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

    const EXAMPLE: &str =
        include_str!("../../../../bench/robustness/examples/fillet-matrix-plane-cylinder-v2.json");
    const V1_REGRESSION: &str =
        include_str!("../../../../bench/robustness/regressions/fillet-box-supported-single.json");
    const BOOLEAN_EXAMPLE: &str =
        include_str!("../../../../bench/robustness/examples/boolean-two-box-overlap-v2.json");

    fn example() -> serde_json::Value {
        serde_json::from_str(EXAMPLE).unwrap()
    }

    fn parse(value: serde_json::Value) -> AppResult<CaseV2> {
        let case: CaseV2 =
            serde_json::from_value(value).map_err(|e| AppError::cli(format!("serde: {e}")))?;
        case.validate()?;
        Ok(case)
    }

    fn boolean_case() -> serde_json::Value {
        serde_json::from_str(BOOLEAN_EXAMPLE).unwrap()
    }

    #[test]
    fn the_committed_example_parses_and_validates() {
        let case = parse(example()).unwrap();
        assert_eq!(case.schema_version, SCHEMA_VERSION_V2);
        assert_eq!(case.geometry.recipe, RecipeName::SupportPair);
        assert_eq!(case.generator.family, OperationFamily::Fillet);
        assert_eq!(case.operation.max_radius(), 3.0);
    }

    #[test]
    fn boolean_definition_and_body_roles_parse_strictly() {
        let case = parse(boolean_case()).unwrap();
        assert_eq!(case.generator.family, OperationFamily::Boolean);
        assert_eq!(case.geometry.recipe, RecipeName::TwoBoxes);
        assert_eq!(case.operation.max_radius(), 0.0);

        let mut mismatched = boolean_case();
        mismatched["operation"]["definition"] =
            serde_json::json!({"radiusLaw":{"mode":"constant","radius":1},"continuity":"g1"});
        assert!(parse(mismatched).is_err());

        let mut extra_parameter = boolean_case();
        extra_parameter["geometry"]["parameters"]["dimensions"] = serde_json::json!([1, 1, 1]);
        assert!(parse(extra_parameter).is_err());

        let mut wrong_roles = boolean_case();
        wrong_roles["selector"]["tools"] = serde_json::json!(["target"]);
        assert!(parse(wrong_roles).is_err());

        let mut fillet_validator = boolean_case();
        fillet_validator["validators"][0]["type"] = serde_json::json!("constantRadius");
        assert!(parse(fillet_validator).is_err());

        let mut radius_search = boolean_case();
        radius_search["search"] = serde_json::json!({
            "parameter": "operation.radius",
            "knownSuccess": 1,
            "upperBound": 2,
            "growthFactor": 2,
            "maxProbes": 8,
            "relativePrecision": 0.001,
            "absolutePrecision": 0.001,
            "relativeOffsets": [0.01, 0.0001, 0.000001, 0.00000001]
        });
        assert!(parse(radius_search).is_err());
    }

    /// Serialization must reproduce the committed file EXACTLY (as a value, so key
    /// order is free). Anything less and the Rust types could drift away from
    /// `case-v2.schema.json` — a `#[serde(rename)]` I forget would still round-trip
    /// through itself while emitting `start_radius` where the schema says
    /// `startRadius`, and only a schema run would catch it.
    /// `serde_json::Value` distinguishes `90` from `90.0`, and the committed file
    /// writes whole numbers without a decimal point while the typed round trip
    /// emits `f64`. That difference is pure JSON spelling, so both sides are
    /// coerced to f64 before comparing — the comparison is about KEY NAMES.
    fn numbers_as_f64(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Number(n) => serde_json::Number::from_f64(n.as_f64().unwrap())
                .map(serde_json::Value::Number)
                .unwrap_or_else(|| value.clone()),
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(numbers_as_f64).collect())
            }
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), numbers_as_f64(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    #[test]
    fn round_trip_reproduces_the_committed_file() {
        let case = parse(example()).unwrap();
        let emitted = serde_json::to_value(&case).unwrap();
        assert_eq!(numbers_as_f64(&emitted), numbers_as_f64(&example()));
        let twice = serde_json::to_value(parse(emitted.clone()).unwrap()).unwrap();
        assert_eq!(numbers_as_f64(&emitted), numbers_as_f64(&twice));
    }

    /// The two formats must never be readable as each other: a v1 regression
    /// silently parsed as v2 (or vice versa) would benchmark different intent
    /// than the file describes.
    #[test]
    fn the_two_versions_are_disjoint() {
        let v1: serde_json::Value = serde_json::from_str(V1_REGRESSION).unwrap();
        assert!(serde_json::from_value::<CaseV2>(v1).is_err());
        assert!(serde_json::from_value::<crate::case::Case>(example()).is_err());
    }

    /// v1 stays FROZEN — its committed regressions keep validating unchanged.
    #[test]
    fn v1_regressions_still_validate_under_v1() {
        let case: crate::case::Case = serde_json::from_str(V1_REGRESSION).unwrap();
        case.validate().unwrap();
    }

    #[test]
    fn a_wrong_schema_version_is_rejected() {
        let mut v = example();
        v["schemaVersion"] = serde_json::json!(1);
        assert!(parse(v).is_err());
    }

    #[test]
    fn unknown_fields_are_rejected_everywhere() {
        for path in ["generator", "geometry", "operation", "selector", "limits"] {
            let mut v = example();
            v[path]["unknown"] = serde_json::json!(true);
            assert!(
                serde_json::from_value::<CaseV2>(v).is_err(),
                "{path} accepted an unknown field"
            );
        }
    }

    #[test]
    fn selector_mode_must_agree_with_its_adjacency_and_anchor_count() {
        // `chain` claimed while the adjacency still says `single`.
        let mut v = example();
        v["selector"]["mode"] = serde_json::json!("chain");
        assert!(parse(v).is_err());

        // `single` with two anchors.
        let mut v = example();
        let anchor = v["selector"]["anchors"][0].clone();
        v["selector"]["anchors"] = serde_json::json!([anchor.clone(), anchor]);
        assert!(parse(v).is_err());
    }

    #[test]
    fn corner_incident_requires_its_role_and_a_common_vertex_anchor() {
        let mut v = example();
        v["selector"]["mode"] = serde_json::json!("cornerIncident");
        v["selector"]["adjacency"]["relation"] = serde_json::json!("incidentToCommonVertex");
        // Role is still supportPairEdge → rejected.
        assert!(parse(v.clone()).is_err());

        v["selector"]["topologyRole"] = serde_json::json!("cornerIncidentEdge");
        // Anchor is still an edge midpoint → rejected.
        assert!(parse(v.clone()).is_err());

        v["selector"]["anchors"][0]["kind"] = serde_json::json!("commonVertex");
        assert!(parse(v).is_ok());
    }

    #[test]
    fn selector_provenance_must_match_the_case() {
        let mut v = example();
        v["selector"]["provenance"]["recipe"] = serde_json::json!("box");
        assert!(parse(v).is_err());

        let mut v = example();
        v["selector"]["provenance"]["generator"] = serde_json::json!("fillet-foundation");
        assert!(parse(v).is_err());
    }

    #[test]
    fn support_pair_requires_both_supports_and_a_real_angle() {
        for bad in [0.0, -5.0, 181.0] {
            let mut v = example();
            v["geometry"]["parameters"]["dihedralDegrees"] = serde_json::json!(bad);
            assert!(parse(v).is_err(), "accepted dihedral {bad}");
        }
        let mut v = example();
        v["geometry"]["parameters"]
            .as_object_mut()
            .unwrap()
            .remove("supportB");
        assert!(parse(v).is_err());
    }

    #[test]
    fn a_cylinder_support_needs_a_radius_and_a_torus_needs_a_thinner_tube() {
        let mut v = example();
        v["geometry"]["parameters"]["supportB"] = serde_json::json!({ "kind": "cylinder" });
        assert!(parse(v).is_err());

        // A tube at least as fat as the ring self-intersects — not a support.
        let mut v = example();
        v["geometry"]["parameters"]["supportB"] =
            serde_json::json!({ "kind": "torus", "radius": 10, "minorRadius": 10 });
        assert!(parse(v).is_err());

        let mut v = example();
        v["geometry"]["parameters"]["supportB"] =
            serde_json::json!({ "kind": "torus", "radius": 10, "minorRadius": 2 });
        v["selector"]["surfaceDescriptors"][0]["adjacentSurfaceKinds"] =
            serde_json::json!(["plane", "torus"]);
        assert!(parse(v).is_ok());
    }

    #[test]
    fn control_point_laws_must_strictly_increase() {
        let mut v = example();
        v["operation"]["definition"]["radiusLaw"] = serde_json::json!({
            "mode": "controlPoints",
            "points": [
                { "parameter": 0.0, "radius": 2 },
                { "parameter": 0.5, "radius": 4 },
                { "parameter": 0.5, "radius": 1 }
            ]
        });
        assert!(parse(v).is_err());

        let mut v = example();
        v["operation"]["definition"]["radiusLaw"] = serde_json::json!({
            "mode": "controlPoints",
            "points": [
                { "parameter": 0.0, "radius": 2 },
                { "parameter": 0.5, "radius": 4 },
                { "parameter": 1.0, "radius": 1 }
            ]
        });
        let case = parse(v).unwrap();
        assert_eq!(case.operation.max_radius(), 4.0);
    }

    #[test]
    fn linear_and_constant_laws_report_their_peak_radius() {
        let mut v = example();
        v["operation"]["definition"]["radiusLaw"] =
            serde_json::json!({ "mode": "linear", "startRadius": 2, "endRadius": 7 });
        assert_eq!(parse(v).unwrap().operation.max_radius(), 7.0);
    }

    #[test]
    fn an_edge_order_permutation_must_be_a_real_permutation_of_the_anchors() {
        let mut v = example();
        let anchor = v["selector"]["anchors"][0].clone();
        v["selector"]["mode"] = serde_json::json!("multiple");
        v["selector"]["adjacency"]["relation"] = serde_json::json!("unconstrained");
        v["selector"]["anchors"] = serde_json::json!([anchor.clone(), anchor]);

        let mut duplicate = v.clone();
        duplicate["metamorphs"] =
            serde_json::json!([{ "type": "edgeOrderPermutation", "order": [0, 0] }]);
        assert!(parse(duplicate).is_err());

        let mut short = v.clone();
        short["metamorphs"] = serde_json::json!([{ "type": "edgeOrderPermutation", "order": [0] }]);
        assert!(parse(short).is_err());

        let mut ok = v;
        ok["metamorphs"] = serde_json::json!([{ "type": "edgeOrderPermutation", "order": [1, 0] }]);
        assert!(parse(ok).is_ok());
    }

    #[test]
    fn a_zero_epsilon_is_not_a_metamorph() {
        let mut v = example();
        v["metamorphs"] = serde_json::json!([{
            "type": "parameterEpsilon",
            "parameter": "operation.radius",
            "relativeDelta": 0.0
        }]);
        assert!(parse(v).is_err());
    }

    #[test]
    fn degenerate_transforms_are_rejected() {
        let mut v = example();
        v["metamorphs"] = serde_json::json!([{ "type": "mirror", "normal": [0, 0, 0], "center": "inputCentroid" }]);
        assert!(parse(v).is_err());

        let mut v = example();
        v["metamorphs"] = serde_json::json!([{
            "type": "uniformScale", "factor": 0, "center": "inputCentroid"
        }]);
        assert!(parse(v).is_err());
    }

    #[test]
    fn tolerance_and_law_validators_require_their_bounds() {
        let mut v = example();
        v["validators"] = serde_json::json!([{ "type": "radiusTolerance", "required": true }]);
        assert!(parse(v).is_err());

        let mut v = example();
        v["validators"] = serde_json::json!([{ "type": "radiusLawFollowed", "required": true }]);
        assert!(parse(v).is_err());

        let mut v = example();
        v["validators"] = serde_json::json!([{
            "type": "radiusLawFollowed", "required": true,
            "samples": 32, "absolute": 0.0001, "relative": 0.001
        }]);
        assert!(parse(v).is_ok());
    }

    #[test]
    fn tags_must_be_unique_safe_identifiers() {
        let mut v = example();
        v["geometry"]["tags"] = serde_json::json!(["dup", "dup"]);
        assert!(parse(v).is_err());

        let mut v = example();
        v["geometry"]["tags"] = serde_json::json!(["../escape"]);
        assert!(parse(v).is_err());
    }

    #[test]
    fn case_ids_and_seeds_are_constrained() {
        let mut v = example();
        v["caseId"] = serde_json::json!("../escape");
        assert!(parse(v).is_err());

        let mut v = example();
        v["generator"]["seed"] = serde_json::json!("0123456789ABCDEF");
        assert!(parse(v).is_err());
    }

    #[test]
    fn scale_bands_expose_their_factor() {
        assert_eq!(ScaleBand::Micro.factor(), 1e-3);
        assert_eq!(ScaleBand::Unit.factor(), 1.0);
        assert_eq!(ScaleBand::Kilo.factor(), 1e3);
        assert_eq!(ScaleBand::Mega.factor(), 1e6);
    }
}
