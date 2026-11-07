#[path = "error_cleanup.rs"]
mod error_cleanup;
use super::*;
use onecad_core::{
    document::{
        component::{ComponentDef, Occurrence, OccurrenceId, RigidTransform},
        Document,
    },
    ids::DocumentId,
    io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta},
};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires production worker; run with ONECAD_REQUIRE_WORKER=1 and ONECAD_WORKER_PATH"]
async fn real_worker_replays_shared_definition_and_fresh_reopen() {
    let area = tempfile::tempdir().unwrap();
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/qa_plate_hole.onecad");
    let loaded = ContainerReader::open(&source).unwrap();
    let source = loaded.document();
    let id = ComponentDefId(Uuid::from_u128(0xD0));
    let mut def = ComponentDef::new(id, "Plate");
    def.variables = source.variables.clone();
    def.sketches = source.sketches.clone();
    def.sketch_visibility = source.sketch_visibility.clone();
    def.datum_planes = source.datum_planes.clone();
    def.timeline = source.timeline.clone();
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0xD1)));
    doc.definitions.insert(id, def);
    for n in 0..2 {
        doc.assembly
            .occurrences
            .insert(Occurrence {
                id: OccurrenceId(Uuid::from_u128(0xD2 + n)),
                def: id,
                parent: None,
                local: RigidTransform::IDENTITY,
                grounded: false,
                name: format!("Plate {n}"),
            })
            .unwrap();
    }
    let path = area.path().join("assembly.onecad");
    let meta = SaveMeta {
        app_version: "test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save(&path, &doc, &ContainerCaches::none(), &meta).unwrap();
    let (_, host, _, _) =
        super::super::recovery_identity_tests::pending_copy(area.path(), false).await;
    let state = Arc::new(AppState::default());
    state.start(host.clone()).unwrap();
    let opened = super::super::open_assembly_document(
        &state,
        host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(state.readiness().wait_ready(Duration::from_secs(15)).await);
    let session = opened.runtime_session;
    error_cleanup::unpromoted_read_only_replay(&state, &session, id).await;
    error_cleanup::failed_replay_releases_capacity(&state, &session, id, host.clone()).await;
    let (generation, projection) = regenerate_component(&state, &session, id).await.unwrap();
    assert!(state.runtime.lock().await.as_ref().unwrap().is_dirty());
    assert_eq!(projection.occurrences.len(), 2);
    let part = projection
        .definitions
        .iter()
        .find(|part| part.id == id.to_string())
        .unwrap();
    assert_eq!(part.mesh_generation, Some(generation));
    assert!(!part.bodies.is_empty());
    let first_body = part.bodies[0].id.clone();
    let body = first_body.parse::<onecad_core::ids::BodyId>().unwrap();
    let first_path =
        onecad_core::document::component::OccurrencePath(vec![OccurrenceId(Uuid::from_u128(0xD2))]);
    let second_path =
        onecad_core::document::component::OccurrencePath(vec![OccurrenceId(Uuid::from_u128(0xD3))]);
    let first_mesh = component_mesh_for_path(&state, &session, &first_path, body, generation)
        .await
        .unwrap();
    let second_mesh = component_mesh_for_path(&state, &session, &second_path, body, generation)
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&first_mesh, &second_mesh));
    let face_id = onecad_protocol::mesh::MeshView::decode(
        &first_mesh,
        onecad_protocol::mesh::MeshLimits::default(),
    )
    .unwrap()
    .face_id(0)
    .unwrap()
    .to_string();
    assert!(face_id.starts_with("el_"), "{face_id}");
    let info = component_element_info(&state, &session, &first_path, body, generation, &face_id)
        .await
        .unwrap()
        .expect("rehydrated face");
    assert_eq!(info.element_id, face_id);
    let datum_body = "88ba8696-36ea-4b00-b2de-d7eb24f39eb4".parse().unwrap();
    let datum_mesh = component_mesh_for_path(&state, &session, &first_path, datum_body, generation)
        .await
        .unwrap();
    let view = onecad_protocol::mesh::MeshView::decode(
        &datum_mesh,
        onecad_protocol::mesh::MeshLimits::default(),
    )
    .unwrap();
    let mut cylinder = None;
    for index in 0..view.header().face_count as usize {
        let candidate = view.face_id(index).unwrap();
        let info = component_element_info(
            &state,
            &session,
            &first_path,
            datum_body,
            generation,
            candidate,
        )
        .await
        .unwrap()
        .unwrap();
        if info.surface_type == 1 {
            let resolved = component_joint_origin(
                &state,
                JointOriginRequest {
                    session: &session,
                    generation: projection.persisted_generation,
                    path: &first_path,
                    body: datum_body,
                    mesh_generation: generation,
                    element: candidate,
                    x: onecad_assembly::math::Vec3d::X,
                },
            )
            .await
            .unwrap();
            if (resolved.frame.origin.x + 20.0).abs() < 1e-8
                && (resolved.frame.origin.y - 30.0).abs() < 1e-8
            {
                cylinder = Some(candidate.to_owned());
                break;
            }
        }
    }
    let cylinder = cylinder.expect("fixture has one 6 mm cylindrical hole");
    let request = |path| JointOriginRequest {
        session: &session,
        generation: projection.persisted_generation,
        path,
        body: datum_body,
        mesh_generation: generation,
        element: &cylinder,
        x: onecad_assembly::math::Vec3d::X,
    };
    let resolved = component_joint_origin(&state, request(&first_path))
        .await
        .unwrap();
    assert_eq!(
        resolved.surface,
        onecad_core::document::component::JointSurface::Cylinder
    );
    assert!((resolved.radius.unwrap() - 3.0).abs() < 1e-9);
    assert!(
        (resolved.frame.origin.x + 20.0).abs() < 1e-8,
        "actual datum: {:?}",
        resolved.frame
    );
    assert!((resolved.frame.origin.y - 30.0).abs() < 1e-8);
    assert!((resolved.frame.z.z.abs() - 1.0).abs() < 1e-8);
    let sibling = component_joint_origin(&state, request(&second_path))
        .await
        .unwrap();
    assert_eq!(resolved.reference, sibling.reference);
    let grounded = apply_assembly_edit(
        &state,
        &session,
        onecad_core::edit::AssemblyEdit::SetGrounded {
            occurrence: first_path.0[0],
            grounded: true,
        },
    )
    .await
    .unwrap();
    let joint_id = onecad_assembly::joint::JointId(Uuid::from_u128(0xD4));
    let joint = onecad_assembly::joint::Joint {
        id: joint_id,
        name: "Bound".into(),
        kind: onecad_assembly::joint::JointKind::Rigid,
        a: onecad_assembly::joint::JointOrigin {
            occurrence: first_path.0[0],
            frame: resolved.frame,
        },
        b: onecad_assembly::joint::JointOrigin {
            occurrence: second_path.0[0],
            frame: resolved.frame,
        },
        limits: None,
        value: 0.0,
        suppressed: false,
    };
    let binding = onecad_core::document::component::JointBinding {
        joint: joint_id,
        side: onecad_core::document::component::JointSide::A,
        path: first_path.clone(),
        definition: resolved.definition,
        body: datum_body,
        element: resolved.reference,
        surface: resolved.surface,
        radius: resolved.radius,
        orientation_x: resolved.frame.x,
        needs_repair: false,
    };
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(true));
    assert!(
        crate::joint_execution::solve_bound_joints_at_generation_cancellable(
            &state,
            &session,
            grounded.persisted_generation,
            vec![joint.clone()],
            vec![],
            vec![binding.clone()],
            cancelled,
        )
        .await
        .is_err()
    );
    assert!(
        get_joint_bindings(&state, &session, grounded.persisted_generation)
            .await
            .unwrap()
            .is_empty()
    );
    crate::joint_execution::solve_bound_joints_at_generation_cancellable(
        &state,
        &session,
        grounded.persisted_generation,
        vec![joint],
        vec![],
        vec![binding],
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await
    .unwrap();
    assert_eq!(
        get_assembly_projection(&state, &session)
            .await
            .unwrap()
            .joint_bindings
            .len(),
        1
    );
    assert!(
        super::super::save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    let bound_copy = area.path().join("bound-reopen.onecad");
    std::fs::copy(&path, &bound_copy).unwrap();
    super::super::undo(&state, host.clone()).await.unwrap();
    assert!(get_assembly_projection(&state, &session)
        .await
        .unwrap()
        .joint_bindings
        .is_empty());
    super::super::redo(&state, host.clone()).await.unwrap();
    assert_eq!(
        get_assembly_projection(&state, &session)
            .await
            .unwrap()
            .joint_bindings
            .len(),
        1
    );
    super::super::undo(&state, host.clone()).await.unwrap();
    super::super::undo(&state, host.clone()).await.unwrap();
    assert!(component_element_info(
        &state,
        &session,
        &first_path,
        datum_body,
        generation,
        "el_missing"
    )
    .await
    .unwrap()
    .is_none());
    let invalid_path = onecad_core::document::component::OccurrencePath(vec![
        OccurrenceId(Uuid::from_u128(0xD2)),
        OccurrenceId(Uuid::from_u128(0xD3)),
    ]);
    assert!(
        component_mesh_for_path(&state, &session, &invalid_path, body, generation)
            .await
            .is_err()
    );
    let (started, ready) = tokio::sync::oneshot::channel();
    let (_release, resume) = tokio::sync::oneshot::channel();
    *state.component_before_commit.lock().unwrap() = Some((started, resume));
    let cancelled_state = state.clone();
    let cancelled_session = session.clone();
    let cancelled = tokio::spawn(async move {
        regenerate_component(&cancelled_state, &cancelled_session, id).await
    });
    ready.await.unwrap();
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(permit) = execution_slot().try_acquire_owned() {
                drop(permit);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled replay must retire its worker and release capacity");
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    *state.component_before_commit.lock().unwrap() = Some((started, resume));
    let replay_state = state.clone();
    let replay_session = session.clone();
    let replay =
        tokio::spawn(async move { regenerate_component(&replay_state, &replay_session, id).await });
    ready.await.unwrap();
    assert!(matches!(regenerate_component(&state, &session, id).await,
        Err(ApiError::InvalidCommand(message)) if message.contains("capacity")));
    apply_assembly_edit(
        &state,
        &session,
        onecad_core::edit::AssemblyEdit::SetVisibility {
            occurrence: OccurrenceId(Uuid::from_u128(0xD3)),
            visible: false,
        },
    )
    .await
    .unwrap();
    release.send(()).unwrap();
    assert!(replay.await.unwrap().is_err());
    let bytes = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let body = first_body.parse::<onecad_core::ids::BodyId>().unwrap();
        rt.component_mesh(id, body, generation).unwrap()
    };
    assert!(!bytes.is_empty());
    let record = source
        .timeline
        .records()
        .iter()
        .rev()
        .find(|record| {
            matches!(
                record.op,
                onecad_core::document::record::Operation::Known(
                    onecad_core::document::record::KnownOperation::Extrude(_)
                )
            )
        })
        .unwrap();
    let mut changed_op = record.op.clone();
    let onecad_core::document::record::Operation::Known(
        onecad_core::document::record::KnownOperation::Extrude(ref mut extrude),
    ) = changed_op
    else {
        unreachable!()
    };
    extrude.distance = onecad_core::document::variables::Scalar::new(9.0);
    let projection = apply_assembly_edit(
        &state,
        &session,
        onecad_core::edit::AssemblyEdit::EditDefinition {
            id,
            command: Box::new(onecad_core::edit::EditCommand::UpdateOperationParams {
                record: record.record_id,
                op: changed_op,
            }),
        },
    )
    .await
    .unwrap();
    assert!(projection
        .definitions
        .iter()
        .find(|def| def.id == id.to_string())
        .unwrap()
        .mesh_generation
        .is_none());
    assert!(
        component_mesh_for_path(&state, &session, &first_path, body, generation)
            .await
            .is_err()
    );
    let (edited_generation, _) = regenerate_component(&state, &session, id).await.unwrap();
    let edited_face = component_element_info(
        &state,
        &session,
        &first_path,
        body,
        edited_generation,
        &face_id,
    )
    .await
    .unwrap()
    .expect("edited face identity");
    assert_eq!(edited_face.element_id, face_id);
    let edited = component_mesh_for_path(&state, &session, &first_path, body, edited_generation)
        .await
        .unwrap();
    assert_ne!(edited.as_slice(), bytes.as_slice());
    let edited_mesh = onecad_protocol::mesh::MeshView::decode(
        &edited,
        onecad_protocol::mesh::MeshLimits::default(),
    )
    .unwrap();
    assert_eq!(edited_mesh.face_id(0), Some(face_id.as_str()));
    assert!(Arc::ptr_eq(
        &edited,
        &component_mesh_for_path(&state, &session, &second_path, body, edited_generation)
            .await
            .unwrap()
    ));
    super::super::undo(&state, host.clone()).await.unwrap();
    assert!(
        component_mesh_for_path(&state, &session, &first_path, body, edited_generation)
            .await
            .is_err()
    );
    super::super::redo(&state, host.clone()).await.unwrap();
    regenerate_component(&state, &session, id).await.unwrap();
    let independent = ComponentDefId(Uuid::from_u128(0xD4));
    apply_assembly_edit(
        &state,
        &session,
        onecad_core::edit::AssemblyEdit::MakeIndependent {
            occurrence: OccurrenceId(Uuid::from_u128(0xD3)),
            new_definition: independent,
            name: "Independent".into(),
        },
    )
    .await
    .unwrap();
    let (independent_generation, _) = regenerate_component(&state, &session, independent)
        .await
        .unwrap();
    let independent_mesh =
        component_mesh_for_path(&state, &session, &second_path, body, independent_generation)
            .await
            .unwrap();
    let mut changed_again = record.op.clone();
    let onecad_core::document::record::Operation::Known(
        onecad_core::document::record::KnownOperation::Extrude(ref mut extrude),
    ) = changed_again
    else {
        unreachable!()
    };
    extrude.distance = onecad_core::document::variables::Scalar::new(10.0);
    apply_assembly_edit(
        &state,
        &session,
        onecad_core::edit::AssemblyEdit::EditDefinition {
            id,
            command: Box::new(onecad_core::edit::EditCommand::UpdateOperationParams {
                record: record.record_id,
                op: changed_again,
            }),
        },
    )
    .await
    .unwrap();
    let (new_generation, _) = regenerate_component(&state, &session, id).await.unwrap();
    let changed_mesh = component_mesh_for_path(&state, &session, &first_path, body, new_generation)
        .await
        .unwrap();
    let preserved_mesh =
        component_mesh_for_path(&state, &session, &second_path, body, independent_generation)
            .await
            .unwrap();
    assert!(Arc::ptr_eq(&independent_mesh, &preserved_mesh));
    assert_ne!(changed_mesh.as_slice(), independent_mesh.as_slice());
    assert!(
        super::super::save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    super::super::close_document(&state, host.clone())
        .await
        .unwrap();
    let reopened = super::super::open_assembly_document(
        &state,
        host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let next_session = reopened.runtime_session;
    let (reopened_generation, projection) = regenerate_component(&state, &next_session, id)
        .await
        .unwrap();
    let reopened_face = component_element_info(
        &state,
        &next_session,
        &first_path,
        body,
        reopened_generation,
        &face_id,
    )
    .await
    .unwrap()
    .expect("reopened face identity");
    assert_eq!(reopened_face.element_id, face_id);
    let part = projection
        .definitions
        .iter()
        .find(|part| part.id == id.to_string())
        .unwrap();
    assert!(part.bodies.iter().any(|body| body.id == first_body));
    let (_, projection) = regenerate_component(&state, &next_session, independent)
        .await
        .unwrap();
    assert!(projection
        .definitions
        .iter()
        .any(|part| part.id == independent.to_string()
            && part.bodies.iter().any(|body| body.id == first_body)));
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    *state.component_before_commit.lock().unwrap() = Some((started, resume));
    let replay_state = state.clone();
    let replay_session = next_session.clone();
    let replay =
        tokio::spawn(async move { regenerate_component(&replay_state, &replay_session, id).await });
    ready.await.unwrap();
    super::super::close_document(&state, host.clone())
        .await
        .unwrap();
    assert!(state.runtime.lock().await.is_none());
    assert!(state.component_worker.lock().unwrap().is_none());
    release.send(()).unwrap();
    assert!(replay.await.unwrap().is_err());
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(permit) = execution_slot().try_acquire_owned() {
                drop(permit);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("closed component worker must release capacity");
    let reopened = super::super::open_assembly_document(
        &state,
        host.clone(),
        bound_copy.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let reopened_session = reopened.runtime_session;
    let (started, ready) = tokio::sync::oneshot::channel();
    let (_release, blocked) = tokio::sync::oneshot::channel();
    *state.component_after_commit.lock().unwrap() = Some((started, blocked));
    let pending_state = state.clone();
    let pending_session = reopened_session.clone();
    let pending =
        tokio::spawn(
            async move { regenerate_component(&pending_state, &pending_session, id).await },
        );
    ready.await.unwrap();
    let during = get_assembly_projection(&state, &reopened_session)
        .await
        .unwrap();
    assert!(!during.joint_bindings[0].verified);
    assert!(during
        .definitions
        .iter()
        .find(|d| d.id == id.to_string())
        .unwrap()
        .mesh_generation
        .is_none());
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    let after = get_assembly_projection(&state, &reopened_session)
        .await
        .unwrap();
    assert!(!after.joint_bindings[0].verified);
    assert!(after
        .definitions
        .iter()
        .find(|d| d.id == id.to_string())
        .unwrap()
        .mesh_generation
        .is_none());
    let (old_generation, _) = regenerate_component(&state, &reopened_session, id)
        .await
        .unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = tokio::sync::oneshot::channel();
    *state.component_after_commit.lock().unwrap() = Some((started, blocked));
    let pending_state = state.clone();
    let pending_session = reopened_session.clone();
    let pending =
        tokio::spawn(
            async move { regenerate_component(&pending_state, &pending_session, id).await },
        );
    ready.await.unwrap();
    let permit = execution_slot().try_acquire_owned().unwrap();
    release.send(()).unwrap();
    assert!(pending.await.unwrap().is_err());
    let after = get_assembly_projection(&state, &reopened_session)
        .await
        .unwrap();
    assert!(!after.joint_bindings[0].verified);
    assert!(after
        .definitions
        .iter()
        .find(|d| d.id == id.to_string())
        .unwrap()
        .mesh_generation
        .is_none());
    assert!(component_mesh_for_path(
        &state,
        &reopened_session,
        &first_path,
        datum_body,
        old_generation
    )
    .await
    .is_err());
    drop(permit);
    let (_, projection) = regenerate_component(&state, &reopened_session, id)
        .await
        .unwrap();
    assert!(projection.joint_bindings[0].verified);
    let bindings = get_joint_bindings(&state, &reopened_session, projection.persisted_generation)
        .await
        .unwrap();
    assert_eq!(bindings.len(), 1);
    assert!(!bindings[0].needs_repair);
    assert_eq!(bindings[0].path, first_path);
    crate::joint_execution::solve_bound_joints_at_generation_cancellable(
        &state,
        &reopened_session,
        projection.persisted_generation,
        projection.joints,
        vec![],
        bindings,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await
    .unwrap();
    let mut missing = ContainerReader::open(&bound_copy)
        .unwrap()
        .document()
        .clone();
    let before = get_assembly_projection(&state, &reopened_session)
        .await
        .unwrap();
    let before_poses: Vec<_> = before
        .occurrences
        .iter()
        .map(|occurrence| occurrence.local)
        .collect();
    let removed = apply_assembly_edit(
        &state,
        &reopened_session,
        onecad_core::edit::AssemblyEdit::EditDefinition {
            id,
            command: Box::new(onecad_core::edit::EditCommand::SetOperationSuppression {
                record: "fea7e23a-e2c3-4aae-b3f4-8e27d9de676c".parse().unwrap(),
                suppressed: true,
                cascade: false,
            }),
        },
    )
    .await
    .unwrap();
    assert!(removed.joint_bindings[0].needs_repair);
    assert_eq!(
        before_poses,
        removed
            .occurrences
            .iter()
            .map(|occurrence| occurrence.local)
            .collect::<Vec<_>>()
    );
    let bindings = get_joint_bindings(&state, &reopened_session, removed.persisted_generation)
        .await
        .unwrap();
    assert!(
        crate::joint_execution::solve_bound_joints_at_generation_cancellable(
            &state,
            &reopened_session,
            removed.persisted_generation,
            removed.joints,
            vec![],
            bindings,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .await
        .is_err()
    );
    assert!(
        super::super::save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    super::super::close_document(&state, host.clone())
        .await
        .unwrap();
    let missing_id = missing.assembly.joint_bindings[0]
        .element
        .primary
        .as_ref()
        .unwrap()
        .element
        .clone();
    assert!(missing
        .definitions
        .get_mut(&id)
        .unwrap()
        .elements
        .remove(&missing_id)
        .is_some());
    let missing_path = area.path().join("missing-joint-face.onecad");
    ContainerWriter::save(&missing_path, &missing, &ContainerCaches::none(), &meta).unwrap();
    let reopened = super::super::open_assembly_document(
        &state,
        host.clone(),
        missing_path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let missing_session = reopened.runtime_session;
    let (missing_generation, projection) = regenerate_component(&state, &missing_session, id)
        .await
        .unwrap();
    assert!(projection.joint_bindings[0].needs_repair);
    let before_poses: Vec<_> = projection
        .occurrences
        .iter()
        .map(|occurrence| occurrence.local)
        .collect();
    let bindings = get_joint_bindings(&state, &missing_session, projection.persisted_generation)
        .await
        .unwrap();
    assert!(matches!(
        component_joint_origin(
            &state,
            JointOriginRequest {
                session: &missing_session,
                generation: projection.persisted_generation,
                path: &first_path,
                body: datum_body,
                mesh_generation: missing_generation,
                element: missing_id.as_str(),
                x: onecad_assembly::math::Vec3d::X,
            }
        )
        .await,
        Err(ApiError::UnresolvedTopology(_))
    ));
    let failure = crate::joint_execution::solve_bound_joints_at_generation_cancellable(
        &state,
        &missing_session,
        projection.persisted_generation,
        projection.joints,
        vec![],
        bindings,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await
    .unwrap_err();
    assert!(matches!(failure, ApiError::InvalidCommand(_)));
    let repaired = get_assembly_projection(&state, &missing_session)
        .await
        .unwrap();
    assert!(repaired.joint_bindings[0].needs_repair);
    assert_eq!(
        before_poses,
        repaired
            .occurrences
            .iter()
            .map(|occurrence| occurrence.local)
            .collect::<Vec<_>>()
    );
    assert!(
        super::super::save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    super::super::close_document(&state, host.clone())
        .await
        .unwrap();
    let saved = ContainerReader::open(&missing_path).unwrap();
    assert!(saved.document().assembly.joint_bindings[0].needs_repair);
}

#[tokio::test]
#[ignore = "requires production worker; run with ONECAD_REQUIRE_WORKER=1 and ONECAD_WORKER_PATH"]
async fn real_worker_ten_definitions_twenty_occurrences_reopen_and_replay() {
    let area = tempfile::tempdir().unwrap();
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/qa_plate_hole.onecad");
    let source = ContainerReader::open(&fixture).unwrap();
    let source = source.document();
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0xE00)));
    for index in 0..10u128 {
        let id = ComponentDefId(Uuid::from_u128(0xE10 + index));
        let mut def = ComponentDef::new(id, format!("Plate {index}"));
        def.variables = source.variables.clone();
        def.sketches = source.sketches.clone();
        def.sketch_visibility = source.sketch_visibility.clone();
        def.datum_planes = source.datum_planes.clone();
        def.timeline = source.timeline.clone();
        doc.definitions.insert(id, def);
        for copy in 0..2u128 {
            doc.assembly
                .occurrences
                .insert(Occurrence {
                    id: OccurrenceId(Uuid::from_u128(0xE80 + index * 2 + copy)),
                    def: id,
                    parent: None,
                    local: RigidTransform::IDENTITY,
                    grounded: index == 0 && copy == 0,
                    name: format!("Plate {index}.{copy}"),
                })
                .unwrap();
        }
    }
    let path = area.path().join("ten-definitions.onecad");
    let meta = SaveMeta {
        app_version: "test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save(&path, &doc, &ContainerCaches::none(), &meta).unwrap();
    let (_, host, _, _) =
        super::super::recovery_identity_tests::pending_copy(area.path(), false).await;
    let state = AppState::default();
    state.start(host.clone()).unwrap();
    let opened = super::super::open_assembly_document(
        &state,
        host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(state.readiness().wait_ready(Duration::from_secs(15)).await);
    let session = opened.runtime_session;
    for index in 0..10u128 {
        let id = ComponentDefId(Uuid::from_u128(0xE10 + index));
        let (_, projection) = regenerate_component(&state, &session, id).await.unwrap();
        assert!(projection
            .definitions
            .iter()
            .any(|def| def.id == id.to_string()
                && def.mesh_generation.is_some()
                && !def.bodies.is_empty()));
    }
    let projection = get_assembly_projection(&state, &session).await.unwrap();
    assert_eq!(projection.definitions.len(), 11);
    assert_eq!(projection.occurrences.len(), 20);
    let definition = ComponentDefId(Uuid::from_u128(0xE10));
    let occurrence = OccurrenceId(Uuid::from_u128(0xE80));
    let first_path = onecad_core::document::component::OccurrencePath(vec![occurrence]);
    let first_generation = projection
        .definitions
        .iter()
        .find(|d| d.id == definition.to_string())
        .unwrap()
        .mesh_generation
        .unwrap();
    let body = "88ba8696-36ea-4b00-b2de-d7eb24f39eb4".parse().unwrap();
    let mesh = component_mesh_for_path(&state, &session, &first_path, body, first_generation)
        .await
        .unwrap();
    let view = onecad_protocol::mesh::MeshView::decode(
        &mesh,
        onecad_protocol::mesh::MeshLimits::default(),
    )
    .unwrap();
    let mut selected = None;
    for index in 0..view.header().face_count as usize {
        let candidate = view.face_id(index).unwrap();
        let info = component_element_info(
            &state,
            &session,
            &first_path,
            body,
            first_generation,
            candidate,
        )
        .await
        .unwrap()
        .unwrap();
        if info.surface_type == 0 && info.has_normal && info.normal[2].abs() > 0.99 {
            match component_joint_origin(
                &state,
                JointOriginRequest {
                    session: &session,
                    generation: projection.persisted_generation,
                    path: &first_path,
                    body,
                    mesh_generation: first_generation,
                    element: candidate,
                    x: onecad_assembly::math::Vec3d::X,
                },
            )
            .await
            {
                Ok(datum) => {
                    selected = Some(datum);
                    break;
                }
                Err(ApiError::UnresolvedTopology(_)) => continue,
                Err(error) => panic!("planar datum query failed: {error}"),
            }
        }
    }
    let datum = selected.expect("fixture has an anchored horizontal planar face");
    let joints: Vec<_> = (0..19u128)
        .map(|index| onecad_assembly::joint::Joint {
            id: onecad_assembly::joint::JointId(Uuid::from_u128(0xF00 + index)),
            name: format!("Joint {index}"),
            kind: onecad_assembly::joint::JointKind::Rigid,
            a: onecad_assembly::joint::JointOrigin {
                occurrence: OccurrenceId(Uuid::from_u128(0xE80 + index)),
                frame: datum.frame,
            },
            b: onecad_assembly::joint::JointOrigin {
                occurrence: OccurrenceId(Uuid::from_u128(0xE81 + index)),
                frame: datum.frame,
            },
            limits: None,
            value: 0.0,
            suppressed: false,
        })
        .collect();
    let binding = onecad_core::document::component::JointBinding {
        joint: joints[0].id,
        side: onecad_core::document::component::JointSide::A,
        path: first_path,
        definition,
        body,
        element: datum.reference,
        surface: datum.surface,
        radius: datum.radius,
        orientation_x: datum.frame.x,
        needs_repair: false,
    };
    let started = std::time::Instant::now();
    crate::joint_execution::solve_bound_joints_at_generation_cancellable(
        &state,
        &session,
        projection.persisted_generation,
        joints,
        vec![],
        vec![binding],
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await
    .unwrap();
    eprintln!(
        "20-occurrence grounded-tree bound solve: {:?}",
        started.elapsed()
    );
    assert!(
        super::super::save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    super::super::close_document(&state, host.clone())
        .await
        .unwrap();
    let reopened = super::super::open_assembly_document(
        &state,
        host,
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    for index in 0..10u128 {
        let id = ComponentDefId(Uuid::from_u128(0xE10 + index));
        let (_, projection) = regenerate_component(&state, &reopened.runtime_session, id)
            .await
            .unwrap();
        assert!(projection
            .definitions
            .iter()
            .any(|def| def.id == id.to_string() && def.mesh_generation.is_some()));
    }
    let projection = get_assembly_projection(&state, &reopened.runtime_session)
        .await
        .unwrap();
    assert_eq!(projection.joints.len(), 19);
    assert_eq!(projection.joint_bindings.len(), 1);
    assert!(!projection.joint_bindings[0].needs_repair);
}
