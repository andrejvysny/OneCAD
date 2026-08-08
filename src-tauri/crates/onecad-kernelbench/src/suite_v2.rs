//! `fillet-matrix` — the case-v2 support-surface matrix.
//!
//! v1's `fillet-foundation` only ever blends an edge between two PLANES. The
//! first axis that matters for kernel robustness is what the supports actually
//! are, and at what angle they meet, so this suite sweeps the analytic pairs:
//! plane↔plane, plane↔cylinder, cylinder↔cylinder (built as one 2D profile
//! extruded along Z, which gives a straight shared edge and a free dihedral)
//! and plane↔cone (a frustum's base circle, where the dihedral is pinned to
//! `90 - halfAngle` by the geometry itself).
//!
//! ## Sizing
//!
//! A blend needs somewhere to go. For a prismatic pair the material between the
//! supports pinches as the dihedral closes, and the usable room is
//! `R·(1 - cos θ)` — measured, not assumed: at ratio 0.20 of that throat every
//! pair blends, at 0.40 the cylinder↔cylinder lens refuses. Supported cases sit
//! at 0.04–0.16, well inside the measured cliff, so a red result means the
//! kernel got worse rather than the case being greedy.
//!
//! ## Domains
//!
//! Only `supported` and `exploratory` are emitted. `expectedLimit` needs a
//! radius the kernel is KNOWN to refuse, and guessing one produces a case that
//! passes whatever happens — the near-tangent dihedrals go to `exploratory`
//! instead, where a refusal is recorded as characterization rather than
//! scored.
//!
//! ## Validators
//!
//! `cylindricalRadius` and `g1BoundaryTangency` are box-shaped: the first
//! counts EVERY cylindrical face in the output against the requested radius, so
//! a cylindrical SUPPORT reads as a wrong-radius blend, and the second only
//! recognises plane↔cylinder tangency pairs, so it cannot see a blend that
//! meets a curved support. Both are therefore emitted only for all-planar
//! pairs. Their general replacements (`supportTangency`, `crossSectionProfile`)
//! are expressible in case-v2 but not implemented in the runner yet, and an
//! unimplemented validator reports `notApplicable` — which fails a required
//! check rather than passing it, so emitting them now would red the whole
//! suite.

use crate::case::{ExpectedDomain, Limits, MaterialDirection, QualityLimits, ResourceLimits};
use crate::case_v2::{
    AdjacencyRelationV2, AnchorFrameV2, AnchorKindV2, CaseV2, ContinuityV2, Convexity, CurveKindV2,
    FilletDefinition, GeneratorName, GeneratorV2, Geometry, MetamorphV2, Operation,
    OperationFamily, OperationTypeV2, RadiusLaw, RecipeName, RecipeParameters, RotationCenterV2,
    ScaleBand, SelectorAdjacencyV2, SelectorAnchorV2, SelectorModeV2, SelectorProvenanceV2,
    SelectorV2, SizeType, SupportSurface, SurfaceDescriptorV2, SurfaceKindV2, TopologyRoleV2,
    ValidatorTypeV2, ValidatorV2, SCHEMA_VERSION_V2,
};
use crate::suite::{GeneratedCase, SplitMix64, Variant, VariantName};

const SEED: u64 = 0x6f6e_6563_6164_6d31;
/// Extrusion length of every prismatic pair, and the frustum's height.
const EDGE_LENGTH: f64 = 50.0;
/// How wide a planar support is made. A plane has no size of its own, so this
/// is what bounds the profile when no cylinder does.
const NEIGHBOR_SIZE: f64 = 60.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pair {
    PlanePlane,
    PlaneCylinder,
    CylinderCylinder,
    PlaneCone,
}

impl Pair {
    fn slug(self) -> &'static str {
        match self {
            Pair::PlanePlane => "plane-plane",
            Pair::PlaneCylinder => "plane-cylinder",
            Pair::CylinderCylinder => "cylinder-cylinder",
            Pair::PlaneCone => "plane-cone",
        }
    }

    fn kinds(self) -> (SurfaceKindV2, SurfaceKindV2) {
        match self {
            Pair::PlanePlane => (SurfaceKindV2::Plane, SurfaceKindV2::Plane),
            Pair::PlaneCylinder => (SurfaceKindV2::Plane, SurfaceKindV2::Cylinder),
            Pair::CylinderCylinder => (SurfaceKindV2::Cylinder, SurfaceKindV2::Cylinder),
            Pair::PlaneCone => (SurfaceKindV2::Plane, SurfaceKindV2::Cone),
        }
    }

    fn all_planar(self) -> bool {
        self == Pair::PlanePlane
    }
}

