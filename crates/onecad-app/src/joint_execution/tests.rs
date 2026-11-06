use super::*;
use onecad_assembly::{
    joint::{JointLimits, JointOrigin},
    math::{Frame, RigidTransform, Vec3d},
};
use uuid::Uuid;
fn body(n: u128, grounded: bool) -> SolverBody {
    SolverBody {
        id: OccurrenceId(Uuid::from_u128(n)),
        pose: RigidTransform::IDENTITY,
        grounded,
    }
}
fn joint(n: u128, a: u128, b: u128) -> Joint {
    let frame = Frame {
        origin: Vec3d::ZERO,
        z: Vec3d::Z,
        x: Vec3d::X,
    };
    Joint {
        id: JointId(Uuid::from_u128(n)),
        name: "slider".into(),
        kind: JointKind::Slider,
        a: JointOrigin {
            occurrence: OccurrenceId(Uuid::from_u128(a)),
            frame,
        },
        b: JointOrigin {
            occurrence: OccurrenceId(Uuid::from_u128(b)),
            frame,
        },
        limits: Some(JointLimits { min: 0.0, max: 1.0 }),
        value: 0.0,
        suppressed: false,
    }
}
#[test]
fn refuses_ungrounded_loops_duplicate_drives_and_invalid_limits() {
    let bodies = vec![body(1, true), body(2, false), body(3, false)];
    let j = joint(10, 1, 2);
    let k = joint(11, 2, 3);
    assert!(validate_tree(&bodies, &[j.clone(), k.clone()], &[]).is_ok());
    assert!(validate_tree(&bodies, std::slice::from_ref(&j), &[]).is_err());
    assert!(validate_tree(&bodies, &[j.clone(), k.clone(), joint(12, 3, 1)], &[]).is_err());
    assert!(validate_tree(
        &bodies,
        &[j.clone(), k.clone()],
        &[(j.id, 0.5), (j.id, 0.8)]
    )
    .is_err());
    assert!(validate_tree(&bodies, &[j.clone(), k], &[(j.id, 2.0)]).is_err());
}
#[test]
fn independent_limited_sliders_can_be_driven_together() {
    let bodies = vec![body(1, true), body(2, false), body(3, false)];
    let a = joint(10, 1, 2);
    let b = joint(11, 2, 3);
    let drives = vec![(a.id, 0.25), (b.id, 0.75)];
    validate_tree(&bodies, &[a.clone(), b.clone()], &drives).unwrap();
    let mut request = solver::SolveRequest::new(bodies, vec![a, b]);
    request.drive = drives;
    let SolveOutcome::Converged {
        poses,
        remaining_dof,
        ..
    } = solver::solve(&request)
    else {
        panic!("valid independent limits did not converge")
    };
    assert_eq!(remaining_dof, 0);
    let z = |id| {
        poses
            .iter()
            .find(|(occurrence, _)| occurrence.0 == Uuid::from_u128(id))
            .unwrap()
            .1
            .translation
            .z
    };
    assert!((z(2) - 0.25).abs() < 1e-6);
    assert!((z(3) - 1.0).abs() < 1e-6);
}

#[test]
fn twenty_occurrence_tree_solve_measures_elapsed_time() {
    let bodies: Vec<_> = (1..=20).map(|n| body(n, n == 1)).collect();
    let joints: Vec<_> = (1..20)
        .map(|n| {
            let mut j = joint(100 + n, n, n + 1);
            j.kind = JointKind::Rigid;
            j.limits = None;
            j
        })
        .collect();
    validate_tree(&bodies, &joints, &[]).unwrap();
    let start = std::time::Instant::now();
    let result = solver::solve(&solver::SolveRequest::new(bodies, joints));
    eprintln!("20 occurrence joint solve: {:?}", start.elapsed());
    assert!(matches!(result, SolveOutcome::Converged { .. }));
}

