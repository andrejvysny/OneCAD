use serde::{Deserialize, Serialize};

use crate::case::*;

const SEED: u64 = 0x6f6e_6563_6164_7430;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Variant {
    pub name: VariantName,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Rotation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<Mirror>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<Scale>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter_epsilon: Option<ParameterEpsilon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_order_permutation: Option<EdgeOrderPermutation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contour_seed: Option<ContourSeed>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VariantName {
    Base,
    Translated,
    FarOriginTranslated,
    Rotated,
    Mirrored,
    Scaled,
    ParameterEpsilon,
    EdgeOrderPermutation,
    ContourSeed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rotation {
    pub axis: [f64; 3],
    pub angle_degrees: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Mirror {
    pub normal: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center: Option<[f64; 3]>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scale {
    pub factor: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center: Option<[f64; 3]>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParameterEpsilon {
    pub parameter: String,
    #[serde(rename = "relativeDelta")]
    pub relative_delta: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EdgeOrderPermutation {
    pub order: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContourSeed {
    #[serde(rename = "anchorIndex")]
    pub anchor_index: u32,
}

#[derive(Clone, Debug)]
pub struct GeneratedCase {
    pub case: crate::prepared::PreparedCase,
    pub variant: Variant,
}

#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    pub fn next_f64(&mut self) -> f64 {
        let mantissa = self.next_u64() >> 11;
        mantissa as f64 * (1.0 / ((1_u64 << 53) as f64))
    }
}

pub fn t0() -> Vec<GeneratedCase> {
    let mut result = Vec::with_capacity(68);
    for case in base_cases() {
        result.push(generated(case.clone(), VariantName::Base));
        if metamorphic_domain(case.expected_domain) {
            result.push(generated(case.clone(), VariantName::Translated));
            result.push(generated(case, VariantName::Rotated));
        }
    }
    result
}

fn base_cases() -> Vec<Case> {
    let mut rng = SplitMix64::new(SEED);
    let mut cases = Vec::with_capacity(36);
    for index in 0..12 {
        cases.push(box_case(index, false, &mut rng));
    }
    for index in 0..4 {
        cases.push(box_case(index, true, &mut rng));
    }
    add_exploratory(&mut cases, GeometryType::Valence3, "valence3", 8, &mut rng);
    add_exploratory(&mut cases, GeometryType::Valence4, "valence4", 8, &mut rng);
    add_exploratory(
        &mut cases,
        GeometryType::OverflowWedge,
        "overflow",
        4,
        &mut rng,
    );
    cases
}

fn box_case(index: usize, oversized: bool, rng: &mut SplitMix64) -> Case {
    let dimensions = dimensions(rng);
    let feature_size = dimensions[0].min(dimensions[1]);
    let ratio = if oversized {
        1.1 + 0.4 * rng.next_f64()
    } else {
        0.04 + 0.12 * rng.next_f64()
    };
    let mode = match index % 3 {
        0 => SelectorMode::Single,
        1 => SelectorMode::Disconnected,
        _ => SelectorMode::Multiple,
    };
    let domain = if oversized {
        ExpectedDomain::ExpectedLimit
    } else {
        ExpectedDomain::Supported
    };
    let prefix = if oversized {
        "box-expected-limit"
    } else {
        "box-supported"
    };
    make_case(
        format!("{prefix}-{index:02}"),
        GeometryType::Box,
        dimensions,
        feature_size * ratio,
        mode,
        domain,
        rng,
    )
}

fn add_exploratory(
    cases: &mut Vec<Case>,
    recipe_type: GeometryType,
    prefix: &str,
    count: usize,
    rng: &mut SplitMix64,
) {
    // Overflow wedges have no valence-3/4 corner to anchor a corner-incident
    // selector on; they exercise a single tapering edge instead.
    let mode = if matches!(recipe_type, GeometryType::OverflowWedge) {
        SelectorMode::Single
    } else {
        SelectorMode::CornerIncident
    };
    for index in 0..count {
        let dimensions = dimensions(rng);
        let radius = dimensions[0] * (0.08 + 0.2 * rng.next_f64());
        cases.push(make_case(
            format!("{prefix}-{index:02}"),
            recipe_type,
            dimensions,
            radius,
            mode,
            ExpectedDomain::Exploratory,
            rng,
        ));
    }
}

fn dimensions(rng: &mut SplitMix64) -> [f64; 3] {
    [
        20.0 + 80.0 * rng.next_f64(),
        25.0 + 70.0 * rng.next_f64(),
        30.0 + 60.0 * rng.next_f64(),
    ]
}

fn make_case(
    case_id: String,
    recipe_type: GeometryType,
    dimensions: [f64; 3],
    radius: f64,
    mode: SelectorMode,
    domain: ExpectedDomain,
    rng: &mut SplitMix64,
) -> Case {
    let seed = format!("{:016x}", rng.next_u64());
    Case {
        schema_version: 1,
        case_id,
        generator: Generator {
            name: "fillet-foundation".into(),
            version: 1,
            seed,
        },
        geometry_recipe: GeometryRecipe {
            recipe_type,
            dimensions: dimensions.to_vec(),
            feature_index: 0,
        },
        geometry_tags: tags(recipe_type, mode),
        operation: operation(radius, mode, recipe_type, dimensions),
        expected_domain: domain,
        validators: validators(recipe_type, domain),
        metamorphs: metamorphs(domain),
        search: Some(search(dimensions[0])),
        limits: default_limits(),
    }
}

fn tags(recipe_type: GeometryType, mode: SelectorMode) -> Vec<String> {
    let recipe = match recipe_type {
        GeometryType::Box => "box",
        GeometryType::Valence3 => "valence3",
        GeometryType::Valence4 => "valence4",
        GeometryType::OverflowWedge => "overflow-wedge",
    };
    let selection = match mode {
        SelectorMode::Single => "single-edge",
        SelectorMode::Disconnected => "disconnected-edges",
        SelectorMode::Multiple => "multiple-edges",
        SelectorMode::CornerIncident => "corner-incident",
    };
    vec!["analytic".into(), recipe.into(), selection.into()]
}

fn operation(
    radius: f64,
    mode: SelectorMode,
    recipe_type: GeometryType,
    dimensions: [f64; 3],
) -> FilletOperation {
    let anchors = selector_anchors(recipe_type, mode, dimensions);
    // Non-corner modes match a `curveKind`/`adjacentSurfaceKinds` descriptor
    // to each selected anchor 1:1 (see SelectorParser.cpp's
    // `selector_matches_case`); every generated recipe here selects straight
    // edges bordered by two planes, so the descriptor is a fixed template
    // repeated once per anchor.
    let surface_descriptors = vec![
        SurfaceDescriptor {
            curve_kind: CurveKind::Line,
            adjacent_surface_kinds: vec![SurfaceKind::Plane, SurfaceKind::Plane],
        };
        anchors.len()
    ];
    FilletOperation {
        operation_type: OperationType::Fillet,
        radius,
        continuity: Continuity::G1,
        radius_mode: RadiusMode::Constant,
        selector: Selector {
            mode,
            topology_role: topology_role(recipe_type),
            provenance: SelectorProvenance {
                generator: "fillet-foundation".into(),
                recipe_type,
                feature_index: 0,
            },
            anchors,
            surface_descriptors,
            adjacency: SelectorAdjacency {
                relation: adjacency(mode),
            },
        },
    }
}

fn topology_role(recipe_type: GeometryType) -> TopologyRole {
    match recipe_type {
        GeometryType::Box => TopologyRole::VerticalEdge,
        GeometryType::OverflowWedge => TopologyRole::OverflowEdge,
        GeometryType::Valence3 | GeometryType::Valence4 => TopologyRole::CornerIncidentEdge,
    }
}

fn selector_anchors(
    recipe_type: GeometryType,
    mode: SelectorMode,
    dimensions: [f64; 3],
) -> Vec<SelectorAnchor> {
    if matches!(mode, SelectorMode::CornerIncident) {
        return vec![anchor(
            AnchorKind::CommonVertex,
            corner(recipe_type, dimensions),
        )];
    }
    if matches!(recipe_type, GeometryType::OverflowWedge) {
        return vec![anchor(AnchorKind::EdgeMidpoint, overflow_edge(dimensions))];
    }
    let [x, y, z] = dimensions;
    let points = [
        [0.0, 0.0, z / 2.0],
        [0.0, y, z / 2.0],
        [x, 0.0, z / 2.0],
        [x, y, z / 2.0],
    ];
    let selected: &[usize] = match mode {
        SelectorMode::Single => &[0],
        SelectorMode::Disconnected => &[0, 3],
        SelectorMode::Multiple => &[0, 1, 2],
        SelectorMode::CornerIncident => unreachable!(),
    };
    selected
        .iter()
        .map(|index| anchor(AnchorKind::EdgeMidpoint, points[*index]))
        .collect()
}

/// `BRepPrimAPI_MakeWedge(dx, dy, dz, ltx)` tapers the X extent from `dx` at
/// `y=0` down to `ltx` at `y=dy`, leaving Z untapered; this is the midpoint of
/// the tapered edge on the `z=dz` face, verified against actual OCCT output.
fn overflow_edge(dimensions: [f64; 3]) -> [f64; 3] {
    let [dx, dy, dz] = dimensions;
    let ltx = dx * 0.25;
    [(dx + ltx) / 2.0, dy / 2.0, dz]
}

fn corner(recipe_type: GeometryType, dimensions: [f64; 3]) -> [f64; 3] {
    match recipe_type {
        GeometryType::Valence4 => [0.0, 0.0, dimensions[2]],
        _ => [0.0, 0.0, 0.0],
    }
}

fn anchor(kind: AnchorKind, point: [f64; 3]) -> SelectorAnchor {
    SelectorAnchor {
        kind,
        point,
        frame: AnchorFrame::RecipeLocal,
    }
}

fn adjacency(mode: SelectorMode) -> AdjacencyRelation {
    match mode {
        SelectorMode::Single => AdjacencyRelation::Single,
        SelectorMode::Disconnected => AdjacencyRelation::PairwiseDisconnected,
        SelectorMode::Multiple => AdjacencyRelation::Unconstrained,
        SelectorMode::CornerIncident => AdjacencyRelation::IncidentToCommonVertex,
    }
}

fn validators(recipe_type: GeometryType, domain: ExpectedDomain) -> Vec<Validator> {
    let mut result = vec![
        simple(ValidatorType::ConstantRadius),
        simple(ValidatorType::GeneratedBlendFace),
        simple(ValidatorType::MaterialChange),
        simple(ValidatorType::DeepAudit),
    ];
    result[2].direction = Some(MaterialDirection::Decrease);
    if matches!(recipe_type, GeometryType::Box) {
        result.extend([
            simple(ValidatorType::CylindricalRadius),
            simple(ValidatorType::G1BoundaryTangency),
            simple(ValidatorType::RemoteSupportsUnchanged),
        ]);
    }
    if metamorphic_domain(domain) {
        // Not gated through the generic required-validator pass: the runner
        // has no cross-variant context to judge this itself (it always
        // reports `notRun`), so campaign.rs's post-hoc metamorph comparison
        // is what actually gates supported/expectedLimit variants.
        result.push(metamorphic_equivalence());
    }
    result
}

fn simple(validator_type: ValidatorType) -> Validator {
    Validator {
        validator_type,
        required: true,
        direction: None,
        absolute: None,
        relative: None,
        surface_samples: None,
        point_tolerance: None,
    }
}

fn metamorphic_equivalence() -> Validator {
    Validator {
        validator_type: ValidatorType::MetamorphicEquivalence,
        required: false,
        direction: None,
        absolute: None,
        relative: None,
        surface_samples: Some(256),
        point_tolerance: Some(1e-6),
    }
}

fn metamorphs(domain: ExpectedDomain) -> Vec<Metamorph> {
    if !metamorphic_domain(domain) {
        return Vec::new();
    }
    vec![
        Metamorph::Translation {
            vector: [1000.0, -2000.0, 3000.0],
        },
        Metamorph::Rotation {
            angle_degrees: 17.137,
            axis: [1.0, 2.0, 3.0],
            center: RotationCenter::InputCentroid,
        },
    ]
}

fn metamorphic_domain(domain: ExpectedDomain) -> bool {
    matches!(
        domain,
        ExpectedDomain::Supported | ExpectedDomain::ExpectedLimit
    )
}

fn generated(case: Case, name: VariantName) -> GeneratedCase {
    let translation = matches!(name, VariantName::Translated).then_some([1000.0, -2000.0, 3000.0]);
    let rotation = matches!(name, VariantName::Rotated).then_some(Rotation {
        axis: [1.0, 2.0, 3.0],
        angle_degrees: 17.137,
    });
    GeneratedCase {
        case: case.prepared(),
        variant: Variant {
            name,
            translation,
            rotation,
            mirror: None,
            scale: None,
            parameter_epsilon: None,
            edge_order_permutation: None,
            contour_seed: None,
        },
    }
}

fn search(feature: f64) -> SearchSpec {
    SearchSpec {
        parameter: SearchParameter::OperationRadius,
        known_success: feature * 0.02,
        upper_bound: feature,
        growth_factor: 2.0,
        max_probes: 256,
        relative_precision: 1e-6,
        absolute_precision: 1e-6,
        relative_offsets: [1e-2, 1e-4, 1e-6, 1e-8],
    }
}

fn default_limits() -> Limits {
    Limits {
        resources: ResourceLimits {
            wall_time_ms: 10_000,
            address_space_bytes: 2_147_483_648,
            stdout_bytes: 1_048_576,
            stderr_bytes: 1_048_576,
            artifact_bytes: 67_108_864,
        },
        quality: QualityLimits {
            max_vertex_tolerance: None,
            max_edge_tolerance: None,
            max_face_tolerance: None,
            max_micro_edges: None,
            max_sliver_faces: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t0_shape_is_frozen() {
        let cases = t0();
        let preset: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../bench/robustness/presets/fillet-foundation-t0.json"
        ))
        .unwrap();
        assert_eq!(preset["generator"]["seed"], format!("{SEED:016x}"));
        assert_eq!(cases.len(), 68);
        assert_eq!(
            cases
                .iter()
                .filter(|item| matches!(item.variant.name, VariantName::Base))
                .count(),
            36
        );
        assert_eq!(
            cases
                .iter()
                .filter(|item| matches!(item.case.expected_domain, ExpectedDomain::Supported))
                .count(),
            36
        );
    }

    #[test]
    fn splitmix_is_frozen() {
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(rng.next_f64().to_bits(), 0x3fdb_9e27_9aa8_6e58);
    }
}
