//! Count typed DTO metadata without serializing another document-shaped buffer.
use onecad_app::dto::DocumentProjection;
struct Counter {
    bytes: usize,
}
impl Counter {
    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        crate::budget::charge(&mut self.bytes, bytes, 2 << 20, "projection metadata")
    }
    fn text(&mut self, value: &str) -> Result<(), String> {
        self.charge(
            value
                .len()
                .checked_add(32)
                .ok_or("metadata size overflow")?,
        )
    }
    fn optional(&mut self, value: Option<&str>) -> Result<(), String> {
        if let Some(value) = value {
            self.text(value)?;
        }
        Ok(())
    }
    fn value(&mut self, value: &serde_json::Value, depth: usize) -> Result<(), String> {
        if depth > 32 {
            return Err("Native diagnostic metadata nesting exceeded".into());
        }
        self.charge(64)?;
        match value {
            serde_json::Value::String(value) => self.text(value)?,
            serde_json::Value::Array(values) => {
                for value in values {
                    self.value(value, depth + 1)?;
                }
            }
            serde_json::Value::Object(values) => {
                for (key, value) in values {
                    self.text(key)?;
                    self.value(value, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
pub fn check(projection: &DocumentProjection) -> Result<(), String> {
    if projection.bodies.len() > crate::budget::MAX_BODIES
        || projection.sketches.len() > 128
        || projection.features.len() > 512
        || projection.variables.len() > 1024
        || projection.datums.len() > 256
    {
        return Err("Native scene object count budget exceeded".into());
    }
    let mut count = Counter {
        bytes: std::mem::size_of::<DocumentProjection>(),
    };
    for text in [
        &projection.document_id,
        &projection.runtime_session,
        &projection.title,
        &projection.geometry_source,
    ] {
        count.text(text)?;
    }
    count.optional(projection.undo_label.as_deref())?;
    count.optional(projection.redo_label.as_deref())?;
    for (key, body) in &projection.bodies {
        count.charge(256)?;
        for text in [key, &body.id, &body.name] {
            count.text(text)?;
        }
        for face in body.face_colors.keys() {
            count.text(face)?;
            count.charge(64)?;
        }
    }
    for (key, sketch) in &projection.sketches {
        count.charge(256)?;
        for text in [key, &sketch.id, &sketch.name, &sketch.geometry_token] {
            count.text(text)?;
        }
        count.optional(sketch.solve_geometry_token.as_deref())?;
        if let Some(face) = &sketch.host_face {
            count.text(&face.body_id)?;
            count.text(&face.element_id)?;
        }
    }
    for feature in &projection.features {
        feature_metadata(&mut count, feature)?;
    }
    for variable in &projection.variables {
        count.charge(256)?;
        for text in [&variable.id, &variable.name, &variable.dimension] {
            count.text(text)?;
        }
        count.optional(variable.expr.as_deref())?;
        count.optional(variable.error.as_deref())?;
    }
    for (key, datum) in &projection.datums {
        count.charge(256)?;
        for text in [
            key,
            &datum.id,
            &datum.name,
            &datum.kind,
            &datum.base_plane_id,
        ] {
            count.text(text)?;
        }
    }
    Ok(())
}
fn feature_metadata(
    count: &mut Counter,
    feature: &onecad_app::dto::FeatureDto,
) -> Result<(), String> {
    count.charge(512)?;
    for text in [
        &feature.id,
        &feature.op_type,
        &feature.label,
        &feature.value_text,
    ] {
        count.text(text)?;
    }
    for text in [
        feature.primary_expr.as_deref(),
        feature.primary_value_kind.as_deref(),
        feature.status_message.as_deref(),
    ] {
        count.optional(text)?;
    }
    for diagnostic in &feature.diagnostics {
        count.charge(128)?;
        count.text(&diagnostic.code)?;
        count.text(&diagnostic.message)?;
        count.optional(diagnostic.stage.as_deref())?;
        count.optional(diagnostic.reason_code.as_deref())?;
        if let Some(evidence) = &diagnostic.evidence {
            count.value(evidence, 0)?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_typed_title_fails_without_json_copy() {
        let mut projection = DocumentProjection::empty();
        projection.title = "x".repeat(2 << 20);
        assert!(check(&projection).is_err());
        projection.title = "Prepared profile".into();
        assert!(check(&projection).is_ok());
    }
}