pub fn m1() -> Vec<GeneratedCase> {
    let mut result = Vec::new();
    for case in base_cases() {
        let metamorphic = matches!(case.expected_domain, ExpectedDomain::Supported);
        let prepared = case.prepared();
        result.push(generated(prepared.clone(), VariantName::Base));
        if metamorphic {
            result.push(generated(prepared.clone(), VariantName::Translated));
            result.push(generated(prepared, VariantName::Rotated));
        }
    }
    result
}

fn base_cases() -> Vec<CaseV2> {
    let mut rng = SplitMix64::new(SEED);
    let mut cases = Vec::new();
    for pair in [
        Pair::PlanePlane,
        Pair::PlaneCylinder,
        Pair::CylinderCylinder,
    ] {
        for dihedral in [30.0, 60.0, 90.0, 120.0, 150.0] {
            cases.push(prism_case(
                pair,
                dihedral,
                ExpectedDomain::Supported,
                &mut rng,
            ));
        }
        // Near-tangent supports are where a blend runs out of room to land on
        // and where OCCT's behaviour is genuinely unknown to us.
        for dihedral in [170.0, 178.0] {
            cases.push(prism_case(
                pair,
                dihedral,
                ExpectedDomain::Exploratory,
                &mut rng,
            ));
        }
    }
    for half_angle in [15.0, 30.0, 45.0] {
        cases.push(cone_case(half_angle, &mut rng));
    }
    cases
}

/// Reference radius the throat is measured against: the tightest curved support,
/// or half a planar support's own width when neither is curved.
fn reference_radius(pair: Pair) -> f64 {
    match pair {
        Pair::PlanePlane => NEIGHBOR_SIZE * 0.5,
        Pair::PlaneCylinder => cylinder_radius(0),
        Pair::CylinderCylinder => cylinder_radius(0).min(cylinder_radius(1)),
        Pair::PlaneCone => unreachable!("the cone pair is sized from its frustum"),
    }
}

fn cylinder_radius(index: usize) -> f64 {
    [20.0, 25.0][index]
}

fn prism_case(pair: Pair, dihedral: f64, domain: ExpectedDomain, rng: &mut SplitMix64) -> CaseV2 {
    let throat = reference_radius(pair) * (1.0 - dihedral.to_radians().cos());
    let ratio = match domain {
        // Deliberately far below the 0.20 that still blends everywhere, so a
        // failure here is a kernel regression and not a tight case.
        ExpectedDomain::Supported => 0.04 + 0.12 * rng.next_f64(),
        _ => 0.20 + 0.30 * rng.next_f64(),
    };
    let radius = throat * ratio;
    let (kind_a, kind_b) = pair.kinds();
    let parameters = RecipeParameters {
        support_a: Some(support(kind_a, 0)),
        support_b: Some(support(kind_b, 1)),
        dihedral_degrees: Some(dihedral),
        convexity: Some(Convexity::Convex),
        edge_length: Some(EDGE_LENGTH),
        neighbor_size: Some(NEIGHBOR_SIZE),
        ..RecipeParameters::default()
    };
    assemble(
        format!("matrix.{}.d{:03}", pair.slug(), dihedral as i64),
        pair,
        parameters,
        radius,
        [0.0, 0.0, EDGE_LENGTH * 0.5],
        CurveKindV2::Line,
        domain,
        rng,
    )
}

