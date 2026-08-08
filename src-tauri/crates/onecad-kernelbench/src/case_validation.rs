use crate::case::{
    AdjacencyRelation, AnchorKind, Case, GeometryType, Limits, Metamorph, QualityLimits,
    SearchSpec, SelectorMode, TopologyRole, Validator, ValidatorType,
};
use crate::{AppError, AppResult};

pub(crate) fn validate(case: &Case) -> AppResult<()> {
    validate_geometry(case)?;
    validate_tags(&case.geometry_tags)?;
    validate_selector(case)?;
    validate_validators(&case.validators)?;
    validate_metamorphs(&case.metamorphs)?;
    validate_search(case.search.as_ref())?;
    validate_limits(&case.limits)
}

fn validate_geometry(case: &Case) -> AppResult<()> {
    let dimensions = &case.geometry_recipe.dimensions;
    if dimensions.len() != 3
        || dimensions.iter().any(|value| !positive(*value, 1e12))
        || !positive(case.operation.radius, 1e12)
        || case.geometry_recipe.feature_index > 65_535
    {
        return Err(AppError::cli("geometry values out of bounds"));
    }
    Ok(())
}

fn validate_tags(tags: &[String]) -> AppResult<()> {
    if tags.is_empty() || tags.len() > 32 || tags.iter().any(|tag| !safe_id(tag)) {
        return Err(AppError::cli("geometry tags out of bounds"));
    }
    let mut unique = tags.to_vec();
    unique.sort_unstable();
    unique.dedup();
    (unique.len() == tags.len())
        .then_some(())
        .ok_or_else(|| AppError::cli("geometry tags must be unique"))
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

fn validate_validators(validators: &[Validator]) -> AppResult<()> {
    if validators.is_empty() || validators.len() > 32 {
        return Err(AppError::cli("validators out of bounds"));
    }
    for validator in validators {
        validate_validator(validator)?;
    }
    Ok(())
}

fn validate_validator(validator: &Validator) -> AppResult<()> {
    let valid = if simple_validator(validator.validator_type) {
        simple_fields_valid(validator)
    } else if tolerance_validator(validator.validator_type) {
        tolerance_fields_valid(validator)
    } else {
        metamorph_fields_valid(validator)
    };
    valid
        .then_some(())
        .ok_or_else(|| AppError::cli("invalid validator fields"))
}

fn simple_validator(kind: ValidatorType) -> bool {
    matches!(
        kind,
        ValidatorType::ConstantRadius
            | ValidatorType::GeneratedBlendFace
            | ValidatorType::CylindricalRadius
            | ValidatorType::G1BoundaryTangency
            | ValidatorType::MaterialChange
            | ValidatorType::RemoteSupportsUnchanged
            | ValidatorType::DeepAudit
    )
}

fn tolerance_validator(kind: ValidatorType) -> bool {
    matches!(
        kind,
        ValidatorType::RadiusTolerance
            | ValidatorType::TangencyTolerance
            | ValidatorType::MaterialTolerance
    )
}

fn simple_fields_valid(validator: &Validator) -> bool {
    validator.absolute.is_none()
        && validator.relative.is_none()
        && validator.surface_samples.is_none()
        && validator.point_tolerance.is_none()
}

fn tolerance_fields_valid(validator: &Validator) -> bool {
    validator
        .absolute
        .is_some_and(|value| bounded(value, 0.0, 1e12))
        && validator
            .relative
            .is_some_and(|value| bounded(value, 0.0, 1.0))
        && validator.direction.is_none()
        && validator.surface_samples.is_none()
        && validator.point_tolerance.is_none()
}

fn metamorph_fields_valid(validator: &Validator) -> bool {
    validator.direction.is_none()
        && validator.absolute.is_none()
        && validator.relative.is_none()
        && validator
            .surface_samples
            .is_some_and(|value| (1..=4096).contains(&value))
        && validator
            .point_tolerance
            .is_some_and(|value| bounded(value, 0.0, 1e12))
}

fn validate_metamorphs(metamorphs: &[Metamorph]) -> AppResult<()> {
    if metamorphs.len() > 16 {
        return Err(AppError::cli("metamorphs out of bounds"));
    }
    for metamorph in metamorphs {
        let valid = match metamorph {
            Metamorph::Translation { vector } => vector_valid(vector),
            Metamorph::Rotation {
                angle_degrees,
                axis,
                ..
            } => bounded(*angle_degrees, -360.0, 360.0) && vector_valid(axis) && nonzero(axis),
        };
        if !valid {
            return Err(AppError::cli("invalid metamorph"));
        }
    }
    Ok(())
}

fn validate_search(search: Option<&SearchSpec>) -> AppResult<()> {
    let Some(search) = search else {
        return Ok(());
    };
    let valid = positive(search.known_success, 1e12)
        && positive(search.upper_bound, 1e12)
        && search.known_success <= search.upper_bound
        && search.growth_factor > 1.0
        && search.growth_factor <= 16.0
        && (8..=4096).contains(&search.max_probes)
        && search.relative_precision > 0.0
        && search.relative_precision <= 0.01
        && search.absolute_precision > 0.0
        && search.absolute_precision <= 1.0
        && search.relative_offsets == [1e-2, 1e-4, 1e-6, 1e-8];
    valid
        .then_some(())
        .ok_or_else(|| AppError::cli("invalid search bounds"))
}

fn validate_selector(case: &Case) -> AppResult<()> {
    let selector = &case.operation.selector;
    let count_valid = match selector.mode {
        SelectorMode::Single => selector.anchors.len() == 1,
        SelectorMode::Disconnected | SelectorMode::Multiple => selector.anchors.len() >= 2,
        SelectorMode::CornerIncident => {
            selector.anchors.len() == 1
                && matches!(selector.anchors[0].kind, AnchorKind::CommonVertex)
        }
    };
    let relation_valid = relation_matches(selector.mode, selector.adjacency.relation);
    let provenance_valid = selector.provenance.generator == "fillet-foundation"
        && matches_recipe_type(
            selector.provenance.recipe_type,
            case.geometry_recipe.recipe_type,
        )
        && selector.provenance.feature_index == case.geometry_recipe.feature_index;
    let role_valid =
        topology_role_matches(case.geometry_recipe.recipe_type, selector.topology_role);
    // Non-corner modes match one surface descriptor to each selected edge
    // anchor 1:1 (see Geometry.cpp's edge selection); corner-incident
    // selects a vertex-incident group whose edge count is independent of
    // the single commonVertex anchor, so it is exempt.
    let descriptor_count_valid = matches!(selector.mode, SelectorMode::CornerIncident)
        || selector.anchors.len() == selector.surface_descriptors.len();
    let bounded = !selector.anchors.is_empty()
        && selector.anchors.len() <= 64
        && !selector.surface_descriptors.is_empty()
        && selector.surface_descriptors.len() <= 64
        && selector.provenance.feature_index <= 65_535
        && selector
            .anchors
            .iter()
            .all(|item| vector_valid(&item.point))
        && selector
            .surface_descriptors
            .iter()
            .all(|item| item.adjacent_surface_kinds.len() == 2);
    (count_valid
        && relation_valid
        && provenance_valid
        && role_valid
        && descriptor_count_valid
        && bounded)
        .then_some(())
        .ok_or_else(|| AppError::cli("invalid semantic selector"))
}

fn matches_recipe_type(a: GeometryType, b: GeometryType) -> bool {
    matches!(
        (a, b),
        (GeometryType::Box, GeometryType::Box)
            | (GeometryType::Valence3, GeometryType::Valence3)
            | (GeometryType::Valence4, GeometryType::Valence4)
            | (GeometryType::OverflowWedge, GeometryType::OverflowWedge)
    )
}

fn topology_role_matches(recipe_type: GeometryType, role: TopologyRole) -> bool {
    matches!(
        (recipe_type, role),
        (GeometryType::Box, TopologyRole::VerticalEdge)
            | (
                GeometryType::Valence3 | GeometryType::Valence4,
                TopologyRole::CornerIncidentEdge
            )
            | (GeometryType::OverflowWedge, TopologyRole::OverflowEdge)
    )
}

fn relation_matches(mode: SelectorMode, relation: AdjacencyRelation) -> bool {
    matches!(
        (mode, relation),
        (SelectorMode::Single, AdjacencyRelation::Single)
            | (
                SelectorMode::Disconnected,
                AdjacencyRelation::PairwiseDisconnected
            )
            | (SelectorMode::Multiple, AdjacencyRelation::Unconstrained)
            | (
                SelectorMode::CornerIncident,
                AdjacencyRelation::IncidentToCommonVertex
            )
    )
}

fn validate_limits(limits: &Limits) -> AppResult<()> {
    let resources = &limits.resources;
    let valid = (1..=3_600_000).contains(&resources.wall_time_ms)
        && (16 * 1024 * 1024..=64 * 1024 * 1024 * 1024).contains(&resources.address_space_bytes)
        && (1024..=16_777_216).contains(&resources.stdout_bytes)
        && (1024..=16_777_216).contains(&resources.stderr_bytes)
        && (1024..=1_073_741_824).contains(&resources.artifact_bytes)
        && quality_values_valid(&limits.quality);
    valid
        .then_some(())
        .ok_or_else(|| AppError::cli("resource limit out of bounds"))
}

fn quality_values_valid(quality: &QualityLimits) -> bool {
    [
        quality.max_vertex_tolerance,
        quality.max_edge_tolerance,
        quality.max_face_tolerance,
    ]
    .into_iter()
    .flatten()
    .all(|value| bounded(value, 0.0, 1e12))
        && quality
            .max_micro_edges
            .is_none_or(|value| value <= 10_000_000)
        && quality
            .max_sliver_faces
            .is_none_or(|value| value <= 10_000_000)
}

fn vector_valid(vector: &[f64; 3]) -> bool {
    vector.iter().all(|value| bounded(*value, -1e12, 1e12))
}

fn nonzero(vector: &[f64; 3]) -> bool {
    vector.iter().any(|value| *value != 0.0)
}

fn positive(value: f64, maximum: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= maximum
}

fn bounded(value: f64, minimum: f64, maximum: f64) -> bool {
    value.is_finite() && value >= minimum && value <= maximum
}