#[tokio::test]
#[ignore = "requires production worker"]
async fn cancelled_before_publication_cannot_commit() {
    use crate::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
    use onecad_assembly::occurrence::{ComponentDefId, Occurrence};
    use onecad_core::edit::AssemblyEdit;
    use std::sync::Arc;
    struct Quiet;
    impl EventSink for Quiet {
        fn emit(&self, _: AppEvent) -> Result<(), String> {
            Ok(())
        }
    }
    #[async_trait::async_trait]
    impl DialogPort for Quiet {
        async fn pick(&self, _: FileDialog) -> Vec<String> {
            Vec::new()
        }
    }
    let area = tempfile::tempdir().unwrap();
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(area.path().into()),
            app_config: Some(area.path().join("config")),
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    };
    let state = Arc::new(AppState::default());
    state.start(host.clone()).unwrap();
    let opened = crate::api::new_document(&state, host.clone())
        .await
        .unwrap();
    assert!(
        state
            .readiness()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let def = ComponentDefId(Uuid::from_u128(3));
    crate::api::apply_assembly_edit(
        &state,
        &opened.runtime_session,
        AssemblyEdit::CreateDefinition {
            id: def,
            name: "part".into(),
        },
    )
    .await
    .unwrap();
    for n in 1..=2 {
        crate::api::apply_assembly_edit(
            &state,
            &opened.runtime_session,
            AssemblyEdit::InsertOccurrence {
                occurrence: Occurrence {
                    id: OccurrenceId(Uuid::from_u128(n)),
                    def,
                    parent: None,
                    local: RigidTransform::IDENTITY,
                    grounded: n == 1,
                    name: "part".into(),
                },
            },
        )
        .await
        .unwrap();
    }
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    *state.joint_before_commit.lock().unwrap() = Some((started, resume));
    let worker = state.clone();
    let session = opened.runtime_session.clone();
    let task = tokio::spawn(async move {
        solve_joints(&worker, &session, vec![joint(10, 1, 2)], vec![]).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(15), ready)
        .await
        .unwrap()
        .unwrap();
    cancel_joint_solves(&state).unwrap();
    release.send(()).unwrap();
    assert!(task.await.unwrap().is_err());
    let guard = state.runtime.lock().await;
    assert!(
        guard
            .as_ref()
            .unwrap()
            .assembly_projection()
            .unwrap()
            .occurrences
            .len()
            == 2
    );
    assert_eq!(guard.as_ref().unwrap().undo_depth(), 3);
    drop(guard);
    let result = solve_joints(
        &state,
        &opened.runtime_session,
        vec![joint(10, 1, 2)],
        vec![(JointId(Uuid::from_u128(10)), 0.5)],
    )
    .await
    .unwrap();
    assert_eq!(result.remaining_dof, 0);
    assert_eq!(state.runtime.lock().await.as_ref().unwrap().undo_depth(), 4);
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    *state.joint_before_commit.lock().unwrap() = Some((started, resume));
    let older_state = state.clone();
    let older_session = opened.runtime_session.clone();
    let older = tokio::spawn(async move {
        solve_joints(
            &older_state,
            &older_session,
            vec![joint(10, 1, 2)],
            vec![(JointId(Uuid::from_u128(10)), 0.25)],
        )
        .await
    });
    ready.await.unwrap();
    solve_joints(
        &state,
        &opened.runtime_session,
        vec![joint(10, 1, 2)],
        vec![(JointId(Uuid::from_u128(10)), 0.75)],
    )
    .await
    .unwrap();
    release.send(()).unwrap();
    assert!(older.await.unwrap().is_err());
    assert_eq!(state.runtime.lock().await.as_ref().unwrap().undo_depth(), 5);
    crate::api::close_document(&state, host).await.unwrap();
    state.retire_all();
}

#[test]
fn cancellation_advances_latest_ticket() {
    let state = AppState::default();
    let first = next_ticket(&state).unwrap();
    cancel_joint_solves(&state).unwrap();
    assert_ne!(state.joint_ticket.load(Ordering::Acquire), first);
}
