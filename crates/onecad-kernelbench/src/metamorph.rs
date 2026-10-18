use serde_json::Value;

pub struct MetamorphComparison {
    pub surface_samples_match: bool,
    pub point_classification_match: bool,
}

/// Compares a metamorphic variant's runner-reported shape evidence against
/// its base variant's, inverse-transforming the variant's points back into
/// the base frame first. Returns `None` when either side has no evidence to
/// compare (e.g. both variants correctly refused an expected-limit case),
/// which the caller must treat as "not a mismatch", not a pass.
pub fn compare(base: &Value, variant: &Value, tolerance: f64) -> Option<MetamorphComparison> {
    let base_signature = base
        .get("shapeSignature")
        .filter(|value| !value.is_null())?;
    let variant_signature = variant
        .get("shapeSignature")
        .filter(|value| !value.is_null())?;
    let variant_spec = variant.get("campaignVariant")?;
    let center = vector3(base_signature.get("rotationCenter")?)?;

    let base_faces = labeled_points(base_signature.get("faceSamples")?, "surfaceKind")?;
    let variant_faces = transformed_points(
        variant_signature.get("faceSamples")?,
        "surfaceKind",
        variant_spec,
        center,
    )?;
    let surface_samples_match = compare_unordered(&base_faces, &variant_faces, tolerance);

    let base_points = labeled_points(base_signature.get("pointClassifications")?, "state")?;
    let variant_points = transformed_points(
        variant_signature.get("pointClassifications")?,
        "state",
        variant_spec,
        center,
    )?;
    // Probe points are generated in a fixed, deterministic sequence (see
    // Geometry.cpp's `make_probe_points`), so index correspondence is valid
    // here and more precise than nearest-match.
    let point_classification_match = compare_ordered(&base_points, &variant_points, tolerance);

    Some(MetamorphComparison {
        surface_samples_match,
        point_classification_match,
    })
}

type LabeledPoint = ([f64; 3], String);

fn labeled_points(value: &Value, label_key: &str) -> Option<Vec<LabeledPoint>> {
    value
        .as_array()?
        .iter()
        .map(|item| {
            let point = vector3(item.get("point")?)?;
            let label = item.get(label_key)?.as_str()?.to_string();
            Some((point, label))
        })
        .collect()
}

fn transformed_points(
    value: &Value,
    label_key: &str,
    variant: &Value,
    center: [f64; 3],
) -> Option<Vec<LabeledPoint>> {
    labeled_points(value, label_key)?
        .into_iter()
        .map(|(point, label)| Some((inverse_transform(point, variant, center)?, label)))
        .collect()
}

fn vector3(value: &Value) -> Option<[f64; 3]> {
    let array = value.as_array()?;
    if array.len() != 3 {
        return None;
    }
    Some([array[0].as_f64()?, array[1].as_f64()?, array[2].as_f64()?])
}

fn inverse_transform(point: [f64; 3], variant: &Value, center: [f64; 3]) -> Option<[f64; 3]> {
    match variant.get("name").and_then(Value::as_str)? {
        "base" => Some(point),
        "translated" | "farOriginTranslated" => {
            Some(sub(point, vector3(variant.get("translation")?)?))
        }
        "rotated" => {
            let rotation = variant.get("rotation")?;
            let axis = normalize(vector3(rotation.get("axis")?)?)?;
            let angle = -rotation.get("angleDegrees")?.as_f64()?.to_radians();
            Some(rotate(point, center, axis, angle))
        }
        "mirrored" => {
            let mirror = variant.get("mirror")?;
            let normal = normalize(vector3(mirror.get("normal")?)?)?;
            Some(mirror_point(point, center, normal))
        }
        "scaled" => {
            let scale_block = variant.get("scale")?;
            let factor = scale_block.get("factor")?.as_f64()?;
            let center = scale_block
                .get("center")
                .and_then(vector3)
                .unwrap_or(center);
            Some(add(center, scale(sub(point, center), 1.0 / factor)))
        }
        "parameterEpsilon" | "edgeOrderPermutation" | "contourSeed" => Some(point),
        _ => None,
    }
}

