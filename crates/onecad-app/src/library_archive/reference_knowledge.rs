//! Pruning requires proof that every potential reference consumer is understood.
use super::*;
use onecad_core::document::record::{
    ComponentSourceRef, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::refs::{AnchorIntent, AxisRef, ElementRef};

pub(super) struct ReferenceKnowledge {
    pub complete: bool,
    pub used: BTreeSet<String>,
}

pub(super) fn assess(document: &Document) -> ReferenceKnowledge {
    let mut knowledge = ReferenceKnowledge {
        complete: metadata_known(document) && document_state_known(document),
        used: BTreeSet::new(),
    };
    for record in document.timeline.records().iter().chain(
        document
            .definitions
            .values()
            .flat_map(|definition| definition.timeline.records()),
    ) {
        knowledge.complete &= record_known(record);
        if let Operation::Known(KnownOperation::PlaceComponent(params)) = &record.op {
            // Suppressed records and rollback suffixes can become active again.
            knowledge.used.insert(params.component_revision.clone());
        }
    }
    knowledge
}

fn metadata_known(document: &Document) -> bool {
    document.modules.iter().all(|(id, state)| {
        id == &module_id()
            && state.schema_version == 1
            && state
                .payload
                .as_object()
                .is_some_and(|object| object.keys().all(|key| key == "snapshots"))
    }) && references(document).is_ok_and(|refs| refs.values().all(|entry| entry.extra.is_empty()))
}

fn document_state_known(document: &Document) -> bool {
    document.schema_version == onecad_core::document::DOCUMENT_SCHEMA_VERSION
        && document.extra.is_empty()
        && document.root_definition_extra.is_empty()
        && document.assembly.extra.is_empty()
        && document
            .definitions
            .values()
            .all(|definition| definition.extra.is_empty())
        && local_maps_known(document)
        && identity_evidence_known(document)
}

fn local_maps_known(document: &Document) -> bool {
    document
        .datum_planes
        .values()
        .chain(
            document
                .definitions
                .values()
                .flat_map(|d| d.datum_planes.values()),
        )
        .all(|datum| datum.extra.is_empty())
        && document
            .named_selections
            .iter()
            .chain(
                document
                    .definitions
                    .values()
                    .flat_map(|d| &d.named_selections),
            )
            .all(|selection| selection.extra.is_empty())
        && document
            .sketches
            .values()
            .chain(
                document
                    .definitions
                    .values()
                    .flat_map(|d| d.sketches.values()),
            )
            .all(sketch_known)
}

fn identity_evidence_known(document: &Document) -> bool {
    document
        .elements
        .iter()
        .chain(
            document
                .definitions
                .values()
                .flat_map(|d| d.elements.iter()),
        )
        .all(|(_, entry)| {
            entry.descriptor.is_none() && entry.anchor.as_ref().is_none_or(anchor_known)
        })
        && document
            .repair
            .items()
            .iter()
            .chain(document.definitions.values().flat_map(|d| d.repair.items()))
            .all(|item| {
                item.candidates
                    .iter()
                    .all(|candidate| candidate.extra.is_empty())
                    && item.anchor.as_ref().is_none_or(anchor_known)
            })
        && document
            .assembly
            .joint_bindings
            .iter()
            .all(|binding| reference_known(&binding.element))
}

fn sketch_known(sketch: &onecad_core::sketch::Sketch) -> bool {
    sketch.extra.is_empty()
        && sketch.regions.iter().all(|region| region.extra.is_empty())
        && match &sketch.attachment {
            onecad_core::sketch::SketchAttachment::HostFace { face, .. } => reference_known(face),
            _ => true,
        }
}

fn record_known(record: &OperationRecord) -> bool {
    if record.record_schema_version != onecad_core::document::record::RECORD_SCHEMA_VERSION
        || !record.extra.is_empty()
        || !record.determinism.extra.is_empty()
        || !record.determinism.occt_options.is_empty()
    {
        return false;
    }
    let Operation::Known(operation) = &record.op else {
        return false;
    };
    params_extra_empty(operation)
        && operation
            .stored_element_refs()
            .into_iter()
            .all(reference_known)
        && nested_params_known(operation)
}

fn params_extra_empty(operation: &KnownOperation) -> bool {
    match operation {
        KnownOperation::Sketch(p) => p.extra.is_empty(),
        KnownOperation::Extrude(p) => p.extra.is_empty(),
        KnownOperation::Revolve(p) => p.extra.is_empty(),
        KnownOperation::Fillet(p) => p.extra.is_empty(),
        KnownOperation::Chamfer(p) => p.extra.is_empty(),
        KnownOperation::Shell(p) => p.extra.is_empty(),
        KnownOperation::Boolean(p) => p.extra.is_empty(),
        KnownOperation::LinearPattern(p) => p.extra.is_empty(),
        KnownOperation::CircularPattern(p) => p.extra.is_empty(),
        KnownOperation::FeaturePattern(p) => p.extra.is_empty(),
        KnownOperation::Loft(p) => p.extra.is_empty(),
        KnownOperation::Sweep(p) => p.extra.is_empty(),
        KnownOperation::MirrorBody(p) => p.extra.is_empty(),
        KnownOperation::ImportStep(p) => p.extra.is_empty(),
        KnownOperation::TransformBody(p) => p.extra.is_empty(),
        KnownOperation::Hole(p) => p.extra.is_empty(),
        KnownOperation::OffsetFace(p) => p.extra.is_empty(),
        KnownOperation::Gear(p) => p.extra.is_empty(),
        KnownOperation::PlaceComponent(p) => p.extra.is_empty(),
        KnownOperation::DetachComponent(p) => p.extra.is_empty(),
    }
}

fn nested_params_known(operation: &KnownOperation) -> bool {
    match operation {
        // Raw wire arrays can contain future variants and arbitrary extensions.
        KnownOperation::Sketch(p) => {
            p.plane.extra.is_empty() && p.entities.is_empty() && p.constraints.is_empty()
        }
        KnownOperation::Extrude(p) => p
            .profile
            .as_ref()
            .is_none_or(|profile| profile.extra.is_empty()),
        KnownOperation::Revolve(p) => {
            p.profile
                .as_ref()
                .is_none_or(|profile| profile.extra.is_empty())
                && p.axis.as_ref().is_none_or(axis_known)
        }
        KnownOperation::Loft(p) => p.profiles.iter().all(|profile| profile.extra.is_empty()),
        KnownOperation::Sweep(p) => p
            .profile
            .as_ref()
            .is_none_or(|profile| profile.extra.is_empty()),
        KnownOperation::Hole(p) => p
            .points
            .iter()
            .flatten()
            .all(|point| point.extra.is_empty()),
        KnownOperation::PlaceComponent(p) => {
            source_known(&p.source) && p.mate.as_ref().is_none_or(|mate| mate.extra.is_empty())
        }
        KnownOperation::DetachComponent(p) => source_known(&p.source),
        _ => true,
    }
}

fn source_known(source: &ComponentSourceRef) -> bool {
    match source {
        ComponentSourceRef::Generator { extra, .. }
        | ComponentSourceRef::Embedded { extra, .. }
        | ComponentSourceRef::Document { extra, .. }
        | ComponentSourceRef::Profile { extra, .. } => extra.is_empty(),
    }
}

fn axis_known(axis: &AxisRef) -> bool {
    match axis {
        AxisRef::SketchLine { extra, .. } => extra.is_empty(),
        AxisRef::Element {
            edge_ref, extra, ..
        } => extra.is_empty() && edge_ref.as_ref().is_none_or(reference_known),
    }
}

fn reference_known(reference: &ElementRef) -> bool {
    reference.extra.is_empty()
        && reference.primary.as_ref().is_none_or(|primary| primary.extra.is_empty())
        // Descriptors are opaque worker-owned payloads, never proof of absence.
        && reference.intent.is_none()
        && reference.anchor.as_ref().is_none_or(anchor_known)
}

fn anchor_known(anchor: &AnchorIntent) -> bool {
    anchor.extra.is_empty()
        && anchor
            .local_frame
            .as_ref()
            .is_none_or(|frame| frame.extra.is_empty())
}
