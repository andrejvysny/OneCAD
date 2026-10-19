//! Validate the entire authoritative correction before applying any member.

use std::collections::BTreeSet;
use std::str::FromStr;

use onecad_core::ids::EntityId;
use onecad_core::math::Vec2;
use onecad_core::sketch::{CurveParams, Sketch, SketchEntity};
use serde_json::Value;

use crate::dto::{CurveParamsDto, SketchUpsertDto};
mod handles;

pub(super) fn validate_wire(sketch: &Sketch, result: &Value) -> Result<(), String> {
    if result.get("sketchId").and_then(Value::as_str) != Some(&sketch.id.to_string()) {
        return Err("SketchUpsert: response belongs to another sketch".into());
    }
    if result.get("upserted").and_then(Value::as_bool) != Some(true)
        || result
            .get("sketchRevision")
            .and_then(Value::as_u64)
            .is_none_or(|r| r == 0)
    {
        return Err("SketchUpsert: invalid upsert receipt".into());
    }
    for (id, position) in object(result, "positions")? {
        let values = position
            .as_array()
            .filter(|v| v.len() == 2)
            .ok_or_else(|| format!("SketchUpsert: invalid point coordinates for {id}"))?;
        for value in values {
            finite(value, id)?;
        }
    }
    for (id, curve) in object(result, "curves")? {
        if !matches!(
            sketch.get_entity(entity(id)?),
            Some(SketchEntity::Circle { .. } | SketchEntity::Arc { .. })
        ) {
            return Err(format!(
                "SketchUpsert: {id} is not a supported sketch curve"
            ));
        }
        let members = curve
            .as_object()
            .ok_or_else(|| format!("SketchUpsert: invalid curve parameters for {id}"))?;
        // Unknown members remain forward-compatible; understood scalars are strict.
        for key in ["radius", "startAngle", "endAngle"] {
            if let Some(value) = members.get(key) {
                finite(value, id)?;
            }
        }
    }
    Ok(())
}

fn object<'a>(result: &'a Value, key: &str) -> Result<Vec<(&'a String, &'a Value)>, String> {
    match result.get(key) {
        None => Ok(Vec::new()),
        Some(value) => value
            .as_object()
            .map(|map| map.iter().collect())
            .ok_or_else(|| format!("SketchUpsert: {key} must be an object")),
    }
}

fn finite(value: &Value, id: &str) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("SketchUpsert: invalid numeric parameter for {id}"))
}

pub(crate) fn adopted(sketch: &Sketch, result: &SketchUpsertDto) -> Result<Sketch, String> {
    super::sketch_truth::validate(sketch, result)?;
    if result.sketch_id != sketch.id.to_string() {
        return Err("SketchUpsert: response belongs to another sketch".into());
    }
    let locked: BTreeSet<_> = sketch
        .entities()
        .iter()
        .filter(|e| e.is_reference_locked())
        .flat_map(|e| std::iter::once(e.id()).chain(e.referenced_entities()))
        .collect();
    let mut after = sketch.clone();
    let endpoints = apply_positions(&mut after, result)?;
    for (key, params) in &result.solved_curves {
        let id = entity(key)?;
        validate_curve(sketch.get_entity(id), params, key)?;
        after.apply_solved_curves(&[(
            id,
            CurveParams {
                radius: params.radius,
                start_angle: params.start_angle,
                end_angle: params.end_angle,
            },
        )]);
    }
    for id in locked {
        if after.get_entity(id) != sketch.get_entity(id) {
            return Err(format!("SketchUpsert: locked entity {id} moved"));
        }
    }
    for (id, start, position) in endpoints {
        handles::validate_endpoint(&after, id, start, position)?;
    }
    handles::validate_fixed(sketch, &after)?;
    Ok(after)
}

/// The derived Arc endpoint position the accepted document geometry defines.
pub(crate) fn arc_endpoint(sketch: &Sketch, id: EntityId, start: bool) -> Result<Vec2, String> {
    handles::endpoint(sketch, id, start)
}

fn apply_positions(
    after: &mut Sketch,
    result: &SketchUpsertDto,
) -> Result<Vec<(EntityId, bool, Vec2)>, String> {
    let mut points = std::collections::BTreeMap::new();
    let mut endpoints = Vec::new();
    for (key, xy) in &result.solved_positions {
        let position = Vec2::new(xy[0], xy[1])
            .ok_or_else(|| format!("SketchUpsert: non-finite position for {key}"))?;
        match handles::resolve(after, key)? {
            handles::Handle::Point(id) => handles::insert_point(&mut points, id, position)?,
            handles::Handle::ArcEndpoint(id, start) => endpoints.push((id, start, position)),
        }
    }
    after.apply_solved_positions(&points.into_iter().collect::<Vec<_>>());
    Ok(endpoints)
}

fn entity(key: &str) -> Result<EntityId, String> {
    EntityId::from_str(key).map_err(|_| format!("SketchUpsert: invalid entity id {key}"))
}

fn validate_curve(
    entity: Option<&SketchEntity>,
    params: &CurveParamsDto,
    key: &str,
) -> Result<(), String> {
    let members = [params.radius, params.start_angle, params.end_angle];
    if members.iter().flatten().any(|v| !v.is_finite()) || params.radius.is_some_and(|r| r < 0.0) {
        return Err(format!("SketchUpsert: invalid curve parameters for {key}"));
    }
    match entity {
        Some(SketchEntity::Arc { .. }) => Ok(()),
        Some(SketchEntity::Circle { .. })
            if params.start_angle.is_none() && params.end_angle.is_none() =>
        {
            Ok(())
        }
        _ => Err(format!(
            "SketchUpsert: incompatible curve parameters for {key}"
        )),
    }
}

#[cfg(test)]
mod tests;