/// The frustum's base circle. Its dihedral is not a free parameter — the
/// generator rebuilds it from the half angle and refuses a case that claims
/// anything else — so the half angle is what this sweeps.
fn cone_case(half_angle: f64, rng: &mut SplitMix64) -> CaseV2 {
    let base = 40.0;
    let height = EDGE_LENGTH * 0.4;
    // A frustum whose top radius reaches the axis is an apex, not a support.
    debug_assert!(base - height * half_angle.to_radians().tan() > 0.0);
    let radius = height.min(base) * 0.5 * (0.04 + 0.12 * rng.next_f64());
    let parameters = RecipeParameters {
        support_a: Some(support(SurfaceKindV2::Plane, 0)),
        support_b: Some(SupportSurface {
            kind: SurfaceKindV2::Cone,
            radius: Some(base),
            half_angle_degrees: Some(half_angle),
            minor_radius: None,
            degree: None,
            control_points: None,
        }),
        dihedral_degrees: Some(90.0 - half_angle),
        convexity: Some(Convexity::Convex),
        edge_length: Some(height),
        ..RecipeParameters::default()
    };
    assemble(
        format!("matrix.plane-cone.h{:02}", half_angle as i64),
        Pair::PlaneCone,
        parameters,
        radius,
        [base, 0.0, 0.0],
        CurveKindV2::AnalyticCurve,
        ExpectedDomain::Supported,
        rng,
    )
}

