use super::*;
use onecad_assembly::{
    math::Vec3d,
    occurrence::{ComponentDefId, OccurrenceId},
};

fn oracle() -> serde_json::Value {
    serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap()
}
fn vector(value: &serde_json::Value) -> Vec3d {
    Vec3d::new(
        value[0].as_f64().unwrap(),
        value[1].as_f64().unwrap(),
        value[2].as_f64().unwrap(),
    )
}
fn near(actual: f64, expected: f64, tolerance: f64, label: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: actual={actual}, expected={expected}, tolerance={tolerance}"
    );
}

pub(super) fn mesh_properties(mesh: &MeshView<'_>, expected: &serde_json::Value) {
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    let mut six_volume = 0.0;
    for index in 0..mesh.header().triangle_count as usize {
        let triangle = mesh.triangle(index).unwrap();
        let points = triangle.map(|vertex| {
            let value = mesh.position(vertex as usize).unwrap();
            [value[0] as f64, value[1] as f64, value[2] as f64]
        });
        for point in points {
            for axis in 0..3 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
        let [a, b, c] = points;
        six_volume += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    let volume = six_volume.abs() / 6.0;
    let target = expected["volumeMm3"].as_f64().unwrap();
    near(
        volume,
        target,
        target * 0.02,
        "tessellated volume vs source reader",
    );
    for axis in 0..3 {
        near(
            low[axis],
            expected["boundsMm"][0][axis].as_f64().unwrap(),
            0.15,
            "mesh minimum",
        );
        near(
            high[axis],
            expected["boundsMm"][1][axis].as_f64().unwrap(),
            0.15,
            "mesh maximum",
        );
    }
}

fn face_bounds(mesh: &MeshView<'_>, face: usize) -> ([f64; 3], [f64; 3]) {
    let span = mesh.face_range(face).unwrap();
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for triangle in span.first..span.first + span.count {
        for vertex in mesh.triangle(triangle as usize).unwrap() {
            let point = mesh.position(vertex as usize).unwrap();
            for axis in 0..3 {
                low[axis] = low[axis].min(point[axis] as f64);
                high[axis] = high[axis].max(point[axis] as f64);
            }
        }
    }
    (low, high)
}

fn candidate_id(mesh: &MeshView<'_>, expected: &serde_json::Value) -> String {
    let mut candidates = Vec::new();
    for face in 0..mesh.header().face_count as usize {
        let (low, high) = face_bounds(mesh, face);
        if (0..3).all(|axis| {
            (low[axis] - expected[0][axis].as_f64().unwrap()).abs() < 0.15
                && (high[axis] - expected[1][axis].as_f64().unwrap()).abs() < 0.15
        }) {
            candidates.push(mesh.face_id(face).unwrap().to_owned());
        }
    }
    assert_eq!(
        candidates.len(),
        1,
        "source face bounds must select exactly one mesh face"
    );
    assert!(candidates[0].starts_with("el_"));
    candidates.remove(0)
}

pub(super) async fn resolved_face(
    state: &AppState,
    session: &str,
    projection: &onecad_app::dto::AssemblyProjectionDto,
    part: &str,
    def: ComponentDefId,
    instance: OccurrenceId,
) -> (BodyId, onecad_app::api::JointOriginResolution) {
    let source = oracle();
    let expected = &source["parts"][part];
    let item = projection
        .definitions
        .iter()
        .find(|d| d.id == def.to_string())
        .unwrap();
    assert_eq!(item.bodies.len(), 1, "{part}: source-derived solid count");
    let body: BodyId = item.bodies[0].id.parse().unwrap();
    let generation = item.mesh_generation.unwrap();
    let path = OccurrencePath(vec![instance]);
    let bytes = api::component_mesh_for_path(state, session, &path, body, generation)
        .await
        .unwrap();
    let mesh = MeshView::decode(&bytes, MeshLimits::default()).unwrap();
    mesh_properties(&mesh, expected);
    let element = candidate_id(&mesh, &expected["candidate"]["faceBoundsMm"]);
    let datum = api::component_joint_origin(
        state,
        api::JointOriginRequest {
            session,
            generation: projection.persisted_generation,
            path: &path,
            body,
            mesh_generation: generation,
            element: &element,
            x: Vec3d::X,
        },
    )
    .await
    .unwrap();
    near(
        datum.radius.unwrap(),
        expected["candidate"]["radiusMm"].as_f64().unwrap(),
        1e-6,
        "source cylinder radius",
    );
    assert!((datum.frame.origin - vector(&expected["candidate"]["originMm"])).norm() < 1e-5);
    assert!((datum.frame.z - vector(&expected["candidate"]["axis"])).norm() < 1e-8);
    (body, datum)
}
