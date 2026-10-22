//! Move / Rotate of the plate by exact typed values: volume is kept, the exact
//! centroid moves rigidly and the mesh bounds shift or swap their extents.
use super::{drive, host::Harness, SAME};
use onecad_core::ids::{BodyId, RecordId};
use onecad_interaction::{operand_binding, *};

/// Mesh bounds are f32 vertex positions.
const MESH: f64 = 1e-5;
/// The OCCT centroid of a rigidly moved solid, in mm.
const CENTROID: f64 = 1e-7;

type Bounds = ([f64; 3], [f64; 3]);

/// A new Move / Rotate about `center` with one typed value.
async fn transform(
    harness: &Harness,
    state: &mut Interaction,
    (body, center): (BodyId, [f64; 3]),
    (parameter, text): (NumericParameter, &str),
    gesture: u64,
) -> RecordId {
    let context = drive::adopt(harness, state).await;
    let step = harness.projection().await.applied_ops as u32;
    let record = operand_binding::transform_record(body, center, RecordId::new(), step);
    let id = record.record_id;
    let fields = operand_binding::transform_fields();
    let effects = state.dispatch(Action::BeginOperation {
        record: Box::new(record),
        focus: fields.first().copied(),
        fields,
        gesture,
        context,
    });
    drive::drive(harness, state, effects).await;
    drive::typed(harness, state, parameter.into(), text).await;
    assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Ready);
    drive::confirm(harness, state).await;
    id
}

fn centre(bounds: &Bounds) -> [f64; 3] {
    [0, 1, 2].map(|axis| (bounds.0[axis] + bounds.1[axis]) * 0.5)
}

fn extent(bounds: &Bounds) -> [f64; 3] {
    [0, 1, 2].map(|axis| bounds.1[axis] - bounds.0[axis])
}

fn near(actual: [f64; 3], expected: [f64; 3], tolerance: f64, what: &str) {
    for axis in 0..3 {
        assert!(
            (actual[axis] - expected[axis]).abs() < tolerance,
            "{what}: {actual:?} vs {expected:?}"
        );
    }
}

/// Moves by (10, 0, 0), then turns 90° about Z through the bounds centre.
pub async fn move_and_rotate(
    harness: &Harness,
    state: &mut Interaction,
    body: BodyId,
    volume: f64,
) -> (RecordId, RecordId) {
    let start = drive::bounds(harness, body).await;
    let centroid = drive::centroid(harness, body).await;
    let x = NumericParameter::TransformTranslation(ParameterAxis::X);
    let moved = transform(harness, state, (body, centre(&start)), (x, "10"), 6).await;
    drive::close(
        drive::volume(harness, body).await,
        volume,
        volume,
        SAME,
        "move keeps volume",
    );
    let shifted = drive::bounds(harness, body).await;
    near(
        shifted.0,
        [start.0[0] + 10.0, start.0[1], start.0[2]],
        MESH,
        "moved min",
    );
    near(
        shifted.1,
        [start.1[0] + 10.0, start.1[1], start.1[2]],
        MESH,
        "moved max",
    );
    let centroid = [centroid[0] + 10.0, centroid[1], centroid[2]];
    near(
        drive::centroid(harness, body).await,
        centroid,
        CENTROID,
        "moved centroid",
    );
    let pivot = centre(&shifted);
    let angle = NumericParameter::TransformRotationAngle;
    let rotated = transform(harness, state, (body, pivot), (angle, "90"), 7).await;
    drive::close(
        drive::volume(harness, body).await,
        volume,
        volume,
        SAME,
        "rotate keeps volume",
    );
    let turned = drive::bounds(harness, body).await;
    let [w, h, t] = extent(&shifted);
    near(
        extent(&turned),
        [h, w, t],
        MESH,
        "turned extents swap X and Y",
    );
    near(
        centre(&turned),
        pivot,
        MESH,
        "turned about the bounds centre",
    );
    let (dx, dy) = (centroid[0] - pivot[0], centroid[1] - pivot[1]);
    let expected = [pivot[0] - dy, pivot[1] + dx, centroid[2]];
    near(
        drive::centroid(harness, body).await,
        expected,
        CENTROID,
        "turned centroid",
    );
    (moved, rotated)
}