fn mirror_point(point: [f64; 3], center: [f64; 3], normal: [f64; 3]) -> [f64; 3] {
    let v = sub(point, center);
    let projection = scale(normal, 2.0 * dot(v, normal));
    sub(point, projection)
}

fn compare_unordered(base: &[LabeledPoint], variant: &[LabeledPoint], tolerance: f64) -> bool {
    if base.len() != variant.len() {
        return false;
    }
    let mut used = vec![false; variant.len()];
    base.iter().all(|(point, label)| {
        let found = variant
            .iter()
            .enumerate()
            .position(|(index, (candidate, candidate_label))| {
                !used[index]
                    && candidate_label == label
                    && distance(*point, *candidate) <= tolerance
            });
        match found {
            Some(index) => {
                used[index] = true;
                true
            }
            None => false,
        }
    })
}

fn compare_ordered(base: &[LabeledPoint], variant: &[LabeledPoint], tolerance: f64) -> bool {
    base.len() == variant.len()
        && base
            .iter()
            .zip(variant)
            .all(|((bp, bs), (vp, vs))| bs == vs && distance(*bp, *vp) <= tolerance)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: [f64; 3]) -> Option<[f64; 3]> {
    let magnitude = norm(a);
    (magnitude > 1e-12).then(|| scale(a, 1.0 / magnitude))
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    norm(sub(a, b))
}

/// Rodrigues' rotation formula, matching OCCT's `gp_Trsf::SetRotation`
/// (right-handed rotation about `axis` through `center`).
fn rotate(point: [f64; 3], center: [f64; 3], axis: [f64; 3], angle: f64) -> [f64; 3] {
    let v = sub(point, center);
    let (sin, cos) = angle.sin_cos();
    let rotated = add(
        add(scale(v, cos), scale(cross(axis, v), sin)),
        scale(axis, dot(axis, v) * (1.0 - cos)),
    );
    add(center, rotated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn translation_inverts_exactly() {
        let variant = json!({"name":"translated","translation":[10.0,-5.0,2.0]});
        let point = [1.0, 2.0, 3.0];
        let forward = add(point, [10.0, -5.0, 2.0]);
        let back = inverse_transform(forward, &variant, [0.0, 0.0, 0.0]).unwrap();
        assert!(distance(back, point) < 1e-9);
    }

    #[test]
    fn rotation_inverts_exactly() {
        let center = [1.0, 1.0, 1.0];
        let axis = normalize([1.0, 2.0, 3.0]).unwrap();
        let angle = 17.137_f64.to_radians();
        let point = [4.0, -2.0, 6.0];
        let forward = rotate(point, center, axis, angle);
        let variant = json!({
            "name":"rotated",
            "rotation":{"axis":[1.0,2.0,3.0],"angleDegrees":17.137}
        });
        let back = inverse_transform(forward, &variant, center).unwrap();
        assert!(distance(back, point) < 1e-9);
    }

    #[test]
    fn unordered_compare_matches_regardless_of_order() {
        let base = vec![
            ([0.0, 0.0, 0.0], "plane".to_string()),
            ([1.0, 0.0, 0.0], "cylinder".to_string()),
        ];
        let variant = vec![
            ([1.0, 0.0, 0.0], "cylinder".to_string()),
            ([0.0, 0.0, 0.0], "plane".to_string()),
        ];
        assert!(compare_unordered(&base, &variant, 1e-9));
    }

    #[test]
    fn unordered_compare_rejects_label_mismatch() {
        let base = vec![([0.0, 0.0, 0.0], "plane".to_string())];
        let variant = vec![([0.0, 0.0, 0.0], "cylinder".to_string())];
        assert!(!compare_unordered(&base, &variant, 1e-9));
    }

    #[test]
    fn ordered_compare_requires_same_length() {
        let base = vec![([0.0, 0.0, 0.0], "in".to_string())];
        let variant = vec![];
        assert!(!compare_ordered(&base, &variant, 1e-9));
    }
}
