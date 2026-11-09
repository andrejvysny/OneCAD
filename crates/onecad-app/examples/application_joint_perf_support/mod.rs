use async_trait::async_trait;
use onecad_app::{
    api,
    dto::AssemblyProjectionDto,
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
    state::AppState,
};
use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointLimits, JointOrigin},
    math::{Frame, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, OccurrenceId, OccurrencePath},
};
use std::{collections::BTreeSet, path::Path, sync::Arc, time::Duration};
use uuid::Uuid;

struct Quiet;
impl EventSink for Quiet {
    fn emit(&self, _: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait]
impl DialogPort for Quiet {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}

pub async fn open(path: &Path, area: &Path) -> (AppState, HostContext, String) {
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(area.join("data")),
            app_config: Some(area.join("config")),
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    };
    let state = AppState::default();
    state.start(host.clone()).expect("start host");
    let opened = api::open_assembly_document(
        &state,
        host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .expect("open fixture");
    assert!(
        state.readiness().wait_ready(Duration::from_secs(30)).await,
        "worker unavailable"
    );
    (state, host, opened.runtime_session)
}

pub async fn replay_all(
    state: &AppState,
    session: &str,
    initial: &AssemblyProjectionDto,
) -> Vec<serde_json::Value> {
    let mut ids: Vec<_> = initial
        .definitions
        .iter()
        .filter(|d| d.id != initial.root_definition)
        .map(|d| d.id.clone())
        .collect();
    ids.sort();
    assert_eq!(ids.len(), 10);
    for id in &ids {
        let definition = ComponentDefId(Uuid::parse_str(id).expect("definition ID"));
        api::regenerate_component(state, session, definition)
            .await
            .expect("component replay");
    }
    let p = api::get_assembly_projection(state, session)
        .await
        .expect("replayed projection");
    let mut rows = Vec::new();
    for id in ids {
        let d = p
            .definitions
            .iter()
            .find(|d| d.id == id)
            .expect("definition");
        let o = p
            .occurrences
            .iter()
            .find(|o| o.definition == id)
            .expect("occurrence");
        let generation = d.mesh_generation.expect("component mesh generation");
        assert!(!d.bodies.is_empty(), "empty replayed component");
        let path = OccurrencePath(vec![OccurrenceId(Uuid::parse_str(&o.id).unwrap())]);
        let mut sizes = Vec::new();
        for body in &d.bodies {
            let mesh = api::component_mesh_for_path(
                state,
                session,
                &path,
                body.id.parse().expect("body ID"),
                generation,
            )
            .await
            .expect("component mesh");
            assert!(!mesh.is_empty(), "empty component mesh");
            sizes.push(mesh.len());
        }
        rows.push(
            serde_json::json!({"definition": id, "generation": generation, "bodyMeshBytes": sizes}),
        );
    }
    rows
}

fn local_frame(pose: RigidTransform) -> Frame {
    let inverse = pose.inverse();
    Frame {
        origin: inverse.apply_point(Vec3d::ZERO),
        z: inverse.apply_vector(Vec3d::X),
        x: inverse.apply_vector(Vec3d::Y),
    }
}

pub fn tree(p: &AssemblyProjectionDto) -> (Vec<Joint>, JointId) {
    assert_eq!(p.definitions.len(), 11);
    assert!(p.definitions.iter().any(|d| d.id == p.root_definition));
    assert_eq!(p.occurrences.len(), 20);
    let used: BTreeSet<_> = p
        .occurrences
        .iter()
        .map(|o| o.definition.as_str())
        .collect();
    let embedded: BTreeSet<_> = p
        .definitions
        .iter()
        .filter(|d| d.id != p.root_definition)
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(used.len(), 10);
    assert!(!used.contains(p.root_definition.as_str()));
    assert_eq!(used, embedded);
    assert_eq!(p.occurrences.iter().filter(|o| o.grounded).count(), 1);
    assert!(p.occurrences.iter().all(|o| o.path.len() == 1));
    assert!(p.joints.is_empty());
    let mut occurrences: Vec<_> = p.occurrences.iter().collect();
    occurrences.sort_by_key(|o| o.id.clone());
    assert!(occurrences[0].grounded && occurrences[1..].iter().all(|o| !o.grounded));
    let slider = JointId(Uuid::from_u128(0xbed000));
    let mut joints = Vec::new();
    for i in 1..20 {
        let parent = if i == 1 { 0 } else { 1 };
        let id = if i == 1 {
            slider
        } else {
            JointId(Uuid::from_u128(0xbed000 + i as u128))
        };
        let origin = |n: usize| JointOrigin {
            occurrence: OccurrenceId(Uuid::parse_str(&occurrences[n].id).unwrap()),
            frame: local_frame(occurrences[n].local),
        };
        joints.push(Joint {
            id,
            name: format!("Logical fixture joint {i}"),
            kind: if i == 1 {
                JointKind::Slider
            } else {
                JointKind::Rigid
            },
            a: origin(parent),
            b: origin(i),
            limits: (i == 1).then_some(JointLimits {
                min: -2.0,
                max: 2.0,
            }),
            value: 0.0,
            suppressed: false,
        });
    }
    (joints, slider)
}

pub fn pose_report(
    initial: &AssemblyProjectionDto,
    current: &AssemblyProjectionDto,
    drive: f64,
) -> Vec<serde_json::Value> {
    const LINEAR_TOLERANCE: f64 = 1e-6;
    const ANGULAR_TOLERANCE: f64 = 1e-9;
    let mut rows = Vec::new();
    if initial.occurrences.len() != current.occurrences.len() {
        rows.push(serde_json::json!({"status": "failed", "error": "occurrence count changed"}));
    }
    for original in &initial.occurrences {
        let Some(now) = current.occurrences.iter().find(|v| v.id == original.id) else {
            rows.push(serde_json::json!({"id": original.id, "status": "failed", "error": "occurrence missing"}));
            continue;
        };
        let mut expected = original.local;
        if !original.grounded {
            expected.translation = expected.translation + Vec3d::X * drive;
        }
        let linear = (now.local.translation - expected.translation).norm();
        let relative = expected
            .rotation
            .conjugate()
            .mul(now.local.rotation)
            .components();
        let angular = 2.0
            * (relative[1] * relative[1] + relative[2] * relative[2] + relative[3] * relative[3])
                .sqrt()
                .atan2(relative[0].abs());
        let passed = linear <= LINEAR_TOLERANCE && angular <= ANGULAR_TOLERANCE;
        rows.push(serde_json::json!({"id": original.id, "grounded": original.grounded, "linearErrorMm": linear, "angularErrorRad": angular, "status": if passed { "passed" } else { "failed" }}));
    }
    rows
}

pub fn stable(p: &AssemblyProjectionDto) -> serde_json::Value {
    serde_json::to_value(p).expect("serialize projection")
}
