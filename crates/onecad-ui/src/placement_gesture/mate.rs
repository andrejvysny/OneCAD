//! The mate the gesture previews, built exactly as the application builds the recorded one.
use super::snap::Candidate;
use onecad_app::{dto::PlaceComponentMateInput, library::component_mate_from_input};
use onecad_core::{
    document::record::{ComponentMate, MateFrame},
    ids::{BodyId, ElementId},
    math::Vec3,
};
use onecad_library::package::AttachmentSpec;
use std::collections::BTreeMap;

/// The package attachment's frame in the record's own type, frozen with the mate
/// (`attachment_frame` in the application service).
pub(crate) fn self_frame(
    attachments: &BTreeMap<String, AttachmentSpec>,
    key: &str,
) -> Option<MateFrame> {
    let frame = attachments.get(key)?.frame?;
    let vec = |v: [f64; 3]| Vec3::new_unchecked(v[0], v[1], v[2]);
    Some(MateFrame {
        origin: vec(frame.origin),
        z: vec(frame.z),
        x: vec(frame.x),
    })
}

/// The mate for `candidate` on `body`'s element `element`; the topo key is
/// pick evidence the application's builder does not record.
pub(crate) fn build(
    attachments: &BTreeMap<String, AttachmentSpec>,
    candidate: &Candidate,
    (body, element): (BodyId, &str),
    flipped: bool,
) -> ComponentMate {
    let input = PlaceComponentMateInput {
        self_attachment: candidate.attachment.clone(),
        target_body_id: body.to_string(),
        target_topo_key: String::new(),
        target_element_id: Some(element.to_owned()),
        target_kind: if candidate.target_is_edge() {
            "edge"
        } else {
            "face"
        }
        .into(),
        kind: candidate.kind,
        flipped,
        anchor_world_point: candidate.anchor,
    };
    component_mate_from_input(
        &input,
        body,
        ElementId::new(element),
        self_frame(attachments, &candidate.attachment),
    )
}
