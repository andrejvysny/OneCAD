//! Unknown consumers cannot be proved inactive by searching for identity strings.
use super::*;
use crate::document::record::{KnownOperation, Operation, OperationRecord};
use crate::document::refs::{ElementRef, Extra};
use crate::sketch::{Sketch, SketchAttachment};

pub(super) fn validate(document: &Document) -> Result<(), String> {
    if !document.modules.is_empty()
        || !document.extra.is_empty()
        || !document.root_definition_extra.is_empty()
        || !document.assembly.extra.is_empty()
        || !document.datum_planes.is_empty()
        || !document.repair.is_empty()
    {
        return unsupported();
    }
    for definition in document.definitions.values() {
        if !definition.extra.is_empty()
            || !definition.datum_planes.is_empty()
            || !definition.repair.is_empty()
        {
            return unsupported();
        }
    }
    for record in document.timeline.records() {
        validate_record(record, true)?;
    }
    for definition in document.definitions.values() {
        for record in definition.timeline.records() {
            validate_record(record, !definition.elements.has_quarantined())?;
        }
    }
    for sketch in document.sketches.values().chain(
        document
            .definitions
            .values()
            .flat_map(|d| d.sketches.values()),
    ) {
        validate_sketch(sketch)?;
    }
    for selection in document.named_selections.iter().chain(
        document
            .definitions
            .values()
            .flat_map(|d| &d.named_selections),
    ) {
        empty(&selection.extra)?;
    }
    for binding in &document.assembly.joint_bindings {
        validate_reference(&binding.element)?;
    }
    for (_, entry) in document.elements.iter().chain(
        document
            .definitions
            .values()
            .flat_map(|definition| definition.elements.iter()),
    ) {
        if let Some(anchor) = &entry.anchor {
            validate_anchor(anchor)?;
        }
    }
    Ok(())
}

fn unsupported<T>() -> Result<T, String> {
    Err("quarantined topology requires a supported closed native history".into())
}

fn empty(extra: &Extra) -> Result<(), String> {
    if extra.is_empty() {
        Ok(())
    } else {
        unsupported()
    }
}

fn validate_record(record: &OperationRecord, allow_import: bool) -> Result<(), String> {
    if record.record_schema_version != crate::document::record::RECORD_SCHEMA_VERSION {
        return unsupported();
    }
    empty(&record.extra)?;
    empty(&record.determinism.extra)?;
    empty(&record.determinism.occt_options)?;
    match &record.op {
        Operation::Known(KnownOperation::Sketch(params)) => {
            empty(&params.extra)?;
            empty(&params.plane.extra)?;
            if let Some(face) = &params.host_face {
                validate_reference(face)?;
            }
            for entity in &params.entities {
                validate_wire_item(entity, false)?;
            }
            for constraint in &params.constraints {
                validate_wire_item(constraint, true)?;
            }
        }
        Operation::Known(KnownOperation::Extrude(params)) => {
            empty(&params.extra)?;
            if let Some(profile) = &params.profile {
                empty(&profile.extra)?;
            }
            for reference in [&params.target_face, &params.target_face2]
                .into_iter()
                .flatten()
            {
                validate_reference(reference)?;
            }
        }
        Operation::Known(KnownOperation::Revolve(params)) => {
            empty(&params.extra)?;
            if let Some(profile) = &params.profile {
                empty(&profile.extra)?;
            }
            match &params.axis {
                Some(crate::document::refs::AxisRef::SketchLine { extra, .. }) => empty(extra)?,
                _ => return unsupported(),
            }
        }
        Operation::Known(KnownOperation::ImportStep(params)) if allow_import => {
            // Imported source bytes produce new bodies; they cannot consume another
            // definition's topology. The reservation owner must stay native.
            empty(&params.extra)?;
            params.validate()?;
        }
        Operation::Known(KnownOperation::Hole(params)) => {
            empty(&params.extra)?;
            validate_reference(&params.face)?;
            if params.thread.is_some() {
                return unsupported();
            }
            for point in params.points.iter().flatten() {
                empty(&point.extra)?;
            }
        }
        _ => return unsupported(),
    }
    Ok(())
}

fn validate_reference(reference: &ElementRef) -> Result<(), String> {
    empty(&reference.extra)?;
    if let Some(primary) = &reference.primary {
        empty(&primary.extra)?;
    }
    if let Some(intent) = &reference.intent {
        empty(&intent.extra)?;
    }
    if let Some(anchor) = &reference.anchor {
        validate_anchor(anchor)?;
    }
    Ok(())
}

fn validate_anchor(anchor: &crate::document::refs::AnchorIntent) -> Result<(), String> {
    empty(&anchor.extra)?;
    if let Some(frame) = &anchor.local_frame {
        empty(&frame.extra)?;
    }
    Ok(())
}

fn validate_sketch(sketch: &Sketch) -> Result<(), String> {
    empty(&sketch.extra)?;
    for region in &sketch.regions {
        empty(&region.extra)?;
    }
    match &sketch.attachment {
        SketchAttachment::World { .. } => {}
        SketchAttachment::HostFace { face, .. } => validate_reference(face)?,
        SketchAttachment::Datum { .. } => return unsupported(),
    }
    if !sketch.projections.is_empty() {
        return unsupported();
    }
    // Typed entities and constraints address sketch-local EntityIds only;
    // unlike a record's raw wire arrays, they cannot contain an opaque variant.
    Ok(())
}

fn validate_wire_item(value: &Value, constraint: bool) -> Result<(), String> {
    let fields = value.as_object().ok_or("unsupported sketch item")?;
    let kind = fields.get("type").and_then(Value::as_str).unwrap_or("");
    let allowed: &[&str] = match (constraint, kind) {
        (false, "Point") => &["type", "id", "at", "construction", "referenceLocked"],
        (false, "Line") => &[
            "type",
            "id",
            "p0Ref",
            "p1Ref",
            "construction",
            "referenceLocked",
        ],
        (false, "Circle") => &[
            "type",
            "id",
            "center",
            "centerRef",
            "radius",
            "construction",
            "referenceLocked",
        ],
        (false, "Arc") => &[
            "type",
            "id",
            "center",
            "centerRef",
            "radius",
            "startAngle",
            "endAngle",
            "construction",
            "referenceLocked",
        ],
        (true, "Horizontal" | "Vertical" | "Fixed") => &["type", "id", "entities"],
        (true, "Distance" | "Diameter") => &["type", "id", "entities", "value"],
        _ => return unsupported(),
    };
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return unsupported();
    }
    for (key, value) in fields {
        let valid = match key.as_str() {
            "type" => true,
            "id" | "p0Ref" | "p1Ref" | "centerRef" => uuid_value(value),
            "construction" | "referenceLocked" => value.is_boolean(),
            "at" | "center" => value
                .as_array()
                .is_some_and(|items| items.len() == 2 && items.iter().all(finite)),
            "radius" | "startAngle" | "endAngle" | "value" => finite(value),
            "entities" => value
                .as_array()
                .is_some_and(|items| items.iter().all(uuid_value)),
            _ => false,
        };
        if !valid {
            return unsupported();
        }
    }
    Ok(())
}

fn uuid_value(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|text| uuid::Uuid::parse_str(text).is_ok())
}

fn finite(value: &Value) -> bool {
    value.as_f64().is_some_and(f64::is_finite)
}
