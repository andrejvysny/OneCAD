//! Reopened repaired joints retain authority while a new free instance remains editable.
use super::*;
use crate::assembly_controls::{Action, Reply, Request as AssemblyRequest};
use onecad_app::dto::AssemblyProjectionDto;
use onecad_assembly::{
    math::RigidTransform,
    occurrence::{Occurrence, OccurrenceId},
};
use onecad_core::edit::AssemblyEdit;

pub(super) async fn move_free_instance(
    backend: &mut Backend,
    session: &str,
    constrained: OccurrenceId,
    free: Occurrence,
    target: RigidTransform,
) -> AssemblyProjectionDto {
    let before = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap();
    refuse_constrained_move(backend, session, constrained, target, &before).await;
    let id = free.id;
    accept(
        backend,
        session,
        AssemblyEdit::InsertOccurrence { occurrence: free },
    )
    .await;
    accept(
        backend,
        session,
        AssemblyEdit::SetPose {
            occurrence: id,
            pose: target,
        },
    )
    .await;
    let after = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap();
    assert_free_move(&before, &after, id, target);
    after
}

fn assert_free_move(
    before: &AssemblyProjectionDto,
    after: &AssemblyProjectionDto,
    id: OccurrenceId,
    target: RigidTransform,
) {
    for original in &before.occurrences {
        let current = after
            .occurrences
            .iter()
            .find(|row| row.id == original.id)
            .unwrap();
        assert_eq!(current.local, original.local);
    }
    assert_eq!(
        serde_json::to_value(&after.joints).unwrap(),
        serde_json::to_value(&before.joints).unwrap()
    );
    assert_eq!(after.occurrences.len(), before.occurrences.len() + 1);
    assert_eq!(
        after
            .occurrences
            .iter()
            .find(|row| row.id == id.to_string())
            .unwrap()
            .local,
        target
    );
}

async fn refuse_constrained_move(
    backend: &mut Backend,
    session: &str,
    occurrence: OccurrenceId,
    pose: RigidTransform,
    before: &AssemblyProjectionDto,
) {
    let error = edit(backend, session, AssemblyEdit::SetPose { occurrence, pose })
        .await
        .unwrap_err();
    assert!(error.contains("constrained occurrence pose requires a fenced joint solve"));
    let after = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

async fn accept(backend: &mut Backend, session: &str, command: AssemblyEdit) {
    let reply = edit(backend, session, command).await.unwrap();
    assert!(reply.changed);
    assert!(reply.capture_error.is_none(), "{:?}", reply.capture_error);
}

async fn edit(
    backend: &mut Backend,
    session: &str,
    command: AssemblyEdit,
) -> Result<Reply, String> {
    let generation = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap()
        .persisted_generation;
    crate::assembly_controls::run(
        backend,
        AssemblyRequest {
            sequence: 1,
            session: session.into(),
            generation,
            action: Action::Edit(command),
            cancellation: None,
        },
    )
    .await
}
