use serde_json::Value;

use crate::dto::BeginGestureDto;

pub(super) fn begin(gesture_id: u64, result: &Value) -> Result<BeginGestureDto, String> {
    if result.get("gestureId").and_then(Value::as_u64) != Some(gesture_id)
        || result.get("ready").and_then(Value::as_bool) != Some(true)
    {
        return Err("BeginGesture: invalid gesture receipt".into());
    }
    if let Some(offset) = result.get("pointOffset") {
        coordinates(offset)?;
    }
    for (id, at) in object(result, "positions")? {
        coordinates(at).map_err(|message| format!("{message}: {id}"))?;
    }
    for (id, parameters) in object(result, "curves")? {
        let values = parameters
            .as_object()
            .ok_or_else(|| format!("BeginGesture: invalid curve {id}"))?;
        for key in ["radius", "startAngle", "endAngle"] {
            if let Some(value) = values.get(key) {
                finite(value)?;
            }
        }
    }
    Ok(super::wire::parse_begin_gesture(gesture_id, result))
}

fn coordinates(value: &Value) -> Result<(), String> {
    let values = value
        .as_array()
        .filter(|values| values.len() == 2)
        .ok_or_else(|| "BeginGesture: invalid point coordinates".to_string())?;
    for scalar in values {
        finite(scalar)?;
    }
    Ok(())
}

fn finite(value: &Value) -> Result<(), String> {
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .map(|_| ())
        .ok_or_else(|| "BeginGesture: nonfinite or malformed scalar".into())
}

fn object<'a>(result: &'a Value, key: &str) -> Result<Vec<(&'a String, &'a Value)>, String> {
    match result.get(key) {
        None => Ok(Vec::new()),
        Some(value) => value
            .as_object()
            .map(|values| values.iter().collect())
            .ok_or_else(|| format!("BeginGesture: {key} must be an object")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn begin_channels_preserve_finite_offset_and_additive_metadata() {
        let result = json!({"gestureId":5,"ready":true,"pointOffset":[-1,2],
            "positions":{"point":[10,20]},"curves":{"curve":{"radius":3,"future":{"opaque":true}}}});
        let parsed = begin(5, &result).unwrap();
        assert_eq!(parsed.point_offset, Some([-1.0, 2.0]));
        assert_eq!(parsed.positions["point"], [10.0, 20.0]);
        assert_eq!(parsed.curves["curve"].radius, Some(3.0));
    }

    #[test]
    fn begin_refuses_bad_identity_offset_coordinates_and_known_scalars() {
        for extra in [
            json!({"gestureId":6}),
            json!({"ready":false}),
            json!({"pointOffset":[1]}),
            json!({"pointOffset":[1,"bad"]}),
            json!({"positions":{"point":[1,2,3]}}),
            json!({"curves":{"curve":{"radius":"bad"}}}),
        ] {
            let mut result = json!({"gestureId":5,"ready":true});
            result
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(begin(5, &result).is_err(), "{result}");
        }
        assert!(begin(5, &json!({"gestureId":5,"ready":true})).is_ok());
    }
}