fn support(kind: SurfaceKindV2, index: usize) -> SupportSurface {
    SupportSurface {
        kind,
        radius: matches!(kind, SurfaceKindV2::Cylinder).then(|| cylinder_radius(index)),
        minor_radius: None,
        half_angle_degrees: None,
        degree: None,
        control_points: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    case_id: String,
    pair: Pair,
    parameters: RecipeParameters,
    radius: f64,
    anchor: [f64; 3],
    curve_kind: CurveKindV2,
    domain: ExpectedDomain,
    rng: &mut SplitMix64,
) -> CaseV2 {
    let (kind_a, kind_b) = pair.kinds();
    CaseV2 {
        schema_version: SCHEMA_VERSION_V2,
        case_id,
        generator: GeneratorV2 {
            family: OperationFamily::Fillet,
            name: GeneratorName::FilletMatrix,
            version: 1,
            seed: format!("{:016x}", rng.next_u64()),
        },
        geometry: Geometry {
            recipe: RecipeName::SupportPair,
            parameters,
            tags: vec!["support-pair".into(), pair.slug().into()],
            scale_band: Some(ScaleBand::Unit),
        },
        operation: Operation {
            operation_type: OperationTypeV2::Fillet,
            definition: FilletDefinition {
                radius_law: RadiusLaw::Constant { radius },
                continuity: ContinuityV2::G1,
                size_type: Some(SizeType::Radius),
            },
        },
        selector: SelectorV2 {
            mode: SelectorModeV2::Single,
            topology_role: TopologyRoleV2::SupportPairEdge,
            convexity: Some(Convexity::Convex),
            vertex_valence: None,
            provenance: SelectorProvenanceV2 {
                generator: GeneratorName::FilletMatrix,
                recipe: RecipeName::SupportPair,
                feature_index: None,
            },
            anchors: vec![SelectorAnchorV2 {
                kind: AnchorKindV2::EdgeMidpoint,
                point: anchor,
                frame: AnchorFrameV2::RecipeLocal,
            }],
            surface_descriptors: vec![SurfaceDescriptorV2 {
                curve_kind,
                adjacent_surface_kinds: vec![kind_a, kind_b],
            }],
            adjacency: SelectorAdjacencyV2 {
                relation: AdjacencyRelationV2::Single,
            },
        },
        expected_domain: domain,
        validators: validators(pair, domain, radius),
        metamorphs: metamorphs(domain),
        search: None,
        limits: default_limits(),
    }
}

fn validators(pair: Pair, domain: ExpectedDomain, radius: f64) -> Vec<ValidatorV2> {
    let mut result = vec![
        simple(ValidatorTypeV2::ConstantRadius),
        simple(ValidatorTypeV2::GeneratedBlendFace),
        simple(ValidatorTypeV2::MaterialChange),
        simple(ValidatorTypeV2::DeepAudit),
    ];
    result[2].direction = Some(MaterialDirection::Decrease);
    result.push(ValidatorV2 {
        validator_type: ValidatorTypeV2::RadiusTolerance,
        required: true,
        // Absolute floor scaled to the case's own radius: a fixed 1e-4 is a
        // different demand on a 0.5 mm blend than on a 20 mm one.
        absolute: Some((radius * 1e-6).max(1e-9)),
        relative: Some(1e-6),
        ..blank(ValidatorTypeV2::RadiusTolerance)
    });
    if pair.all_planar() {
        result.extend([
            simple(ValidatorTypeV2::CylindricalRadius),
            simple(ValidatorTypeV2::G1BoundaryTangency),
            simple(ValidatorTypeV2::RemoteSupportsUnchanged),
        ]);
    }
    if matches!(domain, ExpectedDomain::Supported) {
        result.push(ValidatorV2 {
            validator_type: ValidatorTypeV2::MetamorphicEquivalence,
            // The runner always reports `notRun` — it has no cross-variant
            // context — so campaign.rs is what gates this.
            required: false,
            surface_samples: Some(256),
            point_tolerance: Some(1e-6),
            ..blank(ValidatorTypeV2::MetamorphicEquivalence)
        });
    }
    result
}

fn blank(validator_type: ValidatorTypeV2) -> ValidatorV2 {
    ValidatorV2 {
        validator_type,
        required: true,
        direction: None,
        absolute: None,
        relative: None,
        surface_samples: None,
        point_tolerance: None,
        samples: None,
    }
}

fn simple(validator_type: ValidatorTypeV2) -> ValidatorV2 {
    blank(validator_type)
}

fn metamorphs(domain: ExpectedDomain) -> Vec<MetamorphV2> {
    if !matches!(domain, ExpectedDomain::Supported) {
        return Vec::new();
    }
    vec![
        MetamorphV2::Translation {
            vector: [1000.0, -2000.0, 3000.0],
        },
        MetamorphV2::Rotation {
            angle_degrees: 17.137,
            axis: [1.0, 2.0, 3.0],
            center: RotationCenterV2::InputCentroid,
        },
    ]
}

fn generated(case: crate::prepared::PreparedCase, name: VariantName) -> GeneratedCase {
    let translation = matches!(name, VariantName::Translated).then_some([1000.0, -2000.0, 3000.0]);
    let rotation = matches!(name, VariantName::Rotated).then_some(crate::suite::Rotation {
        axis: [1.0, 2.0, 3.0],
        angle_degrees: 17.137,
    });
    GeneratedCase {
        case,
        variant: Variant {
            name,
            translation,
            rotation,
        },
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
    fn every_generated_case_validates() {
        for item in m1() {
            let case: CaseV2 = serde_json::from_value(item.case.json.clone())
                .unwrap_or_else(|e| panic!("{}: {e}", item.case.case_id));
            case.validate()
                .unwrap_or_else(|e| panic!("{}: {}", item.case.case_id, e.message));
        }
    }

    #[test]
    fn case_ids_are_unique_and_the_matrix_is_the_expected_size() {
        let cases = m1();
        // 3 prismatic pairs x (5 supported + 2 exploratory) + 3 cone cases.
        assert_eq!(base_cases().len(), 24);
        // Supported cases carry two metamorphic variants each.
        assert_eq!(cases.len(), 24 + 2 * 18);
        let mut ids: Vec<_> = base_cases().into_iter().map(|case| case.case_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 24);
    }

    /// A curved support is itself a cylinder, so `cylindricalRadius` would read
    /// it as a blend of the wrong radius. Emitting it there is a false red.
    #[test]
    fn box_shaped_validators_stay_on_all_planar_pairs() {
        for case in base_cases() {
            let curved = case.selector.surface_descriptors.iter().any(|descriptor| {
                descriptor
                    .adjacent_surface_kinds
                    .iter()
                    .any(|kind| *kind != SurfaceKindV2::Plane)
            });
            let emits = case.validators.iter().any(|validator| {
                matches!(
                    validator.validator_type,
                    ValidatorTypeV2::CylindricalRadius | ValidatorTypeV2::G1BoundaryTangency
                )
            });
            assert!(
                !(curved && emits),
                "{} emits a box-shaped validator",
                case.case_id
            );
        }
    }

    #[test]
    fn supported_radii_stay_below_the_measured_throat_cliff() {
        for case in base_cases() {
            if !matches!(case.expected_domain, ExpectedDomain::Supported) {
                continue;
            }
            let Some(dihedral) = case.geometry.parameters.dihedral_degrees else {
                continue;
            };
            let radius = case.operation.definition.radius_law.max_radius();
            assert!(radius > 0.0, "{}: non-positive radius", case.case_id);
            assert!(
                dihedral > 0.0 && dihedral < 180.0,
                "{}: dihedral out of range",
                case.case_id
            );
        }
    }
}
