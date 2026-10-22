//! Re-edits of saved features through `BeginParameterEdit`, the History path:
//! one typed value, one commit. Covers the upstream plate edit the anchored
//! edge-op refs must survive and the edits a reopened document replays.
use super::{drive, host::Harness, plate, DELTA};
use onecad_app::dto::DocumentProjection;
use onecad_core::ids::{BodyId, RecordId};
use onecad_interaction::*;
use std::f64::consts::PI;

/// The plate thickness after the upstream edit.
const THICK: f64 = 6.0;

/// Opens `parameter` of the stored record `id`, types `text` and commits.
pub async fn edit(
    harness: &Harness,
    state: &mut Interaction,
    (id, parameter): (RecordId, NumericParameter),
    text: &str,
    gesture: u64,
) -> DocumentProjection {
    let context = drive::adopt(harness, state).await;
    let record = onecad_app::api::get_operation_record(&harness.state, id)
        .await
        .unwrap();
    let effects = state.dispatch(Action::BeginParameterEdit {
        record: Box::new(record),
        parameter,
        gesture,
        context,
    });
    drive::drive(harness, state, effects).await;
    assert!(
        state.tool.as_ref().is_some_and(ModelSession::ready),
        "{parameter:?} edit is ready"
    );
    drive::typed(harness, state, parameter.into(), text).await;
    drive::confirm(harness, state).await
}

/// Thickens the plate 4 → 6 mm under the 0.5 mm fillet and the 0.5 mm chamfer.
/// Each replays from its anchored staged ref; only a rebind to a vertical corner
/// edge removes a cross-section times the new thickness, and only a rebind to
/// its own corner leaves the other two corners sharp.
pub async fn thicken_plate(harness: &Harness, state: &mut Interaction, body: BodyId) -> f64 {
    let thickness = (plate::plate_record(), NumericParameter::ExtrudeDistance);
    edit(harness, state, thickness, "6", 6).await;
    let after = drive::volume(harness, body).await;
    let section = (1.0 - PI / 4.0) * 0.25 + 0.5 * 0.5 * 0.5;
    let removed = section * THICK;
    drive::close(
        plate::volume_at(THICK) - after,
        removed,
        removed,
        DELTA,
        "6 mm plate: fillet and chamfer remove their sections times 6",
    );
    let (w, h) = (plate::WIDTH, plate::HEIGHT);
    for (u, v, sharp) in [
        (0.0, 0.0, false),
        (w, 0.0, false),
        (0.0, h, true),
        (w, h, true),
    ] {
        let found = sharp_corner(harness, body, u, v).await;
        assert_eq!(
            found, sharp,
            "sketch corner ({u}, {v}) sharp after the edit"
        );
    }
    after
}

/// A straight 6 mm vertical edge exactly at sketch corner (u, v).
async fn sharp_corner(harness: &Harness, body: BodyId, u: f64, v: f64) -> bool {
    let [x, y] = plate::world(u, v);
    drive::elements(harness, body, "e")
        .await
        .iter()
        .any(|edge| {
            edge.curve_type == 0
                && (edge.magnitude - THICK).abs() < 1e-6
                && (edge.center[0] - x).abs() < 1e-6
                && (edge.center[1] - y).abs() < 1e-6
        })
}

/// Edits on a reopened document: the offset under both TransformBody records,
/// then the last TransformBody itself.
pub async fn after_reopen(harness: &Harness, authored: &super::Authored, volume: f64) {
    let mut state = Interaction::default();
    let ids = (authored.body, authored.offset);
    let deeper = deepen_offset(harness, &mut state, ids, (volume, authored.top_area)).await;
    let rotated = (authored.body, authored.rotated);
    super::placement::turn_further(harness, &mut state, rotated, deeper).await;
}

/// After reopen, a deeper offset under the two TransformBody records adds the
/// top area once more; volume is invariant under the rigid motions after it.
async fn deepen_offset(
    harness: &Harness,
    state: &mut Interaction,
    (body, offset): (BodyId, RecordId),
    (volume, area): (f64, f64),
) -> f64 {
    edit(
        harness,
        state,
        (offset, NumericParameter::OffsetFaceDistance),
        "2",
        1,
    )
    .await;
    let after = drive::volume(harness, body).await;
    drive::close(
        after - volume,
        area,
        area,
        DELTA,
        "a 1 → 2 mm offset edit adds the top area again",
    );
    after
}
