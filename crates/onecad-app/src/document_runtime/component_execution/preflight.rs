//! Bounds the authored definition clone before worker replay admission.
use crate::error::ApiError;
use onecad_core::{
    document::{component::ComponentDef, DocumentSettings},
    io::imports::ImportBlobs,
};
use std::io::{self, Write};

// Existing native capture work reservation; counts serialized structural bytes at
// a conservative factor for cloned collections, strings and sketch indices.
const MAX_ESTIMATED_CLONE: usize = 32 << 20;
const MAX_WIRE_BYTES: usize = MAX_ESTIMATED_CLONE / 32;

#[derive(Default)]
struct Counter {
    bytes: usize,
    depth: usize,
    in_string: bool,
    escaped: bool,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX_WIRE_BYTES)
            .ok_or_else(|| io::Error::other("component replay authored-size limit"))?;
        for byte in bytes {
            if self.in_string {
                if self.escaped {
                    self.escaped = false;
                } else if *byte == b'\\' {
                    self.escaped = true;
                } else if *byte == b'"' {
                    self.in_string = false;
                }
            } else {
                match byte {
                    b'"' => self.in_string = true,
                    b'{' | b'[' => {
                        self.depth += 1;
                        if self.depth > 32 {
                            return Err(io::Error::other("component replay nesting limit"));
                        }
                    }
                    b'}' | b']' => self.depth = self.depth.saturating_sub(1),
                    _ => {}
                }
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn count(counter: &mut Counter, value: &impl serde::Serialize) -> Result<(), ApiError> {
    serde_json::to_writer(counter, value)
        .map_err(|e| ApiError::InvalidCommand(format!("component replay preflight: {e}")))
}

pub(super) fn component_clone(
    def: &ComponentDef,
    settings: &DocumentSettings,
    imports: &ImportBlobs,
) -> Result<usize, ApiError> {
    let mut c = Counter::default();
    count(&mut c, &def.name)?;
    count(&mut c, settings)?;
    count(&mut c, &def.variables)?;
    for sketch in def.sketches.values() {
        // Sketch's serde conversion clones it; count the borrowed fields instead.
        count(&mut c, &sketch.name)?;
        count(&mut c, &sketch.plane)?;
        count(&mut c, &sketch.attachment)?;
        count(&mut c, &sketch.regions)?;
        count(&mut c, &sketch.projections)?;
        count(&mut c, &sketch.resolved_plane)?;
        count(&mut c, &sketch.extra)?;
        count(&mut c, &sketch.entities())?;
        count(&mut c, &sketch.constraints())?;
    }
    count(&mut c, &def.sketch_visibility)?;
    count(&mut c, &def.datum_planes)?;
    for record in def.timeline.records() {
        // OperationRecord's custom serializer builds a Value; count fields directly.
        count(&mut c, &record.name)?;
        count(&mut c, &record.op)?;
        count(&mut c, &record.inputs)?;
        count(&mut c, &record.outputs)?;
        count(&mut c, &record.determinism)?;
        count(&mut c, &record.extra)?;
    }
    count(&mut c, &def.bodies)?;
    count(&mut c, &def.elements)?;
    count(&mut c, &def.repair)?;
    count(&mut c, &def.named_selections)?;
    count(&mut c, &def.extra)?;
    for key in imports.keys() {
        count(&mut c, key)?;
    }
    Ok(c.bytes.saturating_mul(32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::{
        document::component::ComponentDefId,
        ids::SketchId,
        sketch::{Sketch, SketchAttachment, WorldPlane},
    };

    #[test]
    fn large_hidden_sketch_refuses_before_definition_clone() {
        let mut definition = ComponentDef::new(ComponentDefId(uuid::Uuid::new_v4()), "definition");
        let id = SketchId::new();
        let sketch = Sketch::new(
            id,
            "x".repeat(MAX_WIRE_BYTES),
            SketchAttachment::World {
                plane: WorldPlane::XY,
            },
        );
        definition.sketches.insert(id, sketch);
        assert!(component_clone(
            &definition,
            &DocumentSettings::default(),
            &ImportBlobs::new()
        )
        .is_err());
    }

    #[test]
    fn nested_unknown_metadata_refuses_without_value_copy() {
        let mut definition = ComponentDef::new(ComponentDefId(uuid::Uuid::new_v4()), "definition");
        let mut value = serde_json::Value::Null;
        for _ in 0..40 {
            value = serde_json::json!([value]);
        }
        definition.extra.insert("future".into(), value);
        assert!(component_clone(
            &definition,
            &DocumentSettings::default(),
            &ImportBlobs::new()
        )
        .is_err());
    }
}
