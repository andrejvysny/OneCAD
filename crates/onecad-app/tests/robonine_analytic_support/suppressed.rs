use super::*;

async fn prepare(
    path: &Path,
    area: &Path,
) -> (AppState, HostContext, String, Vec<Joint>, Vec<JointBinding>) {
    write_fixture(path);
    let (state, host, session) = open(path, area).await;
    for i in 0..4 {
        api::regenerate_component(&state, &session, def(i))
            .await
            .unwrap();
    }
    api::save_document(&state, host.clone(), None, None)
        .await
        .unwrap();
    let (joints, bindings) = fixtures(&state, &session, path).await;
    solve(
        &state,
        &session,
        joints.clone(),
        bindings.clone(),
        vec![],
        false,
    )
    .await
    .unwrap();
    api::apply_assembly_edit(
        &state,
        &session,
        AssemblyEdit::SetGrounded {
            occurrence: occ(1),
            grounded: true,
        },
    )
    .await
    .unwrap();
    (state, host, session, joints, bindings)
}

async fn unchanged_refusals(
    state: &AppState,
    session: &str,
    joints: &[Joint],
    bindings: &[JointBinding],
) {
    let before = api::get_assembly_projection(state, session).await.unwrap();
    let mut active = joints.to_vec();
    active[0].suppressed = false;
    assert!(
        solve(state, session, active, bindings.to_vec(), vec![], false)
            .await
            .unwrap_err()
            .to_string()
            .contains("joint origin needs repair")
    );
    assert!(solve(
        state,
        session,
        joints.to_vec(),
        bindings.to_vec(),
        vec![],
        true
    )
    .await
    .is_err());
    assert!(
        joint_execution::solve_bound_joints_at_generation_cancellable(
            state,
            session,
            before.persisted_generation - 1,
            joints.to_vec(),
            vec![],
            bindings.to_vec(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .is_err()
    );
    let after = api::get_assembly_projection(state, session).await.unwrap();
    assert_eq!(stable(&before), stable(&after));
    assert_eq!(before.persisted_generation, after.persisted_generation);
}

#[tokio::test]
async fn suppressed_stale_origin_preserves_evidence_through_unrelated_solves_and_cold_reopen() {
    let _worker_lane = WORKER_LANE.lock().await;
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    let area = tempfile::tempdir().unwrap();
    let path = area.path().join("suppressed.onecad");
    let (state, host, session, mut joints, mut bindings) = prepare(&path, area.path()).await;
    joints[0].suppressed = true;
    bindings[0].needs_repair = true;
    bindings[0].element.primary.as_mut().unwrap().element =
        onecad_core::ids::ElementId::new("el_deleted_suppressed");
    let before = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    let active = joints[1].id;
    solve(
        &state,
        &session,
        joints.clone(),
        bindings.clone(),
        vec![(active, -1.0)],
        false,
    )
    .await
    .unwrap();
    let changed = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert!((pose(&changed, 4).translation.x - 1.0).abs() < 1e-6);
    assert_pose_near(pose(&before, 1), pose(&changed, 1));
    unchanged_refusals(&state, &session, &joints, &bindings).await;
    api::undo(&state, host.clone()).await.unwrap();
    assert_eq!(
        stable(
            &api::get_assembly_projection(&state, &session)
                .await
                .unwrap()
        ),
        stable(&before)
    );
    api::redo(&state, host.clone()).await.unwrap();
    assert_eq!(
        stable(
            &api::get_assembly_projection(&state, &session)
                .await
                .unwrap()
        ),
        stable(&changed)
    );
    api::save_document(&state, host.clone(), None, None)
        .await
        .unwrap();
    let stored = ContainerReader::open(&path).unwrap();
    assert_eq!(stored.document().assembly.joint_bindings, bindings);
    api::shutdown_host(&state, host, None).await.unwrap();
    let (state, host, session) = open(&path, area.path()).await;
    for i in 0..4 {
        api::regenerate_component(&state, &session, def(i))
            .await
            .unwrap();
    }
    drive(&state, &session, vec![(active, -2.0)], false)
        .await
        .unwrap();
    let reopened = api::get_assembly_projection(&state, &session)
        .await
        .unwrap();
    assert!((pose(&reopened, 4).translation.x - 2.0).abs() < 1e-6);
    let reopened_bindings =
        api::get_joint_bindings(&state, &session, reopened.persisted_generation)
            .await
            .unwrap();
    assert_eq!(reopened_bindings[0], bindings[0]);
    api::shutdown_host(&state, host, None).await.unwrap();
}
